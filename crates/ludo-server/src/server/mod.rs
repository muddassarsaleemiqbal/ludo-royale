//! Online Ludo server composition root.
//!
//! Shared wire types live here because every feature module consumes the same
//! websocket contract. Behavior is isolated in focused modules below.

use std::{
    collections::{HashMap, VecDeque},
    env,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderName, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post},
};
use futures_util::SinkExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use ludo_ai::{BotRequest, ParallelBot};
use ludo_domain::{
    BotDifficulty, Controller, DiceValue, GameCommand, GameState, GameStatus, Player, PlayerColor,
    PlayerId, RulePreset, TokenId, TurnPhase,
};
use ludo_presentation::GameViewModel;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgPoolOptions};
use tokio::sync::{Mutex, RwLock, mpsc};
use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use uuid::Uuid;

mod admin;
mod auth;
mod bootstrap;
mod config;
mod error;
#[cfg(test)]
mod integration_tests;
mod lobby;
mod matches;
mod metrics;
mod realtime;
mod social;
mod support;

use admin::{admin_delete_user, admin_overview};
use auth::{
    authenticate_header, authenticate_token, cancel_deletion, login, logout, me, now, register,
    request_deletion, websocket_token,
};
pub(crate) use bootstrap::run;
use config::ServerConfig;
use error::ApiError;
use lobby::{
    accept_joining_player, broadcast_lobbies, create_lobby, kick_player, leave_lobby,
    maintain_lobby_lifecycle, quick_match, request_join, respond_join, resume_user_state,
    send_lobbies, send_lobby, send_lobby_to, update_lobby,
};
use matches::{
    add_activity, apply_action, broadcast_presence, end_game, leave_match, run_match_supervisor,
    spectate, start_game, sync_game, vote_rematch,
};
use metrics::{Metrics, metrics};
use realtime::{ably_token, enqueue_outbox, publish_ably, run_outbox, send_to, websocket};
use social::{
    invite_friend, level_for_xp, load_user, ranked_match, remove_friend, respond_friend_invite,
    respond_friend_request, search_players, send_friend_request, send_hub, send_replay,
    set_cosmetics,
};
use support::{
    cors_layer, enforce_rate_limit, health_live, health_ready, parse_difficulty, parse_preset,
    player_id, validate_lobby_options,
};

const SESSION_SECONDS: i64 = 60 * 60 * 24 * 30;
const TURN_SECONDS: u16 = 30;
const BOT_ROLL_PAUSE_MS: u64 = 650;
const BOT_MOVE_PAUSE_MS: u64 = 850;

#[derive(Clone)]
struct AppState {
    db: PgPool,
    sockets: Arc<Mutex<HashMap<Uuid, mpsc::Sender<ServerMessage>>>>,
    ably: Option<AblyConfig>,
    rate_limits: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
    alert_webhook: Option<String>,
    config: Arc<ServerConfig>,
    metrics: Arc<Metrics>,
    leaderboard_cache: Arc<RwLock<Option<LeaderboardCache>>>,
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
    Ping,
    ListLobbies,
    GetHub,
    SearchPlayers {
        query: String,
    },
    SendFriendRequest {
        user_id: Uuid,
    },
    RespondFriendRequest {
        user_id: Uuid,
        accept: bool,
    },
    RemoveFriend {
        user_id: Uuid,
    },
    InviteFriend {
        lobby_id: Uuid,
        user_id: Uuid,
    },
    RespondFriendInvite {
        invite_id: Uuid,
        accept: bool,
    },
    SetCosmetics {
        dice_theme: String,
        token_theme: String,
    },
    GetReplay {
        match_id: Uuid,
    },
    RankedMatch,
    CreateLobby {
        name: String,
        rule_preset: String,
        bot_difficulty: String,
        is_public: bool,
        turn_seconds: u16,
    },
    RequestJoin {
        lobby_id: Uuid,
    },
    JoinByCode {
        invite_code: String,
    },
    CancelJoin {
        lobby_id: Uuid,
    },
    RespondJoin {
        request_id: Uuid,
        accept: bool,
    },
    LeaveLobby {
        lobby_id: Uuid,
    },
    LeaveMatch {
        lobby_id: Uuid,
    },
    EndGame {
        lobby_id: Uuid,
    },
    KickPlayer {
        lobby_id: Uuid,
        user_id: Uuid,
    },
    SetReady {
        lobby_id: Uuid,
        ready: bool,
    },
    UpdateLobby {
        lobby_id: Uuid,
        rule_preset: String,
        bot_difficulty: String,
        is_public: bool,
        turn_seconds: u16,
        rematch_mode: String,
    },
    QuickMatch {
        rule_preset: String,
        bot_difficulty: String,
    },
    Spectate {
        lobby_id: Uuid,
    },
    Chat {
        lobby_id: Uuid,
        body: String,
    },
    React {
        lobby_id: Uuid,
        emoji: String,
    },
    VoteRematch {
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
    #[serde(default)]
    protocol_version: Option<u16>,
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
    ready: bool,
    presence: String,
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
    invite_code: String,
    is_public: bool,
    turn_seconds: i16,
    ranked: bool,
    rematch_mode: String,
    spectator_count: i64,
    seats: Vec<SeatView>,
    requests: Vec<JoinRequestView>,
}

#[derive(Debug, Clone, Serialize)]
struct ActivityView {
    id: i64,
    kind: String,
    message: String,
    created_at: String,
}

#[derive(Debug, Clone, Serialize)]
struct ProfileView {
    user_id: Uuid,
    display_name: String,
    xp: i64,
    level: i64,
    matches: i32,
    wins: i32,
    current_streak: i32,
    best_streak: i32,
    rating: i32,
    selected_dice: String,
    selected_tokens: String,
}

#[derive(Debug, Clone, Serialize)]
struct FriendView {
    user_id: Uuid,
    display_name: String,
    level: i64,
    rating: i32,
    relationship: String,
    presence: String,
}

#[derive(Debug, Clone, Serialize)]
struct MatchView {
    id: Uuid,
    played_at: String,
    placement: i16,
    xp_earned: i32,
    rating_delta: i32,
    ranked: bool,
    opponents: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ChallengeView {
    key: &'static str,
    title: &'static str,
    progress: i32,
    target: i32,
    reward: i32,
    claimed: bool,
}

#[derive(Debug, Clone, Serialize)]
struct LeaderboardView {
    rank: i64,
    user_id: Uuid,
    display_name: String,
    rating: i32,
    matches: i32,
    wins: i32,
}

struct LeaderboardCache {
    season_id: Uuid,
    expires_at: Instant,
    rows: Vec<LeaderboardView>,
}

#[derive(Debug, Clone, Serialize)]
struct InviteView {
    id: Uuid,
    lobby_id: Uuid,
    lobby_name: String,
    sender_name: String,
}

#[derive(Debug, Clone, Serialize)]
struct HubView {
    profile: ProfileView,
    friends: Vec<FriendView>,
    matches: Vec<MatchView>,
    achievements: Vec<String>,
    challenges: Vec<ChallengeView>,
    leaderboard: Vec<LeaderboardView>,
    season_name: String,
    season_ends_at: String,
    invites: Vec<InviteView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Ready {
        user: User,
        protocol_version: u16,
    },
    LobbyList {
        lobbies: Vec<LobbySummary>,
    },
    Lobby {
        lobby: LobbyView,
    },
    Hub {
        hub: HubView,
    },
    SearchResults {
        players: Vec<FriendView>,
    },
    Replay {
        match_id: Uuid,
        frames: Vec<GameViewModel>,
    },
    Presence {
        lobby_id: Uuid,
        seats: Vec<SeatView>,
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
    SpectatorStarted {
        lobby_id: Uuid,
        model: GameViewModel,
        turn_seconds: u16,
    },
    Activity {
        lobby_id: Uuid,
        event: ActivityView,
    },
    Feed {
        lobby_id: Uuid,
        events: Vec<ActivityView>,
    },
    RematchUpdate {
        lobby_id: Uuid,
        votes: i64,
        needed: i64,
    },
    MatchEnded {
        lobby_id: Uuid,
        message: String,
    },
    Ack {
        command_id: Uuid,
    },
    Pong,
    Error {
        command_id: Option<Uuid>,
        code: &'static str,
        message: String,
        recoverable: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::{AblyConfig, auth::websocket_token, matches::run_bots, realtime::create_ably_jwt};
    use axum::http::{HeaderMap, HeaderValue};
    use ludo_domain::{Controller, GameState, Rules, standard_players};
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

    #[test]
    fn websocket_session_token_rejects_missing_or_malformed_protocols() {
        let headers = HeaderMap::new();
        assert!(websocket_token(&headers).is_err());

        for value in ["session-token", "chat, session-token", "ludo,", "ludo,   "] {
            let mut headers = HeaderMap::new();
            headers.insert(
                "sec-websocket-protocol",
                HeaderValue::from_str(value).unwrap_or_else(|_| std::process::abort()),
            );
            assert!(
                websocket_token(&headers).is_err(),
                "{value} must be rejected"
            );
        }
    }

    #[test]
    fn online_bots_produce_ordered_presentation_frames() {
        let mut players = standard_players();
        players[0].controller = Controller::Bot;
        players[1].controller = Controller::Human;
        let mut game =
            GameState::new(players, Rules::default()).unwrap_or_else(|_| std::process::abort());
        let initial_revision = game.revision();

        let frames = run_bots(&mut game);

        assert!(!frames.is_empty());
        assert!(frames[0].model.revision > initial_revision);
        // A no-move roll can be followed by the authoritative next-turn
        // frame at the same revision, after its brief dice animation.
        assert!(
            frames
                .windows(2)
                .all(|pair| { pair[0].model.revision <= pair[1].model.revision })
        );
        assert!(frames.iter().any(|frame| frame.model.dice.is_some()));
    }
}
