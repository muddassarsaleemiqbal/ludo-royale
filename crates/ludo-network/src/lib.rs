//! Versioned authoritative LAN/private-room multiplayer.

use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, BufWriter, Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use flume::{Receiver, Sender};
use ludo_domain::{
    Controller, DiceValue, GameCommand, GameEvent, GameState, GameStatus, PlayerId, TokenId,
};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Wire schema spoken by hosts and clients.
pub const PROTOCOL_VERSION: u16 = 4;
/// DNS-SD service used for zero-configuration room discovery.
pub const LAN_SERVICE_TYPE: &str = "_ludo-royale._tcp.local.";
const MAX_PLAYER_NAME_CHARS: usize = 24;
const MAX_PENDING_JOIN_REQUESTS: usize = 16;
const MAX_JOIN_RESOLUTIONS: usize = 64;
const MAX_CLIENT_CONNECTIONS: usize = 32;
const MAX_REQUEST_BYTES: usize = 16 * 1024;
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const RECONNECT_GRACE: Duration = Duration::from_secs(30);
const JOIN_REQUEST_TTL: Duration = Duration::from_mins(1);
const JOIN_RESOLUTION_TTL: Duration = Duration::from_mins(2);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const CLIENT_IO_TIMEOUT: Duration = Duration::from_secs(3);
const SERVER_READ_TIMEOUT: Duration = Duration::from_millis(250);
const SERVER_WRITE_TIMEOUT: Duration = Duration::from_secs(2);

/// Returns usable non-loopback addresses for manual direct connection.
#[must_use]
pub fn local_lan_addresses(port: u16) -> Vec<SocketAddr> {
    let mut addresses = if_addrs::get_if_addrs().map_or_else(
        |_| Vec::new(),
        |interfaces| {
            interfaces
                .into_iter()
                .map(|interface| interface.ip())
                .filter(|address| !address.is_loopback())
                .map(|address| SocketAddr::new(address, port))
                .collect::<Vec<_>>()
        },
    );
    addresses.sort_by_key(|address| (!address.is_ipv4(), *address));
    addresses.dedup();
    addresses
}

/// Six-character private room identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomCode(String);

impl RoomCode {
    /// Generates a readable private code.
    #[must_use]
    pub fn generate() -> Self {
        const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
        let mut rng = rand::rng();
        let code = (0..6)
            .map(|_| char::from(ALPHABET[rng.random_range(0..ALPHABET.len())]))
            .collect();
        Self(code)
    }

    /// String representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque secret used to reclaim a seat after disconnecting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconnectToken(String);

impl ReconnectToken {
    fn generate() -> Self {
        let mut rng = rand::rng();
        Self(format!("{:032x}", rng.random::<u128>()))
    }
}

/// Opaque identifier for one host-moderated join request.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JoinRequestId(String);

impl JoinRequestId {
    /// Generates a cryptographically unguessable request correlation ID.
    #[must_use]
    pub fn generate() -> Self {
        let mut rng = rand::rng();
        Self(format!("{:032x}", rng.random::<u128>()))
    }

    /// Stable string form, suitable for UI element identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Public information shown to the host before a player is admitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinRequest {
    /// Opaque request identity.
    pub id: JoinRequestId,
    /// Normalized player display name.
    pub name: String,
}

/// Lifecycle of an authoritative private room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbyPhase {
    /// Players may join and replace computer seats.
    Waiting,
    /// The host closed the lobby and started play.
    Playing,
}

/// Kind of occupant currently controlling a color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbySeatKind {
    /// The player who created the room.
    Host,
    /// A human connected from another app instance.
    RemoteHuman,
    /// A computer controlled by the authoritative host.
    Computer,
}

/// Public presence information for one color.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbySeat {
    /// Stable game player.
    pub player: PlayerId,
    /// Name displayed in the lobby and match.
    pub name: String,
    /// Controller role.
    pub kind: LobbySeatKind,
    /// Whether a human has a live transport connection.
    pub connected: bool,
}

/// Canonical lobby roster distributed with every host response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbySnapshot {
    /// Monotonic presence/phase revision used by low-cost synchronization.
    pub revision: u64,
    /// Whether joining is still allowed.
    pub phase: LobbyPhase,
    /// All active colors, including computer seats.
    pub seats: Vec<LobbySeat>,
}

impl LobbySnapshot {
    /// Number of human-controlled seats.
    #[must_use]
    pub fn human_count(&self) -> usize {
        self.seats
            .iter()
            .filter(|seat| !matches!(seat.kind, LobbySeatKind::Computer))
            .count()
    }
}

/// Client intent. Dice values are deliberately generated only by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientAction {
    /// Request an authoritative roll.
    Roll,
    /// Move a legal token.
    Move(TokenId),
}

/// Newline-delimited JSON request envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Ask the host to approve a new player.
    RequestJoin {
        /// Wire version.
        protocol: u16,
        /// Client-generated correlation ID, making retries idempotent.
        request: JoinRequestId,
        /// Optional fallback room code. Discovery requests do not need one.
        room: Option<String>,
        /// Player name displayed to everyone in the lobby.
        name: String,
    },
    /// Poll a pending request until the host accepts or rejects it.
    JoinStatus {
        /// Wire version.
        protocol: u16,
        /// Pending request identity.
        request: JoinRequestId,
    },
    /// Reclaim a known seat.
    Reconnect {
        /// Wire version.
        protocol: u16,
        /// Optional fallback room code.
        room: Option<String>,
        /// Previously issued secret.
        token: ReconnectToken,
    },
    /// Submit intent against an exact state revision.
    Command {
        /// Wire version.
        protocol: u16,
        /// Reconnection secret.
        token: ReconnectToken,
        /// Client's source revision.
        expected_revision: u64,
        /// Intent.
        action: ClientAction,
    },
    /// Request the latest authoritative snapshot.
    Sync {
        /// Wire version.
        protocol: u16,
        /// Reconnection secret.
        token: ReconnectToken,
        /// Last game revision already held by the client.
        known_state_revision: u64,
        /// Last lobby revision already held by the client.
        known_lobby_revision: u64,
    },
}

/// Newline-delimited JSON host response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostMessage {
    /// The request is waiting for the host's decision.
    JoinPending {
        /// Wire version.
        protocol: u16,
        /// Pending request identity and display name.
        request: JoinRequest,
        /// Current canonical lobby roster.
        lobby: LobbySnapshot,
    },
    /// Seat was assigned or reclaimed.
    Welcome {
        /// Wire version.
        protocol: u16,
        /// Assigned player.
        player: PlayerId,
        /// Reconnection secret to persist.
        token: ReconnectToken,
        /// Current canonical state.
        state: GameState,
        /// Current canonical lobby roster.
        lobby: LobbySnapshot,
    },
    /// Accepted deterministic transition.
    Applied {
        /// Wire version.
        protocol: u16,
        /// Exact command chosen by the host.
        command: GameCommand,
        /// Resulting facts.
        events: Vec<GameEvent>,
        /// Current canonical state.
        state: GameState,
        /// Current canonical lobby roster.
        lobby: LobbySnapshot,
    },
    /// Latest canonical snapshot.
    Snapshot {
        /// Wire version.
        protocol: u16,
        /// Current canonical state.
        state: GameState,
        /// Current canonical lobby roster.
        lobby: LobbySnapshot,
    },
    /// Both the game and lobby still match the client's known revisions.
    UpToDate {
        /// Wire version.
        protocol: u16,
        /// Current game revision.
        state_revision: u64,
        /// Current lobby revision.
        lobby_revision: u64,
    },
    /// Request was rejected without mutating state.
    Rejected {
        /// Wire version.
        protocol: u16,
        /// Stable human-readable reason.
        reason: String,
        /// Current canonical state when available.
        state: Option<GameState>,
        /// Current lobby when available.
        lobby: Option<LobbySnapshot>,
    },
}

#[derive(Debug, Clone)]
struct Seat {
    player: PlayerId,
    token: ReconnectToken,
    name: String,
    kind: LobbySeatKind,
    connected: bool,
    disconnected_at: Option<Instant>,
    connection_generation: u64,
}

#[derive(Debug, Clone)]
struct PendingJoin {
    request: JoinRequest,
    submitted_at: Instant,
}

#[derive(Debug, Clone)]
enum JoinDecision {
    Accepted {
        player: PlayerId,
        token: ReconnectToken,
    },
    Rejected(String),
}

#[derive(Debug, Clone)]
struct JoinResolution {
    decision: JoinDecision,
    resolved_at: Instant,
}

/// In-memory authoritative room state.
#[derive(Debug)]
pub struct AuthoritativeRoom {
    code: RoomCode,
    state: GameState,
    seats: Vec<Seat>,
    phase: LobbyPhase,
    lobby_revision: u64,
    pending_joins: Vec<PendingJoin>,
    join_resolutions: HashMap<JoinRequestId, JoinResolution>,
}

impl AuthoritativeRoom {
    /// Creates a private room around a deterministic initial state.
    #[must_use]
    pub fn new(state: GameState) -> Self {
        let host_id = state.players()[0].player.id;
        let host_name = state.players()[0].player.name.clone();
        Self {
            code: RoomCode::generate(),
            state,
            seats: vec![Seat {
                player: host_id,
                token: ReconnectToken::generate(),
                name: host_name,
                kind: LobbySeatKind::Host,
                connected: false,
                disconnected_at: None,
                connection_generation: 0,
            }],
            phase: LobbyPhase::Waiting,
            lobby_revision: 0,
            pending_joins: Vec::new(),
            join_resolutions: HashMap::new(),
        }
    }

    /// Private room code.
    #[must_use]
    pub const fn code(&self) -> &RoomCode {
        &self.code
    }

    /// Current authoritative snapshot.
    #[must_use]
    pub const fn state(&self) -> &GameState {
        &self.state
    }

    /// Current player presence.
    #[must_use]
    pub fn lobby(&self) -> LobbySnapshot {
        let seats = self
            .state
            .players()
            .iter()
            .map(|player_state| {
                self.seats
                    .iter()
                    .find(|seat| seat.player == player_state.player.id)
                    .map_or_else(
                        || LobbySeat {
                            player: player_state.player.id,
                            name: player_state.player.name.clone(),
                            kind: LobbySeatKind::Computer,
                            connected: true,
                        },
                        |seat| LobbySeat {
                            player: seat.player,
                            name: seat.name.clone(),
                            kind: seat.kind,
                            connected: seat.connected,
                        },
                    )
            })
            .collect();
        LobbySnapshot {
            revision: self.lobby_revision,
            phase: self.phase,
            seats,
        }
    }

    /// Host credentials used by the local app without consuming a guest seat.
    #[must_use]
    pub fn host_credentials(&self) -> (PlayerId, ReconnectToken) {
        let seat = &self.seats[0];
        (seat.player, seat.token.clone())
    }

    /// Join requests currently awaiting a host decision.
    #[must_use]
    pub fn pending_join_requests(&mut self) -> Vec<JoinRequest> {
        self.expire_join_requests();
        self.pending_joins
            .iter()
            .map(|pending| pending.request.clone())
            .collect()
    }

    /// Accepts a pending player and reserves the next computer seat.
    ///
    /// # Errors
    ///
    /// Returns a stable reason when the request disappeared or no seat remains.
    pub fn accept_join_request(
        &mut self,
        request: &JoinRequestId,
    ) -> Result<LobbySnapshot, String> {
        self.expire_join_requests();
        let Some(index) = self
            .pending_joins
            .iter()
            .position(|pending| pending.request.id == *request)
        else {
            return Err("join request is no longer pending".to_owned());
        };
        let pending = self.pending_joins.remove(index);
        match self.assign_remote_player(&pending.request.name, false) {
            Ok((player, token)) => {
                self.insert_join_resolution(
                    pending.request.id,
                    JoinDecision::Accepted { player, token },
                );
                Ok(self.lobby())
            }
            Err(reason) => {
                self.insert_join_resolution(
                    pending.request.id,
                    JoinDecision::Rejected(reason.clone()),
                );
                Err(reason)
            }
        }
    }

    /// Rejects a pending player without changing the roster.
    ///
    /// # Errors
    ///
    /// Returns a stable reason when the request is no longer pending.
    pub fn reject_join_request(
        &mut self,
        request: &JoinRequestId,
    ) -> Result<LobbySnapshot, String> {
        self.expire_join_requests();
        let Some(index) = self
            .pending_joins
            .iter()
            .position(|pending| pending.request.id == *request)
        else {
            return Err("join request is no longer pending".to_owned());
        };
        let pending = self.pending_joins.remove(index);
        self.insert_join_resolution(
            pending.request.id,
            JoinDecision::Rejected("The host declined your request.".to_owned()),
        );
        Ok(self.lobby())
    }

    /// Closes the lobby to new joins.
    pub fn start_match(&mut self) {
        let disconnected = self
            .seats
            .iter()
            .filter(|seat| matches!(seat.kind, LobbySeatKind::RemoteHuman) && !seat.connected)
            .map(|seat| seat.player)
            .collect::<Vec<_>>();
        for player in disconnected {
            self.fallback_to_computer(player);
        }
        let pending = self.pending_joins.drain(..).collect::<Vec<_>>();
        for pending in pending {
            self.insert_join_resolution(
                pending.request.id,
                JoinDecision::Rejected("The host started the match.".to_owned()),
            );
        }
        if !matches!(self.phase, LobbyPhase::Playing) {
            self.phase = LobbyPhase::Playing;
            self.touch_lobby();
        }
    }

    /// Handles one protocol message.
    #[must_use]
    pub fn handle(&mut self, message: ClientMessage) -> HostMessage {
        self.expire_disconnected_players();
        self.expire_join_requests();
        self.expire_join_resolutions();
        if message_protocol(&message) != PROTOCOL_VERSION {
            return Self::reject_public("protocol version mismatch");
        }
        match message {
            ClientMessage::RequestJoin {
                request,
                room,
                name,
                ..
            } => self.request_join(room.as_deref(), &name, &request),
            ClientMessage::JoinStatus { request, .. } => self.join_status(&request),
            ClientMessage::Reconnect { room, token, .. } => self.reconnect(room.as_deref(), &token),
            ClientMessage::Command {
                token,
                expected_revision,
                action,
                ..
            } => self.command(&token, expected_revision, action),
            ClientMessage::Sync {
                token,
                known_state_revision,
                known_lobby_revision,
                ..
            } => self.sync(&token, known_state_revision, known_lobby_revision),
        }
    }

    fn request_join(
        &mut self,
        room: Option<&str>,
        name: &str,
        request: &JoinRequestId,
    ) -> HostMessage {
        if room.is_some_and(|room| room != self.code.as_str()) {
            return Self::reject_public("room code does not match");
        }
        if let Some(pending) = self
            .pending_joins
            .iter()
            .find(|pending| pending.request.id == *request)
        {
            return HostMessage::JoinPending {
                protocol: PROTOCOL_VERSION,
                request: pending.request.clone(),
                lobby: self.lobby(),
            };
        }
        if self.join_resolutions.contains_key(request) {
            return self.join_status(request);
        }
        if matches!(self.phase, LobbyPhase::Playing) {
            return Self::reject_public("the match has already started");
        }
        let name = normalize_player_name(name);
        if name.is_empty() {
            return Self::reject_public("enter a player name before joining");
        }
        if self
            .seats
            .iter()
            .any(|seat| seat.name.eq_ignore_ascii_case(&name))
            || self
                .pending_joins
                .iter()
                .any(|pending| pending.request.name.eq_ignore_ascii_case(&name))
        {
            return Self::reject_public("that player name is already in use");
        }
        if self.seats.len() >= self.state.players().len() {
            return Self::reject_public("room is full");
        }
        if self.pending_joins.len() >= MAX_PENDING_JOIN_REQUESTS {
            return Self::reject_public("too many players are already waiting for approval");
        }
        let request = JoinRequest {
            id: request.clone(),
            name,
        };
        self.pending_joins.push(PendingJoin {
            request: request.clone(),
            submitted_at: Instant::now(),
        });
        HostMessage::JoinPending {
            protocol: PROTOCOL_VERSION,
            request,
            lobby: self.lobby(),
        }
    }

    fn join_status(&mut self, request: &JoinRequestId) -> HostMessage {
        self.expire_join_requests();
        if let Some(pending) = self
            .pending_joins
            .iter()
            .find(|pending| pending.request.id == *request)
        {
            return HostMessage::JoinPending {
                protocol: PROTOCOL_VERSION,
                request: pending.request.clone(),
                lobby: self.lobby(),
            };
        }
        // A response can be lost after the host writes it. Keep the decision
        // until its TTL so polling the same client-generated ID is idempotent.
        match self
            .join_resolutions
            .get(request)
            .map(|resolution| resolution.decision.clone())
        {
            Some(JoinDecision::Accepted { player, token }) => {
                if self.seat_index(&token).is_none() {
                    return Self::reject_public("the approved seat expired; request access again");
                }
                self.set_connected(&token);
                HostMessage::Welcome {
                    protocol: PROTOCOL_VERSION,
                    player,
                    token,
                    state: self.state.clone(),
                    lobby: self.lobby(),
                }
            }
            Some(JoinDecision::Rejected(reason)) => Self::reject_public(&reason),
            None => Self::reject_public("join request expired or is unknown"),
        }
    }

    fn reconnect(&mut self, room: Option<&str>, token: &ReconnectToken) -> HostMessage {
        if room.is_some_and(|room| room != self.code.as_str()) {
            return Self::reject_public("room code does not match");
        }
        let Some(index) = self.seat_index(token) else {
            return Self::reject_public("unknown reconnect token");
        };
        self.set_connected(token);
        let seat = &self.seats[index];
        HostMessage::Welcome {
            protocol: PROTOCOL_VERSION,
            player: seat.player,
            token: seat.token.clone(),
            state: self.state.clone(),
            lobby: self.lobby(),
        }
    }

    fn assign_remote_player(
        &mut self,
        name: &str,
        connected: bool,
    ) -> Result<(PlayerId, ReconnectToken), String> {
        let Some(player) = self
            .state
            .players()
            .iter()
            .find(|candidate| {
                !self
                    .seats
                    .iter()
                    .any(|seat| seat.player == candidate.player.id)
            })
            .map(|candidate| candidate.player.id)
        else {
            return Err("room is full".to_owned());
        };
        self.replace_lobby_player(player, name, Controller::Human)
            .map_err(|()| "the player seat could not be configured".to_owned())?;
        let token = ReconnectToken::generate();
        self.seats.push(Seat {
            player,
            token: token.clone(),
            name: name.to_owned(),
            kind: LobbySeatKind::RemoteHuman,
            connected,
            disconnected_at: (!connected).then(Instant::now),
            connection_generation: 0,
        });
        self.touch_lobby();
        Ok((player, token))
    }

    fn sync(
        &mut self,
        token: &ReconnectToken,
        known_state_revision: u64,
        known_lobby_revision: u64,
    ) -> HostMessage {
        if self.seat_index(token).is_none() {
            return Self::reject_public("unknown reconnect token");
        }
        self.set_connected(token);
        if known_state_revision == self.state.revision()
            && known_lobby_revision == self.lobby_revision
        {
            HostMessage::UpToDate {
                protocol: PROTOCOL_VERSION,
                state_revision: self.state.revision(),
                lobby_revision: self.lobby_revision,
            }
        } else {
            HostMessage::Snapshot {
                protocol: PROTOCOL_VERSION,
                state: self.state.clone(),
                lobby: self.lobby(),
            }
        }
    }

    fn command(
        &mut self,
        token: &ReconnectToken,
        expected_revision: u64,
        action: ClientAction,
    ) -> HostMessage {
        let Some(seat) = self.seat(token) else {
            return Self::reject_public("unknown reconnect token");
        };
        let player = seat.player;
        let host_controls_computer = matches!(seat.kind, LobbySeatKind::Host)
            && matches!(self.state.current().player.controller, Controller::Bot);
        if !matches!(self.phase, LobbyPhase::Playing) {
            return self.reject_with_snapshot("the match has not started");
        }
        if expected_revision != self.state.revision() {
            return self.reject_with_snapshot("stale revision; synchronize and retry");
        }
        if !matches!(self.state.status(), GameStatus::Playing)
            || (self.state.current().player.id != player && !host_controls_computer)
        {
            return self.reject_with_snapshot("it is not this seat's turn");
        }
        let command = match action {
            ClientAction::Roll => {
                let value = rand::rng().random_range(1..=6);
                let Some(dice) = DiceValue::new(value) else {
                    return self.reject_with_snapshot("host dice generation failed");
                };
                GameCommand::Roll(dice)
            }
            ClientAction::Move(token) => GameCommand::Move(token),
        };
        match self.state.apply(command) {
            Ok(events) => HostMessage::Applied {
                protocol: PROTOCOL_VERSION,
                command,
                events,
                state: self.state.clone(),
                lobby: self.lobby(),
            },
            Err(error) => self.reject_with_snapshot(&error.to_string()),
        }
    }

    fn seat(&self, token: &ReconnectToken) -> Option<&Seat> {
        self.seats.iter().find(|seat| seat.token == *token)
    }

    fn seat_index(&self, token: &ReconnectToken) -> Option<usize> {
        self.seats.iter().position(|seat| seat.token == *token)
    }

    fn set_connected(&mut self, token: &ReconnectToken) {
        if let Some(index) = self.seat_index(token) {
            let changed = !self.seats[index].connected;
            self.seats[index].connected = true;
            self.seats[index].disconnected_at = None;
            if changed {
                self.touch_lobby();
            }
        }
    }

    fn attach_transport(&mut self, token: &ReconnectToken) -> Option<u64> {
        let index = self.seat_index(token)?;
        let generation = self.seats[index]
            .connection_generation
            .wrapping_add(1)
            .max(1);
        self.seats[index].connection_generation = generation;
        self.set_connected(token);
        Some(generation)
    }

    fn disconnect(&mut self, token: &ReconnectToken, generation: u64) {
        if let Some(index) = self.seat_index(token)
            && self.seats[index].connection_generation == generation
            && self.seats[index].connected
        {
            self.seats[index].connected = false;
            self.seats[index].disconnected_at = Some(Instant::now());
            self.touch_lobby();
        }
    }

    fn replace_lobby_player(
        &mut self,
        player: PlayerId,
        name: &str,
        controller: Controller,
    ) -> Result<(), ()> {
        self.state
            .update_player_control(player, name.to_owned(), controller)
            .map_err(|_| ())
    }

    fn expire_join_requests(&mut self) {
        let expired = self
            .pending_joins
            .iter()
            .filter(|pending| pending.submitted_at.elapsed() >= JOIN_REQUEST_TTL)
            .map(|pending| pending.request.id.clone())
            .collect::<Vec<_>>();
        self.pending_joins
            .retain(|pending| !expired.contains(&pending.request.id));
        for request in expired {
            self.insert_join_resolution(
                request,
                JoinDecision::Rejected("The join request expired.".to_owned()),
            );
        }
    }

    fn expire_join_resolutions(&mut self) {
        self.join_resolutions
            .retain(|_, resolution| resolution.resolved_at.elapsed() < JOIN_RESOLUTION_TTL);
    }

    fn insert_join_resolution(&mut self, request: JoinRequestId, decision: JoinDecision) {
        self.expire_join_resolutions();
        if self.join_resolutions.len() >= MAX_JOIN_RESOLUTIONS
            && let Some(oldest) = self
                .join_resolutions
                .iter()
                .min_by_key(|(_, resolution)| resolution.resolved_at)
                .map(|(request, _)| request.clone())
        {
            self.join_resolutions.remove(&oldest);
        }
        self.join_resolutions.insert(
            request,
            JoinResolution {
                decision,
                resolved_at: Instant::now(),
            },
        );
    }

    fn expire_disconnected_players(&mut self) {
        let expired = self
            .seats
            .iter()
            .filter(|seat| {
                matches!(seat.kind, LobbySeatKind::RemoteHuman)
                    && seat
                        .disconnected_at
                        .is_some_and(|instant| instant.elapsed() >= RECONNECT_GRACE)
            })
            .map(|seat| seat.player)
            .collect::<Vec<_>>();
        for player in expired {
            self.fallback_to_computer(player);
        }
    }

    fn fallback_to_computer(&mut self, player: PlayerId) {
        let computer_name = self.state.player(player).map_or_else(
            || "Computer".to_owned(),
            |state| format!("{} Computer", state.player.color.name()),
        );
        let _ = self
            .state
            .update_player_control(player, computer_name, Controller::Bot);
        let previous_len = self.seats.len();
        self.seats.retain(|seat| seat.player != player);
        if self.seats.len() != previous_len {
            self.touch_lobby();
        }
    }

    fn touch_lobby(&mut self) {
        self.lobby_revision = self.lobby_revision.saturating_add(1);
    }

    fn reject_public(reason: &str) -> HostMessage {
        HostMessage::Rejected {
            protocol: PROTOCOL_VERSION,
            reason: reason.to_owned(),
            state: None,
            lobby: None,
        }
    }

    fn reject_with_snapshot(&self, reason: &str) -> HostMessage {
        HostMessage::Rejected {
            protocol: PROTOCOL_VERSION,
            reason: reason.to_owned(),
            state: Some(self.state.clone()),
            lobby: Some(self.lobby()),
        }
    }
}

fn normalize_player_name(name: &str) -> String {
    let mut normalized =
        String::with_capacity(name.len().min(MAX_PLAYER_NAME_CHARS.saturating_mul(4)));
    let mut characters = 0_usize;
    let mut pending_space = false;
    for character in name.chars().filter(|character| !character.is_control()) {
        if character.is_whitespace() {
            pending_space = !normalized.is_empty();
            continue;
        }
        if pending_space {
            // Reserve room for both the collapsed separator and the current
            // visible character, otherwise truncation would leave a trailing
            // space and normalization would not be idempotent.
            if characters.saturating_add(2) > MAX_PLAYER_NAME_CHARS {
                break;
            }
            normalized.push(' ');
            characters += 1;
            pending_space = false;
        }
        if characters >= MAX_PLAYER_NAME_CHARS {
            break;
        }
        normalized.push(character);
        characters += 1;
    }
    normalized
}

fn message_protocol(message: &ClientMessage) -> u16 {
    match message {
        ClientMessage::RequestJoin { protocol, .. }
        | ClientMessage::JoinStatus { protocol, .. }
        | ClientMessage::Reconnect { protocol, .. }
        | ClientMessage::Command { protocol, .. }
        | ClientMessage::Sync { protocol, .. } => *protocol,
    }
}

fn host_message_protocol(message: &HostMessage) -> u16 {
    match message {
        HostMessage::JoinPending { protocol, .. }
        | HostMessage::Welcome { protocol, .. }
        | HostMessage::Applied { protocol, .. }
        | HostMessage::Snapshot { protocol, .. }
        | HostMessage::UpToDate { protocol, .. }
        | HostMessage::Rejected { protocol, .. } => *protocol,
    }
}

/// Background TCP host for one authoritative room.
pub struct LanHost {
    address: SocketAddr,
    room: Arc<Mutex<AuthoritativeRoom>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LanHost {
    /// Binds a host and begins accepting newline-delimited JSON requests.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the address cannot be bound.
    pub fn bind(address: SocketAddr, state: GameState) -> Result<Self, NetworkError> {
        let listener = TcpListener::bind(address)?;
        let address = listener.local_addr()?;
        let room = Arc::new(Mutex::new(AuthoritativeRoom::new(state)));
        let worker_room = room.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("ludo-lan-host".to_owned())
            .spawn(move || {
                let mut clients = Vec::with_capacity(MAX_CLIENT_CONNECTIONS);
                while !worker_stop.load(Ordering::Acquire) {
                    reap_finished_threads(&mut clients);
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if worker_stop.load(Ordering::Acquire) {
                                break;
                            }
                            if clients.len() >= MAX_CLIENT_CONNECTIONS
                                || configure_server_stream(&stream).is_err()
                            {
                                continue;
                            }
                            let connection_room = worker_room.clone();
                            let connection_stop = worker_stop.clone();
                            if let Ok(thread) = std::thread::Builder::new()
                                .name("ludo-lan-client".to_owned())
                                .spawn(move || {
                                    serve_connection(stream, &connection_room, &connection_stop);
                                })
                            {
                                clients.push(thread);
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
                for thread in clients {
                    let _ = thread.join();
                }
            })?;
        Ok(Self {
            address,
            room,
            stop,
            thread: Some(thread),
        })
    }

    /// Actual bound address, including an OS-assigned port.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    /// Private room code.
    ///
    /// # Errors
    ///
    /// Returns an error if the room lock was poisoned.
    pub fn room_code(&self) -> Result<RoomCode, NetworkError> {
        self.room
            .lock()
            .map(|room| room.code().clone())
            .map_err(|_| NetworkError::RoomUnavailable)
    }

    /// Credentials for the host's local client connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the room lock was poisoned.
    pub fn host_credentials(&self) -> Result<(PlayerId, ReconnectToken), NetworkError> {
        self.room
            .lock()
            .map(|room| room.host_credentials())
            .map_err(|_| NetworkError::RoomUnavailable)
    }

    /// Current authoritative lobby roster.
    ///
    /// # Errors
    ///
    /// Returns an error if the room lock was poisoned.
    pub fn lobby(&self) -> Result<LobbySnapshot, NetworkError> {
        self.room
            .lock()
            .map(|room| room.lobby())
            .map_err(|_| NetworkError::RoomUnavailable)
    }

    /// Pending players awaiting a host decision.
    ///
    /// # Errors
    ///
    /// Returns an error if the room lock was poisoned.
    pub fn pending_join_requests(&self) -> Result<Vec<JoinRequest>, NetworkError> {
        self.room
            .lock()
            .map(|mut room| room.pending_join_requests())
            .map_err(|_| NetworkError::RoomUnavailable)
    }

    /// Accepts a pending player into the next computer seat.
    ///
    /// # Errors
    ///
    /// Returns an error if the room is unavailable or the request is stale.
    pub fn accept_join_request(
        &self,
        request: &JoinRequestId,
    ) -> Result<LobbySnapshot, NetworkError> {
        self.room
            .lock()
            .map_err(|_| NetworkError::RoomUnavailable)?
            .accept_join_request(request)
            .map_err(NetworkError::JoinRequest)
    }

    /// Rejects a pending player.
    ///
    /// # Errors
    ///
    /// Returns an error if the room is unavailable or the request is stale.
    pub fn reject_join_request(
        &self,
        request: &JoinRequestId,
    ) -> Result<LobbySnapshot, NetworkError> {
        self.room
            .lock()
            .map_err(|_| NetworkError::RoomUnavailable)?
            .reject_join_request(request)
            .map_err(NetworkError::JoinRequest)
    }

    /// Starts the match and prevents additional players from joining.
    ///
    /// # Errors
    ///
    /// Returns an error if the room lock was poisoned.
    pub fn start_match(&self) -> Result<(), NetworkError> {
        self.room
            .lock()
            .map(|mut room| room.start_match())
            .map_err(|_| NetworkError::RoomUnavailable)
    }
}

impl Drop for LanHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let wake_address = match self.address.ip() {
            IpAddr::V4(address) if address.is_unspecified() => {
                SocketAddr::new(Ipv4Addr::LOCALHOST.into(), self.address.port())
            }
            IpAddr::V6(address) if address.is_unspecified() => {
                SocketAddr::new(Ipv6Addr::LOCALHOST.into(), self.address.port())
            }
            _ => self.address,
        };
        let _ = TcpStream::connect_timeout(&wake_address, SERVER_READ_TIMEOUT);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn configure_server_stream(stream: &TcpStream) -> io::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(SERVER_READ_TIMEOUT))?;
    stream.set_write_timeout(Some(SERVER_WRITE_TIMEOUT))
}

fn reap_finished_threads(threads: &mut Vec<JoinHandle<()>>) {
    let mut index = 0;
    while index < threads.len() {
        if threads[index].is_finished() {
            let thread = threads.swap_remove(index);
            let _ = thread.join();
        } else {
            index += 1;
        }
    }
}

fn serve_connection(stream: TcpStream, room: &Arc<Mutex<AuthoritativeRoom>>, stop: &AtomicBool) {
    let Ok(reader_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reader_stream);
    let mut writer = BufWriter::new(stream);
    let mut line = String::with_capacity(512);
    let mut identity = None;
    loop {
        let bytes = match read_bounded_line(&mut reader, &mut line, MAX_REQUEST_BYTES) {
            Ok(0) => break,
            Ok(bytes) => bytes,
            Err(error)
                if line.is_empty()
                    && matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
            {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                continue;
            }
            Err(_) => break,
        };
        if bytes == 0 {
            break;
        }
        let response = serde_json::from_str::<ClientMessage>(line.trim_end())
            .map_err(|error| error.to_string())
            .and_then(|message| {
                room.lock()
                    .map(|mut room| room.handle(message))
                    .map_err(|_| "room lock was poisoned".to_owned())
            })
            .unwrap_or_else(|reason| HostMessage::Rejected {
                protocol: PROTOCOL_VERSION,
                reason,
                state: None,
                lobby: None,
            });
        if let HostMessage::Welcome { token, .. } = &response {
            let same_identity = identity.as_ref().is_some_and(|(known, _)| known == token);
            if !same_identity
                && let Ok(mut room) = room.lock()
                && let Some(generation) = room.attach_transport(token)
            {
                if let Some((previous, previous_generation)) = identity.take() {
                    room.disconnect(&previous, previous_generation);
                }
                identity = Some((token.clone(), generation));
            }
        }
        if serde_json::to_writer(&mut writer, &response).is_err()
            || writer.write_all(b"\n").is_err()
            || writer.flush().is_err()
        {
            break;
        }
        line.clear();
    }
    if let Some((token, generation)) = identity
        && let Ok(mut room) = room.lock()
    {
        room.disconnect(&token, generation);
    }
}

fn read_bounded_line(
    reader: &mut impl BufRead,
    buffer: &mut String,
    limit: usize,
) -> io::Result<usize> {
    buffer.clear();
    let bytes = reader
        .by_ref()
        .take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1))
        .read_line(buffer)?;
    if bytes > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "network message exceeds the size limit",
        ));
    }
    if bytes > 0 && !buffer.ends_with('\n') {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "network message is missing its delimiter",
        ));
    }
    Ok(bytes)
}

/// Blocking client useful from a dedicated application worker.
pub struct LanClient {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
    response: String,
}

impl LanClient {
    /// Connects to a private host.
    ///
    /// # Errors
    ///
    /// Returns an I/O error.
    pub fn connect(address: SocketAddr) -> Result<Self, NetworkError> {
        let stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(CLIENT_IO_TIMEOUT))?;
        stream.set_write_timeout(Some(CLIENT_IO_TIMEOUT))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            reader,
            writer: BufWriter::new(stream),
            response: String::with_capacity(4 * 1024),
        })
    }

    /// Sends one request and waits for one response.
    ///
    /// # Errors
    ///
    /// Returns an I/O or protocol encoding error.
    pub fn request(&mut self, message: &ClientMessage) -> Result<HostMessage, NetworkError> {
        serde_json::to_writer(&mut self.writer, message)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        if read_bounded_line(&mut self.reader, &mut self.response, MAX_RESPONSE_BYTES)? == 0 {
            return Err(NetworkError::Disconnected);
        }
        let message = serde_json::from_str(self.response.trim_end())?;
        let received = host_message_protocol(&message);
        if received != PROTOCOL_VERSION {
            return Err(NetworkError::ProtocolMismatch {
                expected: PROTOCOL_VERSION,
                received,
            });
        }
        Ok(message)
    }
}

/// Commands accepted by the non-blocking client worker.
#[derive(Debug, Clone)]
pub enum LanWorkerRequest {
    /// Connect and ask the host for a room seat.
    RequestJoin {
        /// Host socket address.
        address: SocketAddr,
        /// Stable correlation ID reused across retries.
        request: JoinRequestId,
        /// Optional fallback room code.
        room: Option<String>,
        /// Player name displayed to the lobby.
        name: String,
    },
    /// Poll a host-moderated join request.
    JoinStatus {
        /// Host socket address.
        address: SocketAddr,
        /// Pending request identity.
        request: JoinRequestId,
    },
    /// Connect and reclaim a previous seat.
    Reconnect {
        /// Host socket address.
        address: SocketAddr,
        /// Optional fallback room code.
        room: Option<String>,
        /// Saved secret.
        token: ReconnectToken,
    },
    /// Send game intent over the existing connection.
    Action {
        /// Seat secret.
        token: ReconnectToken,
        /// Exact source revision.
        expected_revision: u64,
        /// Player intent.
        action: ClientAction,
    },
    /// Request canonical state.
    Sync {
        /// Seat secret.
        token: ReconnectToken,
        /// Last game revision already held by the client.
        known_state_revision: u64,
        /// Last lobby revision already held by the client.
        known_lobby_revision: u64,
    },
}

/// Result produced by the non-blocking client worker.
#[derive(Debug, Clone)]
pub enum LanWorkerEvent {
    /// Valid protocol response.
    Message {
        /// Response body.
        message: HostMessage,
        /// Socket request/response duration.
        round_trip: Duration,
    },
    /// Connection or protocol failure.
    Error(String),
}

/// Long-lived network worker that keeps socket I/O off the UI thread.
pub struct LanClientWorker {
    sender: Sender<LanWorkerRequest>,
    receiver: Receiver<LanWorkerEvent>,
}

impl LanClientWorker {
    /// Starts the worker.
    #[must_use]
    pub fn new() -> Self {
        let (request_sender, request_receiver) = flume::bounded(16);
        let (event_sender, event_receiver) = flume::bounded(16);
        let _ = std::thread::Builder::new()
            .name("ludo-lan-worker".to_owned())
            .spawn(move || {
                let mut client = None;
                while let Ok(request) = request_receiver.recv() {
                    let started = Instant::now();
                    let result = match request {
                        LanWorkerRequest::RequestJoin {
                            address,
                            request,
                            room,
                            name,
                        } => connect_and_request(
                            &mut client,
                            address,
                            &ClientMessage::RequestJoin {
                                protocol: PROTOCOL_VERSION,
                                request,
                                room,
                                name,
                            },
                        ),
                        LanWorkerRequest::JoinStatus { address, request } => {
                            request_reusing_connection(
                                &mut client,
                                address,
                                &ClientMessage::JoinStatus {
                                    protocol: PROTOCOL_VERSION,
                                    request,
                                },
                            )
                        }
                        LanWorkerRequest::Reconnect {
                            address,
                            room,
                            token,
                        } => connect_and_request(
                            &mut client,
                            address,
                            &ClientMessage::Reconnect {
                                protocol: PROTOCOL_VERSION,
                                room,
                                token,
                            },
                        ),
                        LanWorkerRequest::Action {
                            token,
                            expected_revision,
                            action,
                        } => request_existing(
                            &mut client,
                            &ClientMessage::Command {
                                protocol: PROTOCOL_VERSION,
                                token,
                                expected_revision,
                                action,
                            },
                        ),
                        LanWorkerRequest::Sync {
                            token,
                            known_state_revision,
                            known_lobby_revision,
                        } => request_existing(
                            &mut client,
                            &ClientMessage::Sync {
                                protocol: PROTOCOL_VERSION,
                                token,
                                known_state_revision,
                                known_lobby_revision,
                            },
                        ),
                    };
                    if result.is_err() {
                        client = None;
                    }
                    let event = result.map_or_else(
                        |error| LanWorkerEvent::Error(error.to_string()),
                        |message| LanWorkerEvent::Message {
                            message,
                            round_trip: started.elapsed(),
                        },
                    );
                    let _ = event_sender.send(event);
                }
            });
        Self {
            sender: request_sender,
            receiver: event_receiver,
        }
    }

    /// Queues network work without blocking.
    ///
    /// # Errors
    ///
    /// Returns the request when the queue is full or closed.
    pub fn request(&self, request: LanWorkerRequest) -> Result<(), Box<LanWorkerRequest>> {
        self.sender
            .try_send(request)
            .map_err(|error| Box::new(error.into_inner()))
    }

    /// Returns the next worker event when ready.
    #[must_use]
    pub fn try_event(&self) -> Option<LanWorkerEvent> {
        self.receiver.try_recv().ok()
    }
}

impl Default for LanClientWorker {
    fn default() -> Self {
        Self::new()
    }
}

fn connect_and_request(
    client: &mut Option<LanClient>,
    address: SocketAddr,
    message: &ClientMessage,
) -> Result<HostMessage, NetworkError> {
    *client = Some(LanClient::connect(address)?);
    request_existing(client, message)
}

fn request_reusing_connection(
    client: &mut Option<LanClient>,
    address: SocketAddr,
    message: &ClientMessage,
) -> Result<HostMessage, NetworkError> {
    if client.is_none() {
        return connect_and_request(client, address, message);
    }
    match request_existing(client, message) {
        Ok(response) => Ok(response),
        Err(_) => connect_and_request(client, address, message),
    }
}

fn request_existing(
    client: &mut Option<LanClient>,
    message: &ClientMessage,
) -> Result<HostMessage, NetworkError> {
    client
        .as_mut()
        .ok_or(NetworkError::Disconnected)?
        .request(message)
}

/// One automatically discovered joinable game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearbyGame {
    /// Stable DNS-SD service identity.
    pub id: String,
    /// Host-selected room title.
    pub name: String,
    /// Address resolved without manual IP entry.
    pub address: SocketAddr,
    /// Human seats currently advertised by the host.
    pub humans: usize,
    /// Total seats in the match.
    pub capacity: usize,
    /// Rule-preset label.
    pub preset: String,
}

/// Incremental event from the LAN discovery daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// A room was resolved or refreshed.
    Upsert(NearbyGame),
    /// A previously visible room disappeared.
    Removed(String),
}

/// Background mDNS/DNS-SD browser and host advertiser.
pub struct LanDiscovery {
    daemon: ServiceDaemon,
    receiver: Receiver<ServiceEvent>,
    registered: Option<String>,
    instance_suffix: u32,
}

impl LanDiscovery {
    /// Starts browsing for Ludo rooms on all active interfaces.
    ///
    /// # Errors
    ///
    /// Returns an mDNS initialization error.
    pub fn new() -> Result<Self, NetworkError> {
        let daemon = ServiceDaemon::new()?;
        let receiver = daemon.browse(LAN_SERVICE_TYPE)?;
        Ok(Self {
            daemon,
            receiver,
            registered: None,
            instance_suffix: rand::rng().random(),
        })
    }

    /// Advertises a private room without exposing its room code.
    ///
    /// # Errors
    ///
    /// Returns an mDNS registration error.
    pub fn advertise(
        &mut self,
        room_name: &str,
        port: u16,
        humans: usize,
        capacity: usize,
        preset: &str,
    ) -> Result<(), NetworkError> {
        self.stop_advertising();
        let suffix = self.instance_suffix;
        let instance = format!("{} #{suffix:08X}", normalize_player_name(room_name));
        let hostname = format!("ludo-{suffix:08x}.local.");
        let properties = HashMap::from([
            ("name".to_owned(), normalize_player_name(room_name)),
            ("protocol".to_owned(), PROTOCOL_VERSION.to_string()),
            ("humans".to_owned(), humans.to_string()),
            ("capacity".to_owned(), capacity.to_string()),
            ("preset".to_owned(), preset.to_owned()),
            ("joinable".to_owned(), "true".to_owned()),
        ]);
        let service =
            ServiceInfo::new(LAN_SERVICE_TYPE, &instance, &hostname, "", port, properties)?
                .enable_addr_auto();
        self.registered = Some(service.get_fullname().to_owned());
        self.daemon.register(service)?;
        Ok(())
    }

    /// Removes the current host advertisement while keeping discovery active.
    pub fn stop_advertising(&mut self) {
        if let Some(fullname) = self.registered.take() {
            let _ = self.daemon.unregister(&fullname);
        }
    }

    /// Returns the next already-available discovery event.
    #[must_use]
    pub fn try_event(&self) -> Option<DiscoveryEvent> {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    if info
                        .get_property_val_str("protocol")
                        .and_then(|version| version.parse::<u16>().ok())
                        != Some(PROTOCOL_VERSION)
                        || info.get_property_val_str("joinable") != Some("true")
                    {
                        continue;
                    }
                    let address = info
                        .get_addresses()
                        .iter()
                        .find(|address| address.is_ipv4() && !address.is_loopback())
                        .or_else(|| {
                            info.get_addresses()
                                .iter()
                                .find(|address| address.is_ipv4())
                        })
                        .or_else(|| info.get_addresses().iter().next())
                        .map(|address| SocketAddr::new(address.to_ip_addr(), info.get_port()));
                    let Some(address) = address else {
                        continue;
                    };
                    let capacity = txt_usize(&info, "capacity").unwrap_or(4).clamp(2, 4);
                    let humans = txt_usize(&info, "humans").unwrap_or(1).min(capacity);
                    let name = normalize_player_name(
                        info.get_property_val_str("name")
                            .unwrap_or("Nearby Ludo game"),
                    );
                    let preset = normalize_player_name(
                        info.get_property_val_str("preset").unwrap_or("Classic"),
                    );
                    return Some(DiscoveryEvent::Upsert(NearbyGame {
                        id: info.get_fullname().to_owned(),
                        name: if name.is_empty() {
                            "Nearby Ludo game".to_owned()
                        } else {
                            name
                        },
                        address,
                        humans,
                        capacity,
                        preset: if preset.is_empty() {
                            "Classic".to_owned()
                        } else {
                            preset
                        },
                    }));
                }
                ServiceEvent::ServiceRemoved(_, fullname) => {
                    return Some(DiscoveryEvent::Removed(fullname));
                }
                _ => {}
            }
        }
        None
    }
}

impl Drop for LanDiscovery {
    fn drop(&mut self) {
        self.stop_advertising();
        let _ = self.daemon.stop_browse(LAN_SERVICE_TYPE);
        let _ = self.daemon.shutdown();
    }
}

fn txt_usize(info: &mdns_sd::ResolvedService, key: &str) -> Option<usize> {
    info.get_property_val_str(key)?.parse().ok()
}

/// LAN transport failures.
#[derive(Debug, Error)]
pub enum NetworkError {
    /// Socket or stream I/O.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// JSON protocol encoding.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Local discovery daemon failure.
    #[error(transparent)]
    Mdns(#[from] mdns_sd::Error),
    /// Shared host room became unavailable.
    #[error("authoritative room is unavailable")]
    RoomUnavailable,
    /// Host decision targeted a stale or invalid request.
    #[error("{0}")]
    JoinRequest(String),
    /// Peer closed the connection.
    #[error("host disconnected")]
    Disconnected,
    /// Peer spoke an incompatible wire schema.
    #[error("protocol mismatch: expected version {expected}, received {received}")]
    ProtocolMismatch {
        /// Client-supported protocol.
        expected: u16,
        /// Host-reported protocol.
        received: u16,
    },
}

#[cfg(test)]
mod tests {
    use ludo_domain::{GameState, Rules, standard_players};
    use proptest::prelude::*;

    use super::*;

    fn approved_join(room: &mut AuthoritativeRoom, name: &str) -> HostMessage {
        let response = room.handle(ClientMessage::RequestJoin {
            protocol: PROTOCOL_VERSION,
            request: JoinRequestId::generate(),
            room: Some(room.code().as_str().to_owned()),
            name: name.to_owned(),
        });
        let HostMessage::JoinPending { request, .. } = response else {
            return response;
        };
        if room.accept_join_request(&request.id).is_err() {
            return AuthoritativeRoom::reject_public(
                "test host could not approve the join request",
            );
        }
        room.handle(ClientMessage::JoinStatus {
            protocol: PROTOCOL_VERSION,
            request: request.id,
        })
    }

    #[test]
    fn room_supports_join_reconnect_and_stale_revision_rejection() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let code = room.code().as_str().to_owned();
        let welcome = approved_join(&mut room, "Sara");
        let HostMessage::Welcome { token, .. } = welcome else {
            return;
        };
        assert!(matches!(
            room.handle(ClientMessage::Reconnect {
                protocol: PROTOCOL_VERSION,
                room: Some(code),
                token: token.clone(),
            }),
            HostMessage::Welcome { .. }
        ));
        room.start_match();
        assert!(matches!(
            room.handle(ClientMessage::Command {
                protocol: PROTOCOL_VERSION,
                token,
                expected_revision: 99,
                action: ClientAction::Roll,
            }),
            HostMessage::Rejected { .. }
        ));
    }

    #[test]
    fn lobby_starts_with_computers_and_named_join_replaces_one() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let initial = room.lobby();
        assert_eq!(initial.human_count(), 1);
        assert_eq!(
            initial
                .seats
                .iter()
                .filter(|seat| matches!(seat.kind, LobbySeatKind::Computer))
                .count(),
            3
        );

        let response = approved_join(&mut room, "  Sara  ");
        let HostMessage::Welcome {
            player,
            state,
            lobby,
            ..
        } = response
        else {
            std::process::abort();
        };
        assert_eq!(
            player,
            PlayerId::new(1).unwrap_or_else(|| std::process::abort())
        );
        assert_eq!(state.players()[1].player.name, "Sara");
        assert!(matches!(
            state.players()[1].player.controller,
            Controller::Human
        ));
        assert_eq!(lobby.human_count(), 2);
    }

    #[test]
    fn discovered_player_waits_for_host_approval_without_a_room_code() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let pending = room.handle(ClientMessage::RequestJoin {
            protocol: PROTOCOL_VERSION,
            request: JoinRequestId::generate(),
            room: None,
            name: "Nearby Sara".to_owned(),
        });
        let HostMessage::JoinPending { request, lobby, .. } = pending else {
            std::process::abort();
        };
        assert_eq!(request.name, "Nearby Sara");
        assert_eq!(lobby.human_count(), 1);
        assert_eq!(room.pending_join_requests(), vec![request.clone()]);

        let accepted = room
            .accept_join_request(&request.id)
            .unwrap_or_else(|_| std::process::abort());
        assert_eq!(accepted.human_count(), 2);
        assert!(matches!(
            room.handle(ClientMessage::JoinStatus {
                protocol: PROTOCOL_VERSION,
                request: request.id,
            }),
            HostMessage::Welcome { lobby, .. } if lobby.human_count() == 2
        ));
    }

    #[test]
    fn declined_player_receives_the_host_decision() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let pending = room.handle(ClientMessage::RequestJoin {
            protocol: PROTOCOL_VERSION,
            request: JoinRequestId::generate(),
            room: None,
            name: "Nearby Sara".to_owned(),
        });
        let HostMessage::JoinPending { request, .. } = pending else {
            std::process::abort();
        };
        let lobby = room
            .reject_join_request(&request.id)
            .unwrap_or_else(|_| std::process::abort());
        assert_eq!(lobby.human_count(), 1);
        assert!(matches!(
            room.handle(ClientMessage::JoinStatus {
                protocol: PROTOCOL_VERSION,
                request: request.id,
            }),
            HostMessage::Rejected { reason, .. } if reason == "The host declined your request."
        ));
    }

    #[test]
    fn fallback_join_still_requires_the_correct_room_code() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        assert!(matches!(
            room.handle(ClientMessage::RequestJoin {
                protocol: PROTOCOL_VERSION,
                request: JoinRequestId::generate(),
                room: Some("WRONG1".to_owned()),
                name: "Sara".to_owned(),
            }),
            HostMessage::Rejected { reason, .. } if reason == "room code does not match"
        ));
        assert!(room.pending_join_requests().is_empty());
    }

    #[test]
    fn started_lobby_rejects_late_joiners() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let code = room.code().as_str().to_owned();
        room.start_match();
        assert!(matches!(
            room.handle(ClientMessage::RequestJoin {
                protocol: PROTOCOL_VERSION,
                request: JoinRequestId::generate(),
                room: Some(code),
                name: "Late player".to_owned(),
            }),
            HostMessage::Rejected { .. }
        ));
    }

    #[test]
    fn disconnected_guest_becomes_a_computer_when_host_starts() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let response = approved_join(&mut room, "Sara");
        let HostMessage::Welcome { token, player, .. } = response else {
            std::process::abort();
        };
        let generation = room
            .attach_transport(&token)
            .unwrap_or_else(|| std::process::abort());
        room.disconnect(&token, generation);
        room.start_match();
        let seat = &room.lobby().seats[player.index()];
        assert!(matches!(seat.kind, LobbySeatKind::Computer));
        assert!(matches!(
            room.state().players()[player.index()].player.controller,
            Controller::Bot
        ));
    }

    #[test]
    fn host_credentials_can_submit_actions_for_computer_turns() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let (_, host_token) = room.host_credentials();
        let join = approved_join(&mut room, "Second human");
        assert!(matches!(join, HostMessage::Welcome { .. }));
        room.start_match();
        let one = DiceValue::new(1).unwrap_or_else(|| std::process::abort());
        assert!(room.state.apply(GameCommand::Roll(one)).is_ok());
        assert!(room.state.apply(GameCommand::Roll(one)).is_ok());
        assert!(matches!(
            room.state.current().player.controller,
            Controller::Bot
        ));

        let revision = room.state.revision();
        assert!(matches!(
            room.handle(ClientMessage::Command {
                protocol: PROTOCOL_VERSION,
                token: host_token,
                expected_revision: revision,
                action: ClientAction::Roll,
            }),
            HostMessage::Applied { .. }
        ));
    }

    #[test]
    fn authenticated_commands_are_blocked_until_the_host_starts() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let (_, token) = room.host_credentials();
        let revision = room.state().revision();
        assert!(matches!(
            room.handle(ClientMessage::Command {
                protocol: PROTOCOL_VERSION,
                token,
                expected_revision: revision,
                action: ClientAction::Roll,
            }),
            HostMessage::Rejected {
                reason,
                state: Some(_),
                lobby: Some(_),
                ..
            } if reason == "the match has not started"
        ));
        assert_eq!(room.state().revision(), revision);
    }

    #[test]
    fn unauthenticated_rejections_do_not_leak_game_or_lobby_snapshots() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let responses = [
            room.handle(ClientMessage::RequestJoin {
                protocol: PROTOCOL_VERSION,
                request: JoinRequestId::generate(),
                room: Some("WRONG1".to_owned()),
                name: "Sara".to_owned(),
            }),
            room.handle(ClientMessage::Reconnect {
                protocol: PROTOCOL_VERSION,
                room: None,
                token: ReconnectToken("unknown".to_owned()),
            }),
            room.handle(ClientMessage::Sync {
                protocol: PROTOCOL_VERSION,
                token: ReconnectToken("unknown".to_owned()),
                known_state_revision: 0,
                known_lobby_revision: 0,
            }),
            room.handle(ClientMessage::JoinStatus {
                protocol: PROTOCOL_VERSION,
                request: JoinRequestId::generate(),
            }),
        ];
        assert!(responses.into_iter().all(|response| matches!(
            response,
            HostMessage::Rejected {
                state: None,
                lobby: None,
                ..
            }
        )));
    }

    #[test]
    fn unchanged_sync_uses_the_small_heartbeat_response() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let (_, token) = room.host_credentials();
        let welcome = room.handle(ClientMessage::Reconnect {
            protocol: PROTOCOL_VERSION,
            room: None,
            token: token.clone(),
        });
        let HostMessage::Welcome { state, lobby, .. } = welcome else {
            std::process::abort();
        };
        assert!(matches!(
            room.handle(ClientMessage::Sync {
                protocol: PROTOCOL_VERSION,
                token,
                known_state_revision: state.revision(),
                known_lobby_revision: lobby.revision,
            }),
            HostMessage::UpToDate {
                state_revision,
                lobby_revision,
                ..
            } if state_revision == state.revision() && lobby_revision == lobby.revision
        ));
    }

    #[test]
    fn changed_lobby_forces_a_snapshot_even_when_game_revision_matches() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let (_, token) = room.host_credentials();
        let welcome = room.handle(ClientMessage::Reconnect {
            protocol: PROTOCOL_VERSION,
            room: None,
            token: token.clone(),
        });
        let HostMessage::Welcome { state, lobby, .. } = welcome else {
            std::process::abort();
        };
        let pending = room.handle(ClientMessage::RequestJoin {
            protocol: PROTOCOL_VERSION,
            request: JoinRequestId::generate(),
            room: None,
            name: "Sara".to_owned(),
        });
        let HostMessage::JoinPending { request, .. } = pending else {
            std::process::abort();
        };
        room.accept_join_request(&request.id)
            .unwrap_or_else(|_| std::process::abort());
        assert!(matches!(
            room.handle(ClientMessage::Sync {
                protocol: PROTOCOL_VERSION,
                token,
                known_state_revision: state.revision(),
                known_lobby_revision: lobby.revision,
            }),
            HostMessage::Snapshot {
                state: current,
                lobby: current_lobby,
                ..
            } if current.revision() == state.revision()
                && current_lobby.revision > lobby.revision
        ));
    }

    #[test]
    fn stale_transport_disconnect_cannot_override_a_new_connection() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let (_, token) = room.host_credentials();
        let old_generation = room
            .attach_transport(&token)
            .unwrap_or_else(|| std::process::abort());
        let new_generation = room
            .attach_transport(&token)
            .unwrap_or_else(|| std::process::abort());
        room.disconnect(&token, old_generation);
        assert!(room.lobby().seats[0].connected);
        room.disconnect(&token, new_generation);
        assert!(!room.lobby().seats[0].connected);
    }

    #[test]
    fn approved_but_unclaimed_expired_seat_cannot_receive_stale_credentials() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let pending = room.handle(ClientMessage::RequestJoin {
            protocol: PROTOCOL_VERSION,
            request: JoinRequestId::generate(),
            room: None,
            name: "Sara".to_owned(),
        });
        let HostMessage::JoinPending { request, .. } = pending else {
            std::process::abort();
        };
        room.accept_join_request(&request.id)
            .unwrap_or_else(|_| std::process::abort());
        let remote = room
            .seats
            .iter_mut()
            .find(|seat| matches!(seat.kind, LobbySeatKind::RemoteHuman))
            .unwrap_or_else(|| std::process::abort());
        remote.disconnected_at = Instant::now().checked_sub(RECONNECT_GRACE);
        assert!(matches!(
            room.handle(ClientMessage::JoinStatus {
                protocol: PROTOCOL_VERSION,
                request: request.id,
            }),
            HostMessage::Rejected { reason, .. }
                if reason == "the approved seat expired; request access again"
        ));
        assert!(matches!(
            room.state().players()[1].player.controller,
            Controller::Bot
        ));
    }

    #[test]
    fn pending_and_resolved_join_records_expire_and_are_removed() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let request_id = JoinRequestId::generate();
        let response = room.handle(ClientMessage::RequestJoin {
            protocol: PROTOCOL_VERSION,
            request: request_id.clone(),
            room: None,
            name: "Sara".to_owned(),
        });
        assert!(matches!(response, HostMessage::JoinPending { .. }));
        room.pending_joins[0].submitted_at = Instant::now()
            .checked_sub(JOIN_REQUEST_TTL)
            .unwrap_or_else(|| std::process::abort());
        assert!(room.pending_join_requests().is_empty());
        let resolution = room
            .join_resolutions
            .get_mut(&request_id)
            .unwrap_or_else(|| std::process::abort());
        resolution.resolved_at = Instant::now()
            .checked_sub(JOIN_RESOLUTION_TTL)
            .unwrap_or_else(|| std::process::abort());
        assert!(matches!(
            room.handle(ClientMessage::JoinStatus {
                protocol: PROTOCOL_VERSION,
                request: request_id,
            }),
            HostMessage::Rejected { reason, .. }
                if reason == "join request expired or is unknown"
        ));
        assert!(room.join_resolutions.is_empty());
    }

    #[test]
    fn duplicate_request_id_is_idempotent_before_and_after_approval() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let request_id = JoinRequestId::generate();
        let request = || ClientMessage::RequestJoin {
            protocol: PROTOCOL_VERSION,
            request: request_id.clone(),
            room: None,
            name: "Sara".to_owned(),
        };
        let first = room.handle(request());
        let second = room.handle(request());
        assert_eq!(first, second);
        assert_eq!(room.pending_joins.len(), 1);
        room.accept_join_request(&request_id)
            .unwrap_or_else(|_| std::process::abort());
        let first_welcome = room.handle(request());
        let retried_welcome = room.handle(request());
        assert_eq!(first_welcome, retried_welcome);
        assert!(matches!(
            first_welcome,
            HostMessage::Welcome { player, .. } if player.index() == 1
        ));
        assert_eq!(room.seats.len(), 2);
    }

    #[test]
    fn names_are_trimmed_bounded_and_compared_case_insensitively() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let response = room.handle(ClientMessage::RequestJoin {
            protocol: PROTOCOL_VERSION,
            request: JoinRequestId::generate(),
            room: None,
            name: "  Alice\u{0}\nABCDEFGHIJKLMNOPQRSTUVWXYZ  ".to_owned(),
        });
        let HostMessage::JoinPending { request, .. } = response else {
            std::process::abort();
        };
        assert_eq!(request.name.chars().count(), MAX_PLAYER_NAME_CHARS);
        assert!(!request.name.chars().any(char::is_control));
        assert!(matches!(
            room.handle(ClientMessage::RequestJoin {
                protocol: PROTOCOL_VERSION,
                request: JoinRequestId::generate(),
                room: None,
                name: request.name.to_lowercase(),
            }),
            HostMessage::Rejected { reason, .. } if reason == "that player name is already in use"
        ));
    }

    #[test]
    fn pending_join_queue_is_bounded_without_mutating_game_state() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let initial_revision = room.state().revision();
        for index in 0..MAX_PENDING_JOIN_REQUESTS {
            assert!(matches!(
                room.handle(ClientMessage::RequestJoin {
                    protocol: PROTOCOL_VERSION,
                    request: JoinRequestId::generate(),
                    room: None,
                    name: format!("Guest {index}"),
                }),
                HostMessage::JoinPending { .. }
            ));
        }
        assert!(matches!(
            room.handle(ClientMessage::RequestJoin {
                protocol: PROTOCOL_VERSION,
                request: JoinRequestId::generate(),
                room: None,
                name: "One too many".to_owned(),
            }),
            HostMessage::Rejected { reason, .. }
                if reason == "too many players are already waiting for approval"
        ));
        assert_eq!(room.pending_joins.len(), MAX_PENDING_JOIN_REQUESTS);
        assert_eq!(room.state().revision(), initial_revision);
    }

    #[test]
    fn starting_match_rejects_every_pending_request_and_is_idempotent() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut room = AuthoritativeRoom::new(state);
        let request_id = JoinRequestId::generate();
        let pending = room.handle(ClientMessage::RequestJoin {
            protocol: PROTOCOL_VERSION,
            request: request_id.clone(),
            room: None,
            name: "Sara".to_owned(),
        });
        assert!(matches!(pending, HostMessage::JoinPending { .. }));
        room.start_match();
        let lobby_revision = room.lobby().revision;
        room.start_match();
        assert_eq!(room.lobby().revision, lobby_revision);
        assert!(matches!(
            room.handle(ClientMessage::JoinStatus {
                protocol: PROTOCOL_VERSION,
                request: request_id,
            }),
            HostMessage::Rejected { reason, .. } if reason == "The host started the match."
        ));
    }

    #[test]
    fn room_code_and_request_ids_have_fixed_safe_shapes() {
        for _ in 0..1_000 {
            let code = RoomCode::generate();
            assert_eq!(code.as_str().len(), 6);
            assert!(
                code.as_str()
                    .bytes()
                    .all(|character| { b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789".contains(&character) })
            );
            let request = JoinRequestId::generate();
            assert_eq!(request.as_str().len(), 32);
            assert!(
                request
                    .as_str()
                    .bytes()
                    .all(|character| character.is_ascii_hexdigit())
            );
        }
    }

    #[test]
    fn tcp_host_and_client_exchange_versioned_messages() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let address = "127.0.0.1:0"
            .parse()
            .unwrap_or_else(|_| std::process::abort());
        let host = LanHost::bind(address, state).unwrap_or_else(|_| std::process::abort());
        let code = host.room_code().unwrap_or_else(|_| std::process::abort());
        let mut client =
            LanClient::connect(host.address()).unwrap_or_else(|_| std::process::abort());
        let pending = client
            .request(&ClientMessage::RequestJoin {
                protocol: PROTOCOL_VERSION,
                request: JoinRequestId::generate(),
                room: Some(code.as_str().to_owned()),
                name: "Sara".to_owned(),
            })
            .unwrap_or_else(|_| std::process::abort());
        let HostMessage::JoinPending { request, .. } = pending else {
            std::process::abort();
        };
        host.accept_join_request(&request.id)
            .unwrap_or_else(|_| std::process::abort());
        let response = client
            .request(&ClientMessage::JoinStatus {
                protocol: PROTOCOL_VERSION,
                request: request.id,
            })
            .unwrap_or_else(|_| std::process::abort());
        assert!(matches!(
            response,
            HostMessage::Welcome {
                protocol: PROTOCOL_VERSION,
                ..
            }
        ));
    }

    #[test]
    fn two_tcp_clients_keep_connections_across_sync_and_commands() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let address = "127.0.0.1:0"
            .parse()
            .unwrap_or_else(|_| std::process::abort());
        let host = LanHost::bind(address, state).unwrap_or_else(|_| std::process::abort());
        let code = host.room_code().unwrap_or_else(|_| std::process::abort());
        let mut first =
            LanClient::connect(host.address()).unwrap_or_else(|_| std::process::abort());
        let mut second =
            LanClient::connect(host.address()).unwrap_or_else(|_| std::process::abort());
        let (_, host_token) = host
            .host_credentials()
            .unwrap_or_else(|_| std::process::abort());
        let first_welcome = first
            .request(&ClientMessage::Reconnect {
                protocol: PROTOCOL_VERSION,
                room: Some(code.as_str().to_owned()),
                token: host_token,
            })
            .unwrap_or_else(|_| std::process::abort());
        let pending = second
            .request(&ClientMessage::RequestJoin {
                protocol: PROTOCOL_VERSION,
                request: JoinRequestId::generate(),
                room: None,
                name: "Sara".to_owned(),
            })
            .unwrap_or_else(|_| std::process::abort());
        let HostMessage::JoinPending { request, .. } = pending else {
            return;
        };
        host.accept_join_request(&request.id)
            .unwrap_or_else(|_| std::process::abort());
        let second_welcome = second
            .request(&ClientMessage::JoinStatus {
                protocol: PROTOCOL_VERSION,
                request: request.id,
            })
            .unwrap_or_else(|_| std::process::abort());
        let HostMessage::Welcome {
            token: first_token,
            state,
            ..
        } = first_welcome
        else {
            return;
        };
        let HostMessage::Welcome {
            token: second_token,
            ..
        } = second_welcome
        else {
            return;
        };
        host.start_match().unwrap_or_else(|_| std::process::abort());

        for _ in 0..20 {
            assert!(matches!(
                second.request(&ClientMessage::Sync {
                    protocol: PROTOCOL_VERSION,
                    token: second_token.clone(),
                    known_state_revision: u64::MAX,
                    known_lobby_revision: u64::MAX,
                }),
                Ok(HostMessage::Snapshot { .. })
            ));
        }
        assert!(matches!(
            first.request(&ClientMessage::Command {
                protocol: PROTOCOL_VERSION,
                token: first_token,
                expected_revision: state.revision(),
                action: ClientAction::Roll,
            }),
            Ok(HostMessage::Applied { .. })
        ));
        assert!(matches!(
            second.request(&ClientMessage::Sync {
                protocol: PROTOCOL_VERSION,
                token: second_token,
                known_state_revision: u64::MAX,
                known_lobby_revision: u64::MAX,
            }),
            Ok(HostMessage::Snapshot { state, .. }) if state.revision() == 1
        ));
    }

    #[test]
    fn newest_tcp_connection_remains_online_when_an_old_one_closes() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let address = "127.0.0.1:0"
            .parse()
            .unwrap_or_else(|_| std::process::abort());
        let host = LanHost::bind(address, state).unwrap_or_else(|_| std::process::abort());
        let (_, token) = host
            .host_credentials()
            .unwrap_or_else(|_| std::process::abort());
        let mut old = LanClient::connect(host.address()).unwrap_or_else(|_| std::process::abort());
        let mut newest =
            LanClient::connect(host.address()).unwrap_or_else(|_| std::process::abort());
        assert!(matches!(
            old.request(&ClientMessage::Reconnect {
                protocol: PROTOCOL_VERSION,
                room: None,
                token: token.clone(),
            }),
            Ok(HostMessage::Welcome { .. })
        ));
        assert!(matches!(
            newest.request(&ClientMessage::Reconnect {
                protocol: PROTOCOL_VERSION,
                room: None,
                token,
            }),
            Ok(HostMessage::Welcome { .. })
        ));
        drop(old);
        std::thread::sleep(Duration::from_millis(25));
        let lobby = host.lobby().unwrap_or_else(|_| std::process::abort());
        assert!(lobby.seats[0].connected);
    }

    #[test]
    fn concurrent_same_revision_commands_are_serialized_exactly_once() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let address = "127.0.0.1:0"
            .parse()
            .unwrap_or_else(|_| std::process::abort());
        let host = LanHost::bind(address, state).unwrap_or_else(|_| std::process::abort());
        let (_, token) = host
            .host_credentials()
            .unwrap_or_else(|_| std::process::abort());
        let mut first =
            LanClient::connect(host.address()).unwrap_or_else(|_| std::process::abort());
        let mut second =
            LanClient::connect(host.address()).unwrap_or_else(|_| std::process::abort());
        let first_welcome = first
            .request(&ClientMessage::Reconnect {
                protocol: PROTOCOL_VERSION,
                room: None,
                token: token.clone(),
            })
            .unwrap_or_else(|_| std::process::abort());
        assert!(matches!(
            second.request(&ClientMessage::Reconnect {
                protocol: PROTOCOL_VERSION,
                room: None,
                token: token.clone(),
            }),
            Ok(HostMessage::Welcome { .. })
        ));
        let HostMessage::Welcome { state, .. } = first_welcome else {
            std::process::abort();
        };
        host.start_match().unwrap_or_else(|_| std::process::abort());
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let expected_revision = state.revision();
        let spawn_command = |mut client: LanClient, barrier: Arc<std::sync::Barrier>| {
            let token = token.clone();
            std::thread::spawn(move || {
                barrier.wait();
                client.request(&ClientMessage::Command {
                    protocol: PROTOCOL_VERSION,
                    token,
                    expected_revision,
                    action: ClientAction::Roll,
                })
            })
        };
        let first_thread = spawn_command(first, barrier.clone());
        let second_thread = spawn_command(second, barrier.clone());
        barrier.wait();
        let responses = [
            first_thread
                .join()
                .unwrap_or_else(|_| std::process::abort())
                .unwrap_or_else(|_| std::process::abort()),
            second_thread
                .join()
                .unwrap_or_else(|_| std::process::abort())
                .unwrap_or_else(|_| std::process::abort()),
        ];
        assert_eq!(
            responses
                .iter()
                .filter(|response| matches!(response, HostMessage::Applied { .. }))
                .count(),
            1
        );
        assert_eq!(
            responses
                .iter()
                .filter(|response| matches!(response, HostMessage::Rejected { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn malformed_request_is_rejected_without_poisoning_the_connection() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let address = "127.0.0.1:0"
            .parse()
            .unwrap_or_else(|_| std::process::abort());
        let host = LanHost::bind(address, state).unwrap_or_else(|_| std::process::abort());
        let (_, token) = host
            .host_credentials()
            .unwrap_or_else(|_| std::process::abort());
        let stream = TcpStream::connect(host.address()).unwrap_or_else(|_| std::process::abort());
        stream
            .set_read_timeout(Some(CLIENT_IO_TIMEOUT))
            .unwrap_or_else(|_| std::process::abort());
        let mut reader =
            BufReader::new(stream.try_clone().unwrap_or_else(|_| std::process::abort()));
        let mut writer = BufWriter::new(stream);
        writer
            .write_all(b"not-json\n")
            .unwrap_or_else(|_| std::process::abort());
        writer.flush().unwrap_or_else(|_| std::process::abort());
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .unwrap_or_else(|_| std::process::abort());
        assert!(matches!(
            serde_json::from_str::<HostMessage>(line.trim_end()),
            Ok(HostMessage::Rejected {
                state: None,
                lobby: None,
                ..
            })
        ));

        line.clear();
        serde_json::to_writer(
            &mut writer,
            &ClientMessage::Reconnect {
                protocol: PROTOCOL_VERSION,
                room: None,
                token,
            },
        )
        .unwrap_or_else(|_| std::process::abort());
        writer
            .write_all(b"\n")
            .unwrap_or_else(|_| std::process::abort());
        writer.flush().unwrap_or_else(|_| std::process::abort());
        reader
            .read_line(&mut line)
            .unwrap_or_else(|_| std::process::abort());
        assert!(matches!(
            serde_json::from_str::<HostMessage>(line.trim_end()),
            Ok(HostMessage::Welcome { .. })
        ));
    }

    #[test]
    fn oversized_and_unterminated_requests_are_bounded_and_isolated() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let address = "127.0.0.1:0"
            .parse()
            .unwrap_or_else(|_| std::process::abort());
        let host = LanHost::bind(address, state).unwrap_or_else(|_| std::process::abort());

        let mut oversized =
            TcpStream::connect(host.address()).unwrap_or_else(|_| std::process::abort());
        oversized
            .write_all(&vec![b'x'; MAX_REQUEST_BYTES + 1])
            .unwrap_or_else(|_| std::process::abort());
        oversized
            .write_all(b"\n")
            .unwrap_or_else(|_| std::process::abort());
        let _ = oversized.shutdown(std::net::Shutdown::Write);

        let mut unterminated =
            TcpStream::connect(host.address()).unwrap_or_else(|_| std::process::abort());
        unterminated
            .write_all(b"{\"Sync\":{}")
            .unwrap_or_else(|_| std::process::abort());
        let _ = unterminated.shutdown(std::net::Shutdown::Write);

        let (_, token) = host
            .host_credentials()
            .unwrap_or_else(|_| std::process::abort());
        let mut valid =
            LanClient::connect(host.address()).unwrap_or_else(|_| std::process::abort());
        assert!(matches!(
            valid.request(&ClientMessage::Reconnect {
                protocol: PROTOCOL_VERSION,
                room: None,
                token,
            }),
            Ok(HostMessage::Welcome { .. })
        ));
    }

    #[test]
    fn client_rejects_an_oversized_host_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|_| std::process::abort());
        let address = listener
            .local_addr()
            .unwrap_or_else(|_| std::process::abort());
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap_or_else(|_| std::process::abort());
            let mut reader =
                BufReader::new(stream.try_clone().unwrap_or_else(|_| std::process::abort()));
            let mut request = String::new();
            reader
                .read_line(&mut request)
                .unwrap_or_else(|_| std::process::abort());
            let mut writer = BufWriter::new(stream);
            writer
                .write_all(&vec![b'x'; MAX_RESPONSE_BYTES + 1])
                .unwrap_or_else(|_| std::process::abort());
            writer
                .write_all(b"\n")
                .unwrap_or_else(|_| std::process::abort());
            writer.flush().unwrap_or_else(|_| std::process::abort());
        });
        let mut client = LanClient::connect(address).unwrap_or_else(|_| std::process::abort());
        assert!(matches!(
            client.request(&ClientMessage::JoinStatus {
                protocol: PROTOCOL_VERSION,
                request: JoinRequestId::generate(),
            }),
            Err(NetworkError::Io(error)) if error.kind() == io::ErrorKind::InvalidData
        ));
        server.join().unwrap_or_else(|_| std::process::abort());
    }

    #[test]
    fn client_rejects_a_host_with_an_incompatible_protocol() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|_| std::process::abort());
        let address = listener
            .local_addr()
            .unwrap_or_else(|_| std::process::abort());
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap_or_else(|_| std::process::abort());
            let mut reader =
                BufReader::new(stream.try_clone().unwrap_or_else(|_| std::process::abort()));
            let mut request = String::new();
            reader
                .read_line(&mut request)
                .unwrap_or_else(|_| std::process::abort());
            let mut writer = BufWriter::new(stream);
            serde_json::to_writer(
                &mut writer,
                &HostMessage::UpToDate {
                    protocol: PROTOCOL_VERSION.saturating_sub(1),
                    state_revision: 0,
                    lobby_revision: 0,
                },
            )
            .unwrap_or_else(|_| std::process::abort());
            writer
                .write_all(b"\n")
                .unwrap_or_else(|_| std::process::abort());
            writer.flush().unwrap_or_else(|_| std::process::abort());
        });
        let mut client = LanClient::connect(address).unwrap_or_else(|_| std::process::abort());
        assert!(matches!(
            client.request(&ClientMessage::JoinStatus {
                protocol: PROTOCOL_VERSION,
                request: JoinRequestId::generate(),
            }),
            Err(NetworkError::ProtocolMismatch {
                expected: PROTOCOL_VERSION,
                received,
            }) if received == PROTOCOL_VERSION.saturating_sub(1)
        ));
        server.join().unwrap_or_else(|_| std::process::abort());
    }

    #[test]
    fn dropping_host_with_idle_connections_finishes_promptly() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let address = "127.0.0.1:0"
            .parse()
            .unwrap_or_else(|_| std::process::abort());
        let host = LanHost::bind(address, state).unwrap_or_else(|_| std::process::abort());
        let _idle = TcpStream::connect(host.address()).unwrap_or_else(|_| std::process::abort());
        let started = Instant::now();
        drop(host);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn revision_heartbeat_is_much_smaller_than_a_full_snapshot() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let room = AuthoritativeRoom::new(state.clone());
        let heartbeat = HostMessage::UpToDate {
            protocol: PROTOCOL_VERSION,
            state_revision: state.revision(),
            lobby_revision: room.lobby().revision,
        };
        let snapshot = HostMessage::Snapshot {
            protocol: PROTOCOL_VERSION,
            state,
            lobby: room.lobby(),
        };
        let heartbeat_bytes = serde_json::to_vec(&heartbeat)
            .unwrap_or_else(|_| std::process::abort())
            .len();
        let snapshot_bytes = serde_json::to_vec(&snapshot)
            .unwrap_or_else(|_| std::process::abort())
            .len();
        assert!(heartbeat_bytes * 10 < snapshot_bytes);
    }

    proptest! {
        #[test]
        fn arbitrary_names_normalize_to_a_stable_safe_display_form(name in any::<String>()) {
            let normalized = normalize_player_name(&name);
            prop_assert!(normalized.chars().count() <= MAX_PLAYER_NAME_CHARS);
            prop_assert!(!normalized.chars().any(char::is_control));
            prop_assert_eq!(normalized.trim(), normalized.as_str());
            prop_assert!(!normalized.contains("  "));
            prop_assert_eq!(normalize_player_name(&normalized), normalized);
        }

        #[test]
        fn request_messages_round_trip_through_the_wire_schema(
            name in ".{0,128}",
            room in proptest::option::of("[A-Z2-9]{0,12}"),
        ) {
            let message = ClientMessage::RequestJoin {
                protocol: PROTOCOL_VERSION,
                request: JoinRequestId::generate(),
                room,
                name,
            };
            let bytes = serde_json::to_vec(&message)
                .unwrap_or_else(|_| std::process::abort());
            prop_assert!(bytes.len() <= MAX_REQUEST_BYTES);
            let decoded = serde_json::from_slice::<ClientMessage>(&bytes)
                .unwrap_or_else(|_| std::process::abort());
            prop_assert_eq!(decoded, message);
        }

        #[test]
        fn rejected_or_pending_names_never_advance_the_game_revision(name in any::<String>()) {
            let state = GameState::new(standard_players(), Rules::default())
                .unwrap_or_else(|_| std::process::abort());
            let mut room = AuthoritativeRoom::new(state);
            let revision = room.state().revision();
            let _response = room.handle(ClientMessage::RequestJoin {
                protocol: PROTOCOL_VERSION,
                request: JoinRequestId::generate(),
                room: None,
                name,
            });
            prop_assert_eq!(room.state().revision(), revision);
        }
    }
}
