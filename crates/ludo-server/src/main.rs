use std::{
    collections::{HashMap, VecDeque},
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
use ludo_domain::{
    BotDifficulty, Controller, GameCommand, GameState, Player, PlayerColor, PlayerId, Rules,
    TokenId,
};
use ludo_presentation::GameViewModel;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::{Mutex, mpsc};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use uuid::Uuid;

const SESSION_SECONDS: i64 = 60 * 60 * 24 * 30;

#[derive(Clone)]
struct AppState {
    db: PgPool,
    online: Arc<Mutex<OnlineState>>,
}

#[derive(Default)]
struct OnlineState {
    queue: VecDeque<User>,
    sockets: HashMap<Uuid, mpsc::UnboundedSender<ServerMessage>>,
    matches: HashMap<Uuid, Match>,
}

struct Match {
    state: GameState,
    users: [Uuid; 2],
    players: HashMap<Uuid, PlayerId>,
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

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    FindMatch,
    LeaveQueue,
    Sync {
        match_id: Uuid,
    },
    Roll {
        match_id: Uuid,
        revision: u64,
    },
    Move {
        match_id: Uuid,
        revision: u64,
        token: TokenId,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Ready {
        user: User,
    },
    Queued,
    QueueLeft,
    MatchFound {
        match_id: Uuid,
        player: PlayerId,
        model: GameViewModel,
    },
    State {
        match_id: Uuid,
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
    let database_url = env::var("DATABASE_URL")?;
    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await?;
    sqlx::migrate!().run(&db).await?;
    let state = AppState {
        db,
        online: Arc::new(Mutex::new(OnlineState::default())),
    };
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/online", get(websocket))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    let address: SocketAddr = match env::var("LUDO_SERVER_ADDR") {
        Ok(value) => value.parse()?,
        Err(_) => {
            let port = env::var("PORT").unwrap_or_else(|_| "8080".to_owned());
            format!("0.0.0.0:{port}").parse()?
        }
    };
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "online server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
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
    sqlx::query("INSERT INTO users (id,email,display_name,password_hash) VALUES ($1,$2,$3,$4)")
        .bind(id)
        .bind(&email)
        .bind(&display_name)
        .bind(password_hash)
        .execute(&state.db)
        .await
        .map_err(|_| ApiError::conflict("an account with that email already exists"))?;
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
        sqlx::query_as("SELECT id,email,display_name,password_hash FROM users WHERE email = $1")
            .bind(email)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::unauthorized("invalid email or password"))?;
    let parsed = PasswordHash::new(&row.3)
        .map_err(|_| ApiError::internal("stored password hash is invalid"))?;
    Argon2::default()
        .verify_password(input.password.as_bytes(), &parsed)
        .map_err(|_| ApiError::unauthorized("invalid email or password"))?;
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
    let token = bearer(&headers)?;
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(token_hash(token))
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn websocket(
    State(state): State<AppState>,
    Query(query): Query<WsQuery>,
    upgrade: WebSocketUpgrade,
) -> Result<impl IntoResponse, ApiError> {
    let user = authenticate_token(&state, &query.token).await?;
    Ok(upgrade.on_upgrade(move |socket| online_socket(state, user, socket)))
}

async fn online_socket(state: AppState, user: User, socket: WebSocket) {
    let (mut sink, mut source) = socket.split();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    {
        let mut online = state.online.lock().await;
        online.sockets.insert(user.id, sender.clone());
    }
    let _ = sender.send(ServerMessage::Ready { user: user.clone() });
    let writer = tokio::spawn(async move {
        while let Some(message) = receiver.recv().await {
            if sink
                .send(Message::Text(
                    serde_json::to_string(&message).unwrap_or_default().into(),
                ))
                .await
                .is_err()
            {
                break;
            }
        }
    });
    while let Some(Ok(Message::Text(text))) = source.next().await {
        match serde_json::from_str::<ClientMessage>(&text) {
            Ok(message) => handle_online(&state, &user, message).await,
            Err(_) => {
                send_to(
                    &state,
                    user.id,
                    ServerMessage::Error {
                        message: "invalid online message".to_owned(),
                    },
                )
                .await;
            }
        }
    }
    {
        let mut online = state.online.lock().await;
        online.sockets.remove(&user.id);
        online.queue.retain(|queued| queued.id != user.id);
    }
    writer.abort();
}

async fn handle_online(state: &AppState, user: &User, message: ClientMessage) {
    match message {
        ClientMessage::FindMatch => find_match(state, user.clone()).await,
        ClientMessage::LeaveQueue => {
            state
                .online
                .lock()
                .await
                .queue
                .retain(|queued| queued.id != user.id);
            send_to(state, user.id, ServerMessage::QueueLeft).await;
        }
        ClientMessage::Sync { match_id } => {
            let online = state.online.lock().await;
            if let Some(game) = online.matches.get(&match_id)
                && game.players.contains_key(&user.id)
                && let Some(socket) = online.sockets.get(&user.id)
            {
                let _ = socket.send(ServerMessage::State {
                    match_id,
                    model: GameViewModel::from(&game.state),
                });
            }
        }
        ClientMessage::Roll { match_id, revision } => {
            apply_action(state, user.id, match_id, revision, None).await;
        }
        ClientMessage::Move {
            match_id,
            revision,
            token,
        } => {
            apply_action(state, user.id, match_id, revision, Some(token)).await;
        }
    }
}

async fn find_match(state: &AppState, user: User) {
    let mut online = state.online.lock().await;
    if online.queue.iter().any(|queued| queued.id == user.id) {
        return;
    }
    let opponent = online
        .queue
        .iter()
        .position(|queued| online.sockets.contains_key(&queued.id))
        .and_then(|index| online.queue.remove(index));
    let Some(opponent) = opponent else {
        online.queue.push_back(user.clone());
        if let Some(socket) = online.sockets.get(&user.id) {
            let _ = socket.send(ServerMessage::Queued);
        }
        return;
    };
    let match_id = Uuid::new_v4();
    let state_game = online_state(&opponent, &user);
    let players = HashMap::from([(opponent.id, player_id(0)), (user.id, player_id(1))]);
    let game = Match {
        state: state_game.clone(),
        users: [opponent.id, user.id],
        players: players.clone(),
    };
    online.matches.insert(match_id, game);
    for matched in [&opponent, &user] {
        if let Some(socket) = online.sockets.get(&matched.id) {
            let _ = socket.send(ServerMessage::MatchFound {
                match_id,
                player: players[&matched.id],
                model: GameViewModel::from(&state_game),
            });
        }
    }
}

async fn apply_action(
    app: &AppState,
    user: Uuid,
    match_id: Uuid,
    revision: u64,
    token: Option<TokenId>,
) {
    let mut online = app.online.lock().await;
    let Some(game) = online.matches.get_mut(&match_id) else {
        return;
    };
    let Some(player) = game.players.get(&user).copied() else {
        return;
    };
    if game.state.revision() != revision || game.state.current().player.id != player {
        return;
    }
    let command = token.map_or_else(
        || {
            GameCommand::Roll(
                ludo_domain::DiceValue::new(rand::rng().random_range(1..=6))
                    .unwrap_or_else(|| std::process::abort()),
            )
        },
        GameCommand::Move,
    );
    if game.state.apply(command).is_err() {
        return;
    }
    let snapshot = game.state.clone();
    let users = game.users;
    for id in users {
        if let Some(socket) = online.sockets.get(&id) {
            let _ = socket.send(ServerMessage::State {
                match_id,
                model: GameViewModel::from(&snapshot),
            });
        }
    }
}

async fn send_to(state: &AppState, user: Uuid, message: ServerMessage) {
    let online = state.online.lock().await;
    if let Some(socket) = online.sockets.get(&user) {
        let _ = socket.send(message);
    }
}

fn online_state(first: &User, second: &User) -> GameState {
    let players = vec![
        Player {
            id: player_id(0),
            name: first.display_name.clone(),
            color: PlayerColor::Red,
            controller: Controller::Human,
            bot_difficulty: BotDifficulty::Medium,
        },
        Player {
            id: player_id(1),
            name: second.display_name.clone(),
            color: PlayerColor::Green,
            controller: Controller::Human,
            bot_difficulty: BotDifficulty::Medium,
        },
    ];
    GameState::new(players, Rules::default()).unwrap_or_else(|_| std::process::abort())
}

fn player_id(index: u8) -> PlayerId {
    PlayerId::new(index).unwrap_or_else(|| std::process::abort())
}

async fn issue_session(state: &AppState, user: User) -> Result<Json<AuthResponse>, ApiError> {
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query("INSERT INTO sessions (token_hash,user_id,expires_at) VALUES ($1,$2,$3)")
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
    let row: (Uuid, String, String) = sqlx::query_as(
        "SELECT users.id,users.email,users.display_name FROM sessions JOIN users ON users.id=sessions.user_id WHERE sessions.token_hash=$1 AND sessions.expires_at>$2",
    )
    .bind(token_hash(token))
    .bind(now())
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::unauthorized("login required"))?;
    Ok(User {
        id: row.0,
        email: row.1,
        display_name: row.2,
    })
}

fn bearer(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::unauthorized("login required"))
}

fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

fn normalize_email(value: &str) -> Result<String, ApiError> {
    let value = value.trim().to_ascii_lowercase();
    if value.len() > 254 || !value.contains('@') {
        return Err(ApiError::bad_request("enter a valid email"));
    }
    Ok(value)
}

fn normalize_name(value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if !(2..=24).contains(&value.chars().count()) {
        return Err(ApiError::bad_request(
            "display name must be 2–24 characters",
        ));
    }
    Ok(value.to_owned())
}

fn validate_password(value: &str) -> Result<(), ApiError> {
    if value.len() < 10 || value.len() > 128 {
        return Err(ApiError::bad_request("password must be 10–128 characters"));
    }
    Ok(())
}

fn hash_password(value: &str) -> Result<String, ApiError> {
    Argon2::default()
        .hash_password(value.as_bytes(), &SaltString::generate(&mut OsRng))
        .map(|hash| hash.to_string())
        .map_err(|_| ApiError::internal("password hashing failed"))
}

struct ApiError(StatusCode, String);

impl ApiError {
    fn bad_request(message: &str) -> Self {
        Self(StatusCode::BAD_REQUEST, message.to_owned())
    }
    fn unauthorized(message: &str) -> Self {
        Self(StatusCode::UNAUTHORIZED, message.to_owned())
    }
    fn conflict(message: &str) -> Self {
        Self(StatusCode::CONFLICT, message.to_owned())
    }
    fn internal(message: &str) -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR, message.to_owned())
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(_: sqlx::Error) -> Self {
        Self::internal("database operation failed")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}
