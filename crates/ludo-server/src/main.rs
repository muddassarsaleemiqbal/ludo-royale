use std::{
    collections::HashMap,
    env,
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use ludo_domain::{
    BotDifficulty, Controller, DiceValue, GameCommand, GameState, GameStatus, Player, PlayerColor,
    PlayerId, RulePreset, TokenId, TurnPhase,
};
use ludo_presentation::GameViewModel;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use tokio::sync::{Mutex, mpsc};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use uuid::Uuid;

const SESSION_SECONDS: i64 = 60 * 60 * 24 * 30;

#[derive(Clone)]
struct AppState {
    db: PgPool,
    sockets: Arc<Mutex<HashMap<Uuid, mpsc::UnboundedSender<ServerMessage>>>>,
    ably: Option<AblyConfig>,
}

#[derive(Clone)]
struct AblyConfig {
    key_name: String,
    key_secret: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize)]
struct User {
    id: Uuid,
    email: String,
    display_name: String,
}

#[derive(Deserialize)]
struct Credentials {
    email: String,
    password: String,
    display_name: Option<String>,
}

#[derive(Serialize)]
struct AuthResponse {
    token: String,
    user: User,
}

#[derive(Deserialize)]
struct WsQuery {
    token: String,
}

#[derive(Serialize)]
struct AblyClaims {
    iat: i64,
    exp: i64,
    #[serde(rename = "x-ably-capability")]
    capability: String,
    #[serde(rename = "x-ably-clientId")]
    client_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    ListLobbies,
    CreateLobby {
        name: String,
        rule_preset: String,
        bot_difficulty: String,
        is_public: bool,
    },
    RequestJoin {
        lobby_id: Uuid,
    },
    RespondJoin {
        request_id: Uuid,
        accept: bool,
    },
    LeaveLobby {
        lobby_id: Uuid,
    },
    StartGame {
        lobby_id: Uuid,
    },
    Sync {
        lobby_id: Uuid,
    },
    Roll {
        lobby_id: Uuid,
        revision: u64,
    },
    Move {
        lobby_id: Uuid,
        revision: u64,
        token: TokenId,
    },
}

#[derive(Debug, Clone, Serialize)]
struct LobbySummary {
    id: Uuid,
    name: String,
    host_name: String,
    human_players: i64,
    rule_preset: String,
    bot_difficulty: String,
    status: String,
    is_host: bool,
    requested: bool,
}

#[derive(Debug, Clone, Serialize)]
struct SeatView {
    seat: i16,
    user_id: Option<Uuid>,
    name: String,
    is_bot: bool,
}

#[derive(Debug, Clone, Serialize)]
struct JoinRequestView {
    id: Uuid,
    user_id: Uuid,
    display_name: String,
}

#[derive(Debug, Clone, Serialize)]
struct LobbyView {
    id: Uuid,
    name: String,
    host_user_id: Uuid,
    rule_preset: String,
    bot_difficulty: String,
    status: String,
    seats: Vec<SeatView>,
    requests: Vec<JoinRequestView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Ready {
        user: User,
    },
    LobbyList {
        lobbies: Vec<LobbySummary>,
    },
    Lobby {
        lobby: LobbyView,
    },
    JoinRequested {
        lobby_id: Uuid,
    },
    GameStarted {
        lobby_id: Uuid,
        player: PlayerId,
        model: GameViewModel,
    },
    State {
        lobby_id: Uuid,
        model: GameViewModel,
    },
    Error {
        message: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let db = PgPoolOptions::new()
        .max_connections(20)
        .connect(&env::var("DATABASE_URL")?)
        .await?;
    sqlx::migrate!().run(&db).await?;
    let state = AppState {
        db,
        sockets: Arc::new(Mutex::new(HashMap::new())),
        ably: env::var("ABLY_API_KEY").ok().and_then(|key| {
            key.split_once(':')
                .map(|(key_name, key_secret)| AblyConfig {
                    key_name: key_name.to_owned(),
                    key_secret: key_secret.to_owned(),
                    http: reqwest::Client::new(),
                })
        }),
    };
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/ably/token", get(ably_token))
        .route("/api/online", get(websocket))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    let address: SocketAddr = env::var("LUDO_SERVER_ADDR")
        .unwrap_or_else(|_| {
            format!(
                "0.0.0.0:{}",
                env::var("PORT").unwrap_or_else(|_| "8080".to_owned())
            )
        })
        .parse()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "online server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn websocket(
    State(state): State<AppState>,
    Query(query): Query<WsQuery>,
    upgrade: WebSocketUpgrade,
) -> Result<impl IntoResponse, ApiError> {
    let user = authenticate_token(&state, &query.token).await?;
    Ok(upgrade.on_upgrade(move |socket| online_socket(state, user, socket)))
}

async fn ably_token(State(state): State<AppState>, headers: HeaderMap) -> Result<String, ApiError> {
    let user = authenticate_header(&state, &headers).await?;
    let ably = state
        .ably
        .as_ref()
        .ok_or_else(|| ApiError::internal("Ably is not configured"))?;
    let capability = serde_json::json!({
        format!("ludo:user:{}", user.id): ["subscribe"],
        "ludo:lobbies": ["subscribe"]
    })
    .to_string();
    let issued = now();
    let claims = AblyClaims {
        iat: issued,
        exp: issued + 60 * 60,
        capability,
        client_id: user.id.to_string(),
    };
    let mut header = Header::new(Algorithm::HS256);
    header.kid = Some(ably.key_name.clone());
    encode(
        &header,
        &claims,
        &EncodingKey::from_secret(ably.key_secret.as_bytes()),
    )
    .map_err(|_| ApiError::internal("Could not create Ably token"))
}

async fn online_socket(state: AppState, user: User, socket: WebSocket) {
    let (mut sink, mut source) = socket.split();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    state.sockets.lock().await.insert(user.id, sender.clone());
    let _ = sender.send(ServerMessage::Ready { user: user.clone() });
    let writer = tokio::spawn(async move {
        while let Some(message) = receiver.recv().await {
            let Ok(text) = serde_json::to_string(&message) else {
                continue;
            };
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    send_lobbies(&state, &user).await;
    while let Some(Ok(Message::Text(text))) = source.next().await {
        match serde_json::from_str(&text) {
            Ok(message) => {
                if let Err(error) = handle_online(&state, &user, message).await {
                    send_to(&state, user.id, ServerMessage::Error { message: error.1 }).await;
                }
            }
            Err(_) => {
                send_to(
                    &state,
                    user.id,
                    ServerMessage::Error {
                        message: "Invalid online message".to_owned(),
                    },
                )
                .await
            }
        }
    }
    state.sockets.lock().await.remove(&user.id);
    writer.abort();
}

async fn handle_online(
    state: &AppState,
    user: &User,
    message: ClientMessage,
) -> Result<(), ApiError> {
    match message {
        ClientMessage::ListLobbies => send_lobbies(state, user).await,
        ClientMessage::CreateLobby {
            name,
            rule_preset,
            bot_difficulty,
            is_public,
        } => {
            validate_lobby_options(&rule_preset, &bot_difficulty)?;
            let id = Uuid::new_v4();
            let name = if name.trim().is_empty() {
                format!("{}'s table", user.display_name)
            } else {
                name.trim().chars().take(40).collect()
            };
            let mut tx = state.db.begin().await?;
            sqlx::query("INSERT INTO game_lobbies(id,host_user_id,name,rule_preset,bot_difficulty,is_public) VALUES($1,$2,$3,$4,$5,$6)")
                .bind(id).bind(user.id).bind(name).bind(rule_preset).bind(bot_difficulty).bind(is_public).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO lobby_members(lobby_id,user_id,seat) VALUES($1,$2,0)")
                .bind(id)
                .bind(user.id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            send_lobby(state, id).await?;
            broadcast_lobbies(state).await;
        }
        ClientMessage::RequestJoin { lobby_id } => {
            sqlx::query("INSERT INTO lobby_join_requests(id,lobby_id,user_id) SELECT $1,$2,$3 WHERE EXISTS(SELECT 1 FROM game_lobbies WHERE id=$2 AND status='waiting') ON CONFLICT(lobby_id,user_id) DO UPDATE SET status='pending',created_at=CURRENT_TIMESTAMP")
                .bind(Uuid::new_v4()).bind(lobby_id).bind(user.id).execute(&state.db).await?;
            let host: Option<Uuid> =
                sqlx::query_scalar("SELECT host_user_id FROM game_lobbies WHERE id=$1")
                    .bind(lobby_id)
                    .fetch_optional(&state.db)
                    .await?;
            if let Some(host) = host {
                send_lobby_to(state, lobby_id, host).await?;
            }
            send_to(state, user.id, ServerMessage::JoinRequested { lobby_id }).await;
            broadcast_lobbies(state).await;
        }
        ClientMessage::RespondJoin { request_id, accept } => {
            let mut tx = state.db.begin().await?;
            let row = sqlx::query("SELECT r.lobby_id,r.user_id FROM lobby_join_requests r JOIN game_lobbies l ON l.id=r.lobby_id WHERE r.id=$1 AND l.host_user_id=$2 AND r.status='pending' FOR UPDATE")
                .bind(request_id).bind(user.id).fetch_optional(&mut *tx).await?.ok_or_else(|| ApiError::bad_request("Join request is no longer available"))?;
            let lobby_id: Uuid = row.get(0);
            let joining: Uuid = row.get(1);
            if accept {
                let seat: Option<i16> = sqlx::query_scalar("SELECT s FROM generate_series(1,3) s WHERE NOT EXISTS(SELECT 1 FROM lobby_members WHERE lobby_id=$1 AND seat=s) ORDER BY s LIMIT 1")
                    .bind(lobby_id).fetch_optional(&mut *tx).await?;
                let seat = seat.ok_or_else(|| ApiError::conflict("This table is full"))?;
                sqlx::query("INSERT INTO lobby_members(lobby_id,user_id,seat) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(lobby_id).bind(joining).bind(seat).execute(&mut *tx).await?;
            }
            sqlx::query(
                "UPDATE lobby_join_requests SET status=$2::join_request_status WHERE id=$1",
            )
            .bind(request_id)
            .bind(if accept { "accepted" } else { "declined" })
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            send_lobby(state, lobby_id).await?;
            broadcast_lobbies(state).await;
        }
        ClientMessage::LeaveLobby { lobby_id } => {
            sqlx::query("DELETE FROM lobby_members WHERE lobby_id=$1 AND user_id=$2 AND EXISTS(SELECT 1 FROM game_lobbies WHERE id=$1 AND host_user_id<>$2 AND status='waiting')").bind(lobby_id).bind(user.id).execute(&state.db).await?;
            send_lobby(state, lobby_id).await?;
            broadcast_lobbies(state).await;
        }
        ClientMessage::StartGame { lobby_id } => start_game(state, user, lobby_id).await?,
        ClientMessage::Sync { lobby_id } => sync_game(state, user, lobby_id).await?,
        ClientMessage::Roll { lobby_id, revision } => {
            apply_action(state, user, lobby_id, revision, None).await?
        }
        ClientMessage::Move {
            lobby_id,
            revision,
            token,
        } => apply_action(state, user, lobby_id, revision, Some(token)).await?,
    }
    Ok(())
}

async fn start_game(state: &AppState, user: &User, lobby_id: Uuid) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    let lobby = sqlx::query("SELECT rule_preset,bot_difficulty FROM game_lobbies WHERE id=$1 AND host_user_id=$2 AND status='waiting' FOR UPDATE")
        .bind(lobby_id).bind(user.id).fetch_optional(&mut *tx).await?.ok_or_else(|| ApiError::bad_request("Only the host can start this game"))?;
    let preset: String = lobby.get(0);
    let difficulty: String = lobby.get(1);
    let members = sqlx::query("SELECT m.seat,u.id,u.display_name FROM lobby_members m JOIN users u ON u.id=m.user_id WHERE m.lobby_id=$1 ORDER BY m.seat")
        .bind(lobby_id).fetch_all(&mut *tx).await?;
    let mut names: [Option<(Uuid, String)>; 4] = [None, None, None, None];
    for row in members {
        names[usize::try_from(row.get::<i16, _>(0)).unwrap_or(0)] = Some((row.get(1), row.get(2)));
    }
    let bot = parse_difficulty(&difficulty);
    let players = PlayerColor::ALL
        .into_iter()
        .enumerate()
        .map(|(index, color)| {
            let human = names[index].as_ref();
            Player {
                id: player_id(u8::try_from(index).unwrap_or(0)),
                name: human.map_or_else(
                    || format!("Royal Bot {}", index + 1),
                    |(_, name)| name.clone(),
                ),
                color,
                controller: if human.is_some() {
                    Controller::Human
                } else {
                    Controller::Bot
                },
                bot_difficulty: bot,
            }
        })
        .collect();
    let game = GameState::new(players, parse_preset(&preset).rules())
        .map_err(|_| ApiError::internal("Could not create game"))?;
    let json =
        serde_json::to_value(&game).map_err(|_| ApiError::internal("Could not serialize game"))?;
    sqlx::query("UPDATE game_lobbies SET status='playing',game_state=$2,updated_at=CURRENT_TIMESTAMP WHERE id=$1").bind(lobby_id).bind(json).execute(&mut *tx).await?;
    tx.commit().await?;
    broadcast_game_started(state, lobby_id, &game).await?;
    broadcast_lobbies(state).await;
    Ok(())
}

async fn sync_game(state: &AppState, user: &User, lobby_id: Uuid) -> Result<(), ApiError> {
    let row = sqlx::query("SELECT l.game_state,m.seat FROM game_lobbies l JOIN lobby_members m ON m.lobby_id=l.id WHERE l.id=$1 AND m.user_id=$2 AND l.status='playing'")
        .bind(lobby_id).bind(user.id).fetch_optional(&state.db).await?.ok_or_else(|| ApiError::bad_request("Game not found"))?;
    let game: GameState = serde_json::from_value(row.get(0))
        .map_err(|_| ApiError::internal("Stored game is invalid"))?;
    send_to(
        state,
        user.id,
        ServerMessage::GameStarted {
            lobby_id,
            player: player_id(u8::try_from(row.get::<i16, _>(1)).unwrap_or(0)),
            model: GameViewModel::from(&game),
        },
    )
    .await;
    Ok(())
}

async fn apply_action(
    state: &AppState,
    user: &User,
    lobby_id: Uuid,
    revision: u64,
    token: Option<TokenId>,
) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    let row = sqlx::query("SELECT l.game_state,m.seat FROM game_lobbies l JOIN lobby_members m ON m.lobby_id=l.id WHERE l.id=$1 AND m.user_id=$2 AND l.status='playing' FOR UPDATE OF l")
        .bind(lobby_id).bind(user.id).fetch_optional(&mut *tx).await?.ok_or_else(|| ApiError::bad_request("Game not found"))?;
    let mut game: GameState = serde_json::from_value(row.get(0))
        .map_err(|_| ApiError::internal("Stored game is invalid"))?;
    let seat = usize::try_from(row.get::<i16, _>(1)).unwrap_or(4);
    if game.revision() != revision || game.current_player_index() != seat {
        return Err(ApiError::conflict(
            "The game advanced; refreshing the board",
        ));
    }
    let command = token.map_or_else(random_roll, GameCommand::Move);
    game.apply(command)
        .map_err(|error| ApiError::bad_request(&error.to_string()))?;
    run_bots(&mut game);
    let status = if game.status() == GameStatus::Finished {
        "finished"
    } else {
        "playing"
    };
    sqlx::query("UPDATE game_lobbies SET game_state=$2,status=$3::lobby_status,updated_at=CURRENT_TIMESTAMP WHERE id=$1")
        .bind(lobby_id).bind(serde_json::to_value(&game).map_err(|_| ApiError::internal("Could not save game"))?).bind(status).execute(&mut *tx).await?;
    tx.commit().await?;
    broadcast_state(state, lobby_id, &game).await?;
    Ok(())
}

fn run_bots(game: &mut GameState) {
    let mut steps = 0;
    while game.status() == GameStatus::Playing
        && game.current().player.controller == Controller::Bot
        && steps < 128
    {
        let command = match game.phase() {
            TurnPhase::AwaitingRoll => random_roll(),
            TurnPhase::AwaitingMove { legal_tokens, .. } => {
                let index = rand::rng().random_range(0..legal_tokens.len());
                GameCommand::Move(legal_tokens[index])
            }
        };
        if game.apply(command).is_err() {
            break;
        }
        steps += 1;
    }
}

fn random_roll() -> GameCommand {
    GameCommand::Roll(
        DiceValue::new(rand::rng().random_range(1..=6)).unwrap_or_else(|| std::process::abort()),
    )
}

async fn broadcast_game_started(
    state: &AppState,
    lobby_id: Uuid,
    game: &GameState,
) -> Result<(), ApiError> {
    let members = sqlx::query("SELECT user_id,seat FROM lobby_members WHERE lobby_id=$1")
        .bind(lobby_id)
        .fetch_all(&state.db)
        .await?;
    for row in members {
        send_to(
            state,
            row.get(0),
            ServerMessage::GameStarted {
                lobby_id,
                player: player_id(u8::try_from(row.get::<i16, _>(1)).unwrap_or(0)),
                model: GameViewModel::from(game),
            },
        )
        .await;
    }
    Ok(())
}

async fn broadcast_state(
    state: &AppState,
    lobby_id: Uuid,
    game: &GameState,
) -> Result<(), ApiError> {
    let users: Vec<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM lobby_members WHERE lobby_id=$1")
            .bind(lobby_id)
            .fetch_all(&state.db)
            .await?;
    for id in users {
        send_to(
            state,
            id,
            ServerMessage::State {
                lobby_id,
                model: GameViewModel::from(game),
            },
        )
        .await;
    }
    Ok(())
}

async fn send_lobbies(state: &AppState, user: &User) {
    let result = async {
        let rows = sqlx::query("SELECT l.id,l.name,u.display_name,(SELECT count(*) FROM lobby_members m WHERE m.lobby_id=l.id),l.rule_preset,l.bot_difficulty,l.status::text,l.host_user_id=$1,EXISTS(SELECT 1 FROM lobby_join_requests r WHERE r.lobby_id=l.id AND r.user_id=$1 AND r.status='pending') FROM game_lobbies l JOIN users u ON u.id=l.host_user_id WHERE (l.is_public OR l.host_user_id=$1 OR EXISTS(SELECT 1 FROM lobby_members m WHERE m.lobby_id=l.id AND m.user_id=$1)) AND l.status='waiting' ORDER BY l.updated_at DESC LIMIT 100")
            .bind(user.id).fetch_all(&state.db).await?;
        Ok::<_, sqlx::Error>(rows.into_iter().map(|r| LobbySummary { id:r.get(0),name:r.get(1),host_name:r.get(2),human_players:r.get(3),rule_preset:r.get(4),bot_difficulty:r.get(5),status:r.get(6),is_host:r.get(7),requested:r.get(8) }).collect())
    }.await;
    match result {
        Ok(lobbies) => send_to(state, user.id, ServerMessage::LobbyList { lobbies }).await,
        Err(_) => {
            send_to(
                state,
                user.id,
                ServerMessage::Error {
                    message: "Could not load games".to_owned(),
                },
            )
            .await
        }
    }
}

async fn broadcast_lobbies(state: &AppState) {
    if state.ably.is_some() {
        publish_ably(
            state,
            "ludo:lobbies",
            "changed",
            &serde_json::json!({ "at": now() }),
        )
        .await;
        return;
    }
    let users: Vec<Uuid> = state.sockets.lock().await.keys().copied().collect();
    for id in users {
        if let Ok(Some(row)) = sqlx::query_as::<_, (Uuid, String, String)>(
            "SELECT id,email,display_name FROM users WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(&state.db)
        .await
        {
            send_lobbies(
                state,
                &User {
                    id: row.0,
                    email: row.1,
                    display_name: row.2,
                },
            )
            .await;
        }
    }
}

async fn send_lobby(state: &AppState, lobby_id: Uuid) -> Result<(), ApiError> {
    let users: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM lobby_members WHERE lobby_id=$1 UNION SELECT host_user_id FROM game_lobbies WHERE id=$1").bind(lobby_id).fetch_all(&state.db).await?;
    for user in users {
        send_lobby_to(state, lobby_id, user).await?;
    }
    Ok(())
}

async fn send_lobby_to(state: &AppState, lobby_id: Uuid, user_id: Uuid) -> Result<(), ApiError> {
    let row = sqlx::query("SELECT id,name,host_user_id,rule_preset,bot_difficulty,status::text FROM game_lobbies WHERE id=$1").bind(lobby_id).fetch_optional(&state.db).await?.ok_or_else(|| ApiError::bad_request("Lobby not found"))?;
    let members = sqlx::query("SELECT m.seat,u.id,u.display_name FROM lobby_members m JOIN users u ON u.id=m.user_id WHERE m.lobby_id=$1 ORDER BY m.seat").bind(lobby_id).fetch_all(&state.db).await?;
    let mut seats = (0_i16..4)
        .map(|seat| SeatView {
            seat,
            user_id: None,
            name: format!("Royal Bot {}", seat + 1),
            is_bot: true,
        })
        .collect::<Vec<_>>();
    for member in members {
        let seat: i16 = member.get(0);
        seats[usize::try_from(seat).unwrap_or(0)] = SeatView {
            seat,
            user_id: Some(member.get(1)),
            name: member.get(2),
            is_bot: false,
        };
    }
    let requests = if row.get::<Uuid, _>(2) == user_id {
        sqlx::query("SELECT r.id,u.id,u.display_name FROM lobby_join_requests r JOIN users u ON u.id=r.user_id WHERE r.lobby_id=$1 AND r.status='pending' ORDER BY r.created_at").bind(lobby_id).fetch_all(&state.db).await?.into_iter().map(|r|JoinRequestView{id:r.get(0),user_id:r.get(1),display_name:r.get(2)}).collect()
    } else {
        Vec::new()
    };
    send_to(
        state,
        user_id,
        ServerMessage::Lobby {
            lobby: LobbyView {
                id: row.get(0),
                name: row.get(1),
                host_user_id: row.get(2),
                rule_preset: row.get(3),
                bot_difficulty: row.get(4),
                status: row.get(5),
                seats,
                requests,
            },
        },
    )
    .await;
    Ok(())
}

async fn send_to(state: &AppState, user: Uuid, message: ServerMessage) {
    if publish_ably(state, &format!("ludo:user:{user}"), "event", &message).await {
        return;
    }
    if let Some(socket) = state.sockets.lock().await.get(&user) {
        let _ = socket.send(message);
    }
}

async fn publish_ably<T: Serialize + ?Sized>(
    state: &AppState,
    channel: &str,
    name: &str,
    data: &T,
) -> bool {
    let Some(ably) = &state.ably else {
        return false;
    };
    let response = ably
        .http
        .post(format!("https://rest.ably.io/channels/{channel}/messages"))
        .basic_auth(&ably.key_name, Some(&ably.key_secret))
        .json(&serde_json::json!({ "name": name, "data": data }))
        .send()
        .await;
    match response {
        Ok(response) if response.status().is_success() => true,
        Ok(response) => {
            tracing::warn!(status = %response.status(), %channel, "Ably rejected publish");
            false
        }
        Err(error) => {
            tracing::warn!(%error, %channel, "Ably publish failed");
            false
        }
    }
}

fn validate_lobby_options(preset: &str, difficulty: &str) -> Result<(), ApiError> {
    if !["classic", "quick", "tournament"].contains(&preset)
        || !["easy", "medium", "hard"].contains(&difficulty)
    {
        return Err(ApiError::bad_request("Invalid game options"));
    }
    Ok(())
}
fn parse_preset(value: &str) -> RulePreset {
    match value {
        "quick" => RulePreset::Quick,
        "tournament" => RulePreset::Tournament,
        _ => RulePreset::Classic,
    }
}
fn parse_difficulty(value: &str) -> BotDifficulty {
    match value {
        "easy" => BotDifficulty::Easy,
        "hard" => BotDifficulty::Hard,
        _ => BotDifficulty::Medium,
    }
}
fn player_id(index: u8) -> PlayerId {
    PlayerId::new(index).unwrap_or_else(|| std::process::abort())
}

async fn register(
    State(state): State<AppState>,
    Json(input): Json<Credentials>,
) -> Result<Json<AuthResponse>, ApiError> {
    let email = normalize_email(&input.email)?;
    validate_password(&input.password)?;
    let display_name = normalize_name(input.display_name.as_deref().unwrap_or(""))?;
    let password_hash = hash_password(&input.password)?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,$3,$4)")
        .bind(id)
        .bind(&email)
        .bind(&display_name)
        .bind(password_hash)
        .execute(&state.db)
        .await
        .map_err(|_| ApiError::conflict("An account with that email already exists"))?;
    issue_session(
        &state,
        User {
            id,
            email,
            display_name,
        },
    )
    .await
}
async fn login(
    State(state): State<AppState>,
    Json(input): Json<Credentials>,
) -> Result<Json<AuthResponse>, ApiError> {
    let email = normalize_email(&input.email)?;
    let row: (Uuid, String, String, String) =
        sqlx::query_as("SELECT id,email,display_name,password_hash FROM users WHERE email=$1")
            .bind(email)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::unauthorized("Invalid email or password"))?;
    let parsed = PasswordHash::new(&row.3)
        .map_err(|_| ApiError::internal("Stored password hash is invalid"))?;
    Argon2::default()
        .verify_password(input.password.as_bytes(), &parsed)
        .map_err(|_| ApiError::unauthorized("Invalid email or password"))?;
    issue_session(
        &state,
        User {
            id: row.0,
            email: row.1,
            display_name: row.2,
        },
    )
    .await
}
async fn me(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<User>, ApiError> {
    Ok(Json(authenticate_header(&state, &headers).await?))
}
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<StatusCode, ApiError> {
    sqlx::query("DELETE FROM sessions WHERE token_hash=$1")
        .bind(token_hash(bearer(&headers)?))
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn issue_session(state: &AppState, user: User) -> Result<Json<AuthResponse>, ApiError> {
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at) VALUES($1,$2,$3)")
        .bind(token_hash(&token))
        .bind(user.id)
        .bind(now() + SESSION_SECONDS)
        .execute(&state.db)
        .await?;
    Ok(Json(AuthResponse { token, user }))
}
async fn authenticate_header(state: &AppState, headers: &HeaderMap) -> Result<User, ApiError> {
    authenticate_token(state, bearer(headers)?).await
}
async fn authenticate_token(state: &AppState, token: &str) -> Result<User, ApiError> {
    let row:(Uuid,String,String)=sqlx::query_as("SELECT u.id,u.email,u.display_name FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=$1 AND s.expires_at>$2").bind(token_hash(token)).bind(now()).fetch_optional(&state.db).await?.ok_or_else(||ApiError::unauthorized("Login required"))?;
    Ok(User {
        id: row.0,
        email: row.1,
        display_name: row.2,
    })
}
fn bearer(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::unauthorized("Login required"))
}
fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
fn normalize_email(value: &str) -> Result<String, ApiError> {
    let value = value.trim().to_ascii_lowercase();
    if value.len() > 254 || !value.contains('@') {
        return Err(ApiError::bad_request("Enter a valid email"));
    }
    Ok(value)
}
fn normalize_name(value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if !(2..=24).contains(&value.chars().count()) {
        return Err(ApiError::bad_request(
            "Display name must be 2–24 characters",
        ));
    }
    Ok(value.to_owned())
}
fn validate_password(value: &str) -> Result<(), ApiError> {
    if value.len() < 10 || value.len() > 128 {
        return Err(ApiError::bad_request("Password must be 10–128 characters"));
    }
    Ok(())
}
fn hash_password(value: &str) -> Result<String, ApiError> {
    Argon2::default()
        .hash_password(value.as_bytes(), &SaltString::generate(&mut OsRng))
        .map(|h| h.to_string())
        .map_err(|_| ApiError::internal("Password hashing failed"))
}

struct ApiError(StatusCode, String);
impl ApiError {
    fn bad_request(m: &str) -> Self {
        Self(StatusCode::BAD_REQUEST, m.to_owned())
    }
    fn unauthorized(m: &str) -> Self {
        Self(StatusCode::UNAUTHORIZED, m.to_owned())
    }
    fn conflict(m: &str) -> Self {
        Self(StatusCode::CONFLICT, m.to_owned())
    }
    fn internal(m: &str) -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR, m.to_owned())
    }
}
impl From<sqlx::Error> for ApiError {
    fn from(_: sqlx::Error) -> Self {
        Self::internal("Database operation failed")
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.0, Json(serde_json::json!({"error":self.1}))).into_response()
    }
}
