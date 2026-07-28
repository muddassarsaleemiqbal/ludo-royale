use std::{
    collections::{HashMap, VecDeque},
    env,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use ludo_ai::{BotRequest, ParallelBot};
use ludo_domain::{
    BotDifficulty, Controller, DiceValue, GameCommand, GameState, GameStatus, Player, PlayerColor,
    PlayerId, RulePreset, TokenId, TurnPhase,
};
use ludo_presentation::GameViewModel;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgPoolOptions};
use tokio::sync::{Mutex, mpsc};
use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
    trace::TraceLayer,
};
use uuid::Uuid;

const SESSION_SECONDS: i64 = 60 * 60 * 24 * 30;
const TURN_SECONDS: u16 = 30;

#[derive(Clone)]
struct AppState {
    db: PgPool,
    sockets: Arc<Mutex<HashMap<Uuid, mpsc::UnboundedSender<ServerMessage>>>>,
    ably: Option<AblyConfig>,
    rate_limits: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
    alert_webhook: Option<String>,
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

#[derive(Debug, Deserialize)]
struct ClientEnvelope {
    command_id: Uuid,
    #[serde(flatten)]
    message: ClientMessage,
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
    JoinDecision {
        lobby_id: Uuid,
        accepted: bool,
    },
    GameStarted {
        lobby_id: Uuid,
        player: PlayerId,
        model: GameViewModel,
        turn_seconds: u16,
    },
    State {
        lobby_id: Uuid,
        model: GameViewModel,
        turn_seconds: u16,
    },
    Ack {
        command_id: Uuid,
    },
    Error {
        command_id: Option<Uuid>,
        code: &'static str,
        message: String,
        recoverable: bool,
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
        rate_limits: Arc::new(Mutex::new(HashMap::new())),
        alert_webhook: env::var("LUDO_ALERT_WEBHOOK").ok(),
    };
    if state.ably.is_some() {
        tokio::spawn(run_outbox(state.clone()));
    }
    tokio::spawn(run_match_supervisor(state.clone()));
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/health/ready", get(health_ready))
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/ably/token", get(ably_token))
        .route("/api/online", get(websocket))
        .layer(cors_layer()?)
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

async fn health_ready(State(state): State<AppState>) -> Result<&'static str, ApiError> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.db)
        .await?;
    Ok("ready")
}

async fn websocket(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<impl IntoResponse, ApiError> {
    let token = websocket_token(&headers)?;
    let user = authenticate_token(&state, token).await?;
    Ok(upgrade
        .protocols(["ludo"])
        .on_upgrade(move |socket| online_socket(state, user, socket)))
}

async fn ably_token(State(state): State<AppState>, headers: HeaderMap) -> Result<String, ApiError> {
    let user = authenticate_header(&state, &headers).await?;
    let ably = state
        .ably
        .as_ref()
        .ok_or_else(|| ApiError::internal("Ably is not configured"))?;
    create_ably_jwt(ably, user.id)
}

fn create_ably_jwt(ably: &AblyConfig, user_id: Uuid) -> Result<String, ApiError> {
    let capability = serde_json::json!({
        format!("ludo:user:{user_id}"): ["subscribe"],
        "ludo:lobbies": ["subscribe"]
    })
    .to_string();
    let issued = now();
    let claims = AblyClaims {
        iat: issued,
        exp: issued + 60 * 60,
        capability,
        client_id: user_id.to_string(),
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
    resume_user_state(&state, &user).await;
    while let Some(Ok(Message::Text(text))) = source.next().await {
        if text.len() > 16 * 1024 {
            send_to(
                &state,
                user.id,
                ServerMessage::Error {
                    command_id: None,
                    code: "message_too_large",
                    message: "Online messages are limited to 16 KiB".to_owned(),
                    recoverable: true,
                },
            )
            .await;
            continue;
        }
        match serde_json::from_str::<ClientEnvelope>(&text) {
            Ok(envelope) => handle_envelope(&state, &user, envelope).await,
            Err(_) => {
                send_to(
                    &state,
                    user.id,
                    ServerMessage::Error {
                        command_id: None,
                        code: "invalid_message",
                        message: "Invalid online message".to_owned(),
                        recoverable: true,
                    },
                )
                .await;
            }
        }
    }
    state.sockets.lock().await.remove(&user.id);
    writer.abort();
}

async fn handle_envelope(state: &AppState, user: &User, envelope: ClientEnvelope) {
    let result = enforce_rate_limit(state, &format!("command:{}", user.id), 90).await;
    if let Err(error) = result {
        send_command_error(state, user.id, envelope.command_id, error).await;
        return;
    }
    if command_was_processed(state, user.id, envelope.command_id).await {
        send_to(
            state,
            user.id,
            ServerMessage::Ack {
                command_id: envelope.command_id,
            },
        )
        .await;
        return;
    }
    if let Err(error) = handle_online(state, user, envelope.message).await {
        tracing::warn!(
            user_id = %user.id,
            status = %error.0,
            message = %error.1,
            "online command failed"
        );
        send_command_error(state, user.id, envelope.command_id, error).await;
    } else {
        record_processed_command(state, user.id, envelope.command_id).await;
        send_to(
            state,
            user.id,
            ServerMessage::Ack {
                command_id: envelope.command_id,
            },
        )
        .await;
    }
}

async fn send_command_error(state: &AppState, user_id: Uuid, command_id: Uuid, error: ApiError) {
    send_to(
        state,
        user_id,
        ServerMessage::Error {
            command_id: Some(command_id),
            code: error.code(),
            recoverable: error.0 != StatusCode::UNAUTHORIZED,
            message: error.1,
        },
    )
    .await;
}

async fn command_was_processed(state: &AppState, user_id: Uuid, command_id: Uuid) -> bool {
    match sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM processed_commands WHERE command_id=$1 AND user_id=$2)",
    )
    .bind(command_id)
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    {
        Ok(processed) => processed,
        Err(error) => {
            tracing::error!(%error, %user_id, %command_id, "could not check command replay");
            false
        }
    }
}

async fn record_processed_command(state: &AppState, user_id: Uuid, command_id: Uuid) {
    if let Err(error) = sqlx::query(
        "INSERT INTO processed_commands(command_id,user_id) VALUES($1,$2)
         ON CONFLICT(command_id) DO NOTHING",
    )
    .bind(command_id)
    .bind(user_id)
    .execute(&state.db)
    .await
    {
        tracing::error!(%error, %user_id, %command_id, "could not record processed command");
    }
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
        } => create_lobby(state, user, name, rule_preset, bot_difficulty, is_public).await?,
        ClientMessage::RequestJoin { lobby_id } => request_join(state, user, lobby_id).await?,
        ClientMessage::RespondJoin { request_id, accept } => {
            respond_join(state, user, request_id, accept).await?;
        }
        ClientMessage::LeaveLobby { lobby_id } => {
            sqlx::query("DELETE FROM lobby_members WHERE lobby_id=$1 AND user_id=$2 AND EXISTS(SELECT 1 FROM game_lobbies WHERE id=$1 AND host_user_id<>$2 AND status='waiting')").bind(lobby_id).bind(user.id).execute(&state.db).await?;
            send_lobby(state, lobby_id).await?;
            broadcast_lobbies(state).await;
        }
        ClientMessage::StartGame { lobby_id } => start_game(state, user, lobby_id).await?,
        ClientMessage::Sync { lobby_id } => sync_game(state, user, lobby_id).await?,
        ClientMessage::Roll { lobby_id, revision } => {
            apply_action(state, user, lobby_id, revision, None).await?;
        }
        ClientMessage::Move {
            lobby_id,
            revision,
            token,
        } => apply_action(state, user, lobby_id, revision, Some(token)).await?,
    }
    Ok(())
}

async fn create_lobby(
    state: &AppState,
    user: &User,
    name: String,
    rule_preset: String,
    bot_difficulty: String,
    is_public: bool,
) -> Result<(), ApiError> {
    validate_lobby_options(&rule_preset, &bot_difficulty)?;
    let id = Uuid::new_v4();
    let name = if name.trim().is_empty() {
        format!("{}'s table", user.display_name)
    } else {
        name.trim().chars().take(40).collect()
    };
    let mut tx = state.db.begin().await?;
    let active_lobby: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM lobby_members m
            JOIN game_lobbies l ON l.id=m.lobby_id
            WHERE m.user_id=$1 AND l.status IN ('waiting','playing')
        )",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    if active_lobby {
        return Err(ApiError::conflict(
            "Leave or finish your current table before creating another",
        ));
    }
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
    Ok(())
}

async fn request_join(state: &AppState, user: &User, lobby_id: Uuid) -> Result<(), ApiError> {
    let inserted = sqlx::query(
        "INSERT INTO lobby_join_requests(id,lobby_id,user_id)
         SELECT $1,$2,$3
         WHERE EXISTS(
            SELECT 1 FROM game_lobbies l
            WHERE l.id=$2 AND l.status='waiting' AND l.host_user_id<>$3
              AND NOT EXISTS(
                SELECT 1 FROM lobby_members m WHERE m.lobby_id=l.id AND m.user_id=$3
              )
              AND (SELECT count(*) FROM lobby_members m WHERE m.lobby_id=l.id) < 4
         )
         ON CONFLICT(lobby_id,user_id)
         DO UPDATE SET status='pending',created_at=CURRENT_TIMESTAMP",
    )
    .bind(Uuid::new_v4())
    .bind(lobby_id)
    .bind(user.id)
    .execute(&state.db)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::conflict(
            "This table is unavailable, full, or you already have a seat",
        ));
    }
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
    Ok(())
}

async fn respond_join(
    state: &AppState,
    user: &User,
    request_id: Uuid,
    accept: bool,
) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    let row = sqlx::query("SELECT r.lobby_id,r.user_id FROM lobby_join_requests r JOIN game_lobbies l ON l.id=r.lobby_id WHERE r.id=$1 AND l.host_user_id=$2 AND l.status='waiting' AND r.status='pending' FOR UPDATE OF l,r")
        .bind(request_id).bind(user.id).fetch_optional(&mut *tx).await?.ok_or_else(|| ApiError::bad_request("Join request is no longer available"))?;
    let lobby_id: Uuid = row.get(0);
    let joining: Uuid = row.get(1);
    if accept {
        accept_joining_player(&mut tx, lobby_id, joining).await?;
    }
    sqlx::query("UPDATE lobby_join_requests SET status=$2::join_request_status WHERE id=$1")
        .bind(request_id)
        .bind(if accept { "accepted" } else { "declined" })
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    send_lobby(state, lobby_id).await?;
    send_to(
        state,
        joining,
        ServerMessage::JoinDecision {
            lobby_id,
            accepted: accept,
        },
    )
    .await;
    broadcast_lobbies(state).await;
    Ok(())
}

async fn accept_joining_player(
    tx: &mut Transaction<'_, Postgres>,
    lobby_id: Uuid,
    joining: Uuid,
) -> Result<(), ApiError> {
    let existing: Option<i16> =
        sqlx::query_scalar("SELECT seat FROM lobby_members WHERE lobby_id=$1 AND user_id=$2")
            .bind(lobby_id)
            .bind(joining)
            .fetch_optional(&mut **tx)
            .await?;
    let active_elsewhere: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM lobby_members m JOIN game_lobbies l ON l.id=m.lobby_id
            WHERE m.user_id=$1 AND m.lobby_id<>$2 AND l.status IN ('waiting','playing')
        )",
    )
    .bind(joining)
    .bind(lobby_id)
    .fetch_one(&mut **tx)
    .await?;
    if active_elsewhere {
        return Err(ApiError::conflict(
            "That player has already joined another active table",
        ));
    }
    if existing.is_some() {
        return Ok(());
    }
    let seat: Option<i16> = sqlx::query_scalar(
        "SELECT s::smallint FROM generate_series(1,3) AS s
         WHERE NOT EXISTS(
            SELECT 1 FROM lobby_members WHERE lobby_id=$1 AND seat=s::smallint
         )
         ORDER BY s LIMIT 1",
    )
    .bind(lobby_id)
    .fetch_optional(&mut **tx)
    .await?;
    let seat = seat.ok_or_else(|| ApiError::conflict("This table is full"))?;
    sqlx::query("INSERT INTO lobby_members(lobby_id,user_id,seat) VALUES($1,$2,$3)")
        .bind(lobby_id)
        .bind(joining)
        .bind(seat)
        .execute(&mut **tx)
        .await?;
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
    sqlx::query("UPDATE game_lobbies SET status='playing',game_state=$2,turn_deadline=CURRENT_TIMESTAMP+($3::text||' seconds')::interval,updated_at=CURRENT_TIMESTAMP WHERE id=$1").bind(lobby_id).bind(json).bind(TURN_SECONDS.to_string()).execute(&mut *tx).await?;
    if state.ably.is_some() {
        for (seat, member) in names.iter().enumerate() {
            let Some((user_id, _)) = member else {
                continue;
            };
            enqueue_outbox(
                &mut tx,
                &format!("ludo:user:{user_id}"),
                "event",
                &ServerMessage::GameStarted {
                    lobby_id,
                    player: player_id(u8::try_from(seat).unwrap_or(0)),
                    model: GameViewModel::from(&game),
                    turn_seconds: TURN_SECONDS,
                },
            )
            .await?;
        }
    }
    tx.commit().await?;
    if state.ably.is_none() {
        broadcast_game_started(state, lobby_id, &game).await?;
    }
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
            turn_seconds: TURN_SECONDS,
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
    sqlx::query("UPDATE game_lobbies SET game_state=$2,status=$3::lobby_status,turn_deadline=CASE WHEN $3='playing' THEN CURRENT_TIMESTAMP+($4::text||' seconds')::interval ELSE NULL END,updated_at=CURRENT_TIMESTAMP WHERE id=$1")
        .bind(lobby_id).bind(serde_json::to_value(&game).map_err(|_| ApiError::internal("Could not save game"))?).bind(status).bind(TURN_SECONDS.to_string()).execute(&mut *tx).await?;
    if game.status() == GameStatus::Finished {
        let member_rows =
            sqlx::query("SELECT seat,user_id FROM lobby_members WHERE lobby_id=$1 ORDER BY seat")
                .bind(lobby_id)
                .fetch_all(&mut *tx)
                .await?;
        let mut player_ids = vec![serde_json::Value::Null; 4];
        let mut winner_user_id = None;
        let winner_seat = game.rankings().first().map(|winner| winner.index());
        for member in member_rows {
            let seat = usize::try_from(member.get::<i16, _>(0)).unwrap_or(4);
            let member_id: Uuid = member.get(1);
            if seat < player_ids.len() {
                player_ids[seat] = serde_json::Value::String(member_id.to_string());
            }
            if Some(seat) == winner_seat {
                winner_user_id = Some(member_id);
            }
        }
        sqlx::query(
            "INSERT INTO match_results(id,lobby_id,winner_user_id,player_ids,final_state)
             VALUES($1,$2,$3,$4,$5) ON CONFLICT(lobby_id) DO NOTHING",
        )
        .bind(Uuid::new_v4())
        .bind(lobby_id)
        .bind(winner_user_id)
        .bind(serde_json::Value::Array(player_ids))
        .bind(
            serde_json::to_value(&game)
                .map_err(|_| ApiError::internal("Could not save match result"))?,
        )
        .execute(&mut *tx)
        .await?;
    }
    if state.ably.is_some() {
        let users: Vec<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM lobby_members WHERE lobby_id=$1")
                .bind(lobby_id)
                .fetch_all(&mut *tx)
                .await?;
        let message = ServerMessage::State {
            lobby_id,
            model: GameViewModel::from(&game),
            turn_seconds: TURN_SECONDS,
        };
        for user_id in users {
            enqueue_outbox(&mut tx, &format!("ludo:user:{user_id}"), "event", &message).await?;
        }
    }
    tx.commit().await?;
    if state.ably.is_none() {
        broadcast_state(state, lobby_id, &game).await?;
    }
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
                let difficulty = game.current().player.bot_difficulty;
                let decision = ParallelBot::choose(
                    &BotRequest::new(game.clone(), difficulty).with_thinking_time_ms(0),
                );
                let Some(token) = decision.token.or_else(|| legal_tokens.first().copied()) else {
                    break;
                };
                GameCommand::Move(token)
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
                turn_seconds: TURN_SECONDS,
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
                turn_seconds: TURN_SECONDS,
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
                    command_id: None,
                    code: "lobby_list_failed",
                    message: "Could not load games".to_owned(),
                    recoverable: true,
                },
            )
            .await;
        }
    }
}

async fn resume_user_state(state: &AppState, user: &User) {
    let row = sqlx::query(
        "SELECT l.id,l.status::text
         FROM lobby_members m
         JOIN game_lobbies l ON l.id=m.lobby_id
         WHERE m.user_id=$1 AND l.status IN ('waiting','playing')
         ORDER BY l.updated_at DESC
         LIMIT 1",
    )
    .bind(user.id)
    .fetch_optional(&state.db)
    .await;
    match row {
        Ok(Some(row)) if row.get::<String, _>(1) == "playing" => {
            if let Err(error) = sync_game(state, user, row.get(0)).await {
                tracing::warn!(user_id=%user.id, message=%error.1, "failed to resume game");
            }
        }
        Ok(Some(row)) => {
            if let Err(error) = send_lobby_to(state, row.get(0), user.id).await {
                tracing::warn!(user_id=%user.id, message=%error.1, "failed to resume lobby");
            }
        }
        Ok(None) => {}
        Err(error) => tracing::error!(%error, user_id=%user.id, "failed to query resumable state"),
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

async fn enqueue_outbox<T: Serialize + ?Sized>(
    tx: &mut Transaction<'_, Postgres>,
    channel: &str,
    event_name: &str,
    data: &T,
) -> Result<(), ApiError> {
    let payload =
        serde_json::to_value(data).map_err(|_| ApiError::internal("Could not queue update"))?;
    sqlx::query("INSERT INTO realtime_outbox(channel,event_name,payload) VALUES($1,$2,$3)")
        .bind(channel)
        .bind(event_name)
        .bind(payload)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn run_outbox(state: AppState) {
    let mut cleanup_counter = 0_u16;
    loop {
        let rows = sqlx::query(
            "UPDATE realtime_outbox
             SET attempts=attempts+1,next_attempt_at=CURRENT_TIMESTAMP+INTERVAL '30 seconds'
             WHERE id IN (
               SELECT id FROM realtime_outbox
               WHERE published_at IS NULL AND next_attempt_at<=CURRENT_TIMESTAMP
               ORDER BY id FOR UPDATE SKIP LOCKED LIMIT 50
             )
             RETURNING id,channel,event_name,payload,attempts",
        )
        .fetch_all(&state.db)
        .await;
        match rows {
            Ok(rows) => {
                for row in rows {
                    let id: i64 = row.get(0);
                    let channel: String = row.get(1);
                    let event_name: String = row.get(2);
                    let payload: serde_json::Value = row.get(3);
                    let attempts: i32 = row.get(4);
                    let published = publish_ably(&state, &channel, &event_name, &payload).await;
                    if published
                        && let Err(error) = sqlx::query(
                            "UPDATE realtime_outbox SET published_at=CURRENT_TIMESTAMP WHERE id=$1",
                        )
                        .bind(id)
                        .execute(&state.db)
                        .await
                    {
                        tracing::error!(%error, outbox_id=id, "could not complete outbox event");
                    }
                    if !published && attempts == 5 {
                        send_deployment_alert(
                            &state,
                            "Ably delivery is failing",
                            &format!("Outbox event {id} failed five times on channel {channel}"),
                        )
                        .await;
                    }
                }
            }
            Err(error) => tracing::error!(%error, "could not claim realtime outbox events"),
        }
        cleanup_counter = cleanup_counter.wrapping_add(1);
        if cleanup_counter == 0 {
            if let Err(error) = sqlx::query(
                "DELETE FROM processed_commands
                 WHERE processed_at<CURRENT_TIMESTAMP-INTERVAL '24 hours'",
            )
            .execute(&state.db)
            .await
            {
                tracing::debug!(%error, "processed command cleanup deferred");
            }
            if let Err(error) = sqlx::query(
                "DELETE FROM realtime_outbox
                 WHERE published_at<CURRENT_TIMESTAMP-INTERVAL '7 days'",
            )
            .execute(&state.db)
            .await
            {
                tracing::debug!(%error, "outbox cleanup deferred");
            }
        }
        tokio::time::sleep(Duration::from_millis(750)).await;
    }
}

async fn send_deployment_alert(state: &AppState, title: &str, detail: &str) {
    let Some(webhook) = &state.alert_webhook else {
        return;
    };
    if let Err(error) = reqwest::Client::new()
        .post(webhook)
        .json(&serde_json::json!({ "title": title, "detail": detail, "service": "ludo-server" }))
        .send()
        .await
    {
        tracing::warn!(%error, "could not send deployment alert");
    }
}

async fn run_match_supervisor(state: AppState) {
    loop {
        if let Err(error) = advance_expired_turn(&state).await {
            tracing::error!(status=%error.0, message=%error.1, "turn supervisor failed");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn advance_expired_turn(state: &AppState) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    let row = sqlx::query(
        "SELECT id,game_state FROM game_lobbies
         WHERE status='playing' AND turn_deadline<=CURRENT_TIMESTAMP
         ORDER BY turn_deadline
         FOR UPDATE SKIP LOCKED LIMIT 1",
    )
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.rollback().await?;
        return Ok(());
    };
    let lobby_id: Uuid = row.get(0);
    let mut game: GameState = serde_json::from_value(row.get(1))
        .map_err(|_| ApiError::internal("Stored game is invalid"))?;
    let command = match game.phase() {
        TurnPhase::AwaitingRoll => random_roll(),
        TurnPhase::AwaitingMove { legal_tokens, .. } => {
            let difficulty = game.current().player.bot_difficulty;
            let decision = ParallelBot::choose(
                &BotRequest::new(game.clone(), difficulty).with_thinking_time_ms(0),
            );
            let Some(token) = decision.token.or_else(|| legal_tokens.first().copied()) else {
                return Ok(());
            };
            GameCommand::Move(token)
        }
    };
    game.apply(command)
        .map_err(|error| ApiError::bad_request(&error.to_string()))?;
    run_bots(&mut game);
    let status = if game.status() == GameStatus::Finished {
        "finished"
    } else {
        "playing"
    };
    sqlx::query(
        "UPDATE game_lobbies
         SET game_state=$2,status=$3::lobby_status,
             turn_deadline=CASE WHEN $3='playing'
               THEN CURRENT_TIMESTAMP+($4::text||' seconds')::interval ELSE NULL END,
             updated_at=CURRENT_TIMESTAMP
         WHERE id=$1",
    )
    .bind(lobby_id)
    .bind(serde_json::to_value(&game).map_err(|_| ApiError::internal("Could not save timed turn"))?)
    .bind(status)
    .bind(TURN_SECONDS.to_string())
    .execute(&mut *tx)
    .await?;
    if state.ably.is_some() {
        let users: Vec<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM lobby_members WHERE lobby_id=$1")
                .bind(lobby_id)
                .fetch_all(&mut *tx)
                .await?;
        let message = ServerMessage::State {
            lobby_id,
            model: GameViewModel::from(&game),
            turn_seconds: TURN_SECONDS,
        };
        for user_id in users {
            enqueue_outbox(&mut tx, &format!("ludo:user:{user_id}"), "event", &message).await?;
        }
    }
    tx.commit().await?;
    tracing::info!(%lobby_id, "advanced expired player turn with temporary AI");
    if state.ably.is_none() {
        broadcast_state(state, lobby_id, &game).await?;
    }
    Ok(())
}

async fn enforce_rate_limit(
    state: &AppState,
    key: &str,
    maximum_per_minute: usize,
) -> Result<(), ApiError> {
    let cutoff = Instant::now()
        .checked_sub(Duration::from_mins(1))
        .unwrap_or_else(Instant::now);
    let mut limits = state.rate_limits.lock().await;
    let attempts = limits.entry(key.to_owned()).or_default();
    while attempts.front().is_some_and(|attempt| *attempt < cutoff) {
        attempts.pop_front();
    }
    if attempts.len() >= maximum_per_minute {
        return Err(ApiError::too_many_requests(
            "Too many requests. Please wait a moment and try again.",
        ));
    }
    attempts.push_back(Instant::now());
    if limits.len() > 10_000 {
        limits.retain(|_, attempts| attempts.back().is_some_and(|attempt| *attempt >= cutoff));
    }
    Ok(())
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

fn cors_layer() -> Result<CorsLayer, Box<dyn std::error::Error>> {
    let Ok(origins) = env::var("LUDO_ALLOWED_ORIGINS") else {
        tracing::warn!(
            "LUDO_ALLOWED_ORIGINS is unset; allowing all origins for backward compatibility"
        );
        return Ok(CorsLayer::permissive());
    };
    let origins = origins
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .map(|origin| origin.trim_end_matches('/'))
        .map(str::parse)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(Any)
        .allow_headers(Any))
}

async fn register(
    State(state): State<AppState>,
    Json(input): Json<Credentials>,
) -> Result<Json<AuthResponse>, ApiError> {
    let email = normalize_email(&input.email)?;
    enforce_rate_limit(&state, &format!("register:{email}"), 5).await?;
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
    enforce_rate_limit(&state, &format!("login:{email}"), 10).await?;
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
fn websocket_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get("sec-websocket-protocol")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .split(',')
                .map(str::trim)
                .find(|value| *value != "ludo")
        })
        .filter(|value| !value.is_empty())
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
    fn too_many_requests(m: &str) -> Self {
        Self(StatusCode::TOO_MANY_REQUESTS, m.to_owned())
    }
    fn code(&self) -> &'static str {
        match self.1.as_str() {
            "Game not found" => "game_not_found",
            "The game advanced; refreshing the board" => "stale_revision",
            "This table is full" => "lobby_full",
            "Only the host can start this game" => "host_required",
            _ if self.0 == StatusCode::UNAUTHORIZED => "unauthorized",
            _ if self.0 == StatusCode::CONFLICT => "conflict",
            _ if self.0 == StatusCode::TOO_MANY_REQUESTS => "rate_limited",
            _ if self.0 == StatusCode::BAD_REQUEST => "invalid_request",
            _ => "internal_error",
        }
    }
}
impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        tracing::error!(%error, "database operation failed");
        let conflict = error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .is_some_and(|code| matches!(code.as_ref(), "23505" | "40001" | "40P01"));
        if conflict {
            Self::conflict("The table changed while processing that request. Please try again.")
        } else {
            Self::internal("Database operation failed")
        }
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.0, Json(serde_json::json!({"error":self.1}))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::{AblyConfig, create_ably_jwt, websocket_token};
    use axum::http::{HeaderMap, HeaderValue};
    use uuid::Uuid;

    #[test]
    fn ably_jwt_signing_has_an_explicit_crypto_provider() {
        let config = AblyConfig {
            key_name: "app.key".to_owned(),
            key_secret: "development-secret".to_owned(),
            http: reqwest::Client::new(),
        };
        let token = create_ably_jwt(&config, Uuid::nil());
        assert!(token.is_ok_and(|value| value.matches('.').count() == 2));
    }

    #[test]
    fn websocket_session_token_is_read_from_the_protocol_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "sec-websocket-protocol",
            HeaderValue::from_static("ludo, session-token"),
        );
        assert_eq!(websocket_token(&headers).ok(), Some("session-token"));
    }
}
