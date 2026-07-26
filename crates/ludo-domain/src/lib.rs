//! Pure, deterministic, GUI-independent Ludo rules.

pub mod competition;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Current serialized domain schema.
pub const GAME_SCHEMA_VERSION: u16 = 1;
/// Number of tokens owned by each player.
pub const TOKENS_PER_PLAYER: usize = 4;
/// Number of positions on the shared track.
pub const TRACK_LEN: u8 = 52;
/// Exact progress at which a token finishes.
pub const FINISH_PROGRESS: u8 = 57;

const START_CELLS: [u8; 4] = [0, 13, 26, 39];
const STAR_CELLS: [u8; 4] = [8, 21, 34, 47];

/// Stable player identity and array index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlayerId(u8);

impl PlayerId {
    /// Creates an ID valid for a four-player board.
    #[must_use]
    pub const fn new(index: u8) -> Option<Self> {
        if index < 4 { Some(Self(index)) } else { None }
    }

    /// Returns the zero-based player index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Stable token identity within a player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TokenId(u8);

impl TokenId {
    /// Creates an ID valid for a four-token set.
    #[must_use]
    pub const fn new(index: u8) -> Option<Self> {
        if index < 4 { Some(Self(index)) } else { None }
    }

    /// Returns the zero-based token index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The four Ludo colors in clockwise board order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlayerColor {
    /// Top-left.
    Red,
    /// Top-right.
    Green,
    /// Bottom-right.
    Yellow,
    /// Bottom-left.
    Blue,
}

impl PlayerColor {
    /// All colors in clockwise order.
    pub const ALL: [Self; 4] = [Self::Red, Self::Green, Self::Yellow, Self::Blue];

    /// Start index on the shared track.
    #[must_use]
    pub const fn start_index(self) -> u8 {
        match self {
            Self::Red => 0,
            Self::Green => 13,
            Self::Yellow => 26,
            Self::Blue => 39,
        }
    }

    /// User-facing name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Green => "Green",
            Self::Yellow => "Yellow",
            Self::Blue => "Blue",
        }
    }
}

/// Whether a seat is locally controlled or computerized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Controller {
    /// Local human.
    Human,
    /// Computer player.
    Bot,
}

/// Persisted computer-player strength.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BotDifficulty {
    /// Direct tactical choices.
    Easy,
    /// Tactical choices with safety awareness.
    Medium,
    /// Parallel look-ahead evaluation.
    Hard,
}

impl BotDifficulty {
    /// Values in display order.
    pub const ALL: [Self; 3] = [Self::Easy, Self::Medium, Self::Hard];

    /// User-facing name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Easy => "Easy",
            Self::Medium => "Medium",
            Self::Hard => "Hard",
        }
    }
}

/// Immutable player setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Player {
    /// Stable ID.
    pub id: PlayerId,
    /// Display name.
    pub name: String,
    /// Board color.
    pub color: PlayerColor,
    /// Input controller.
    pub controller: Controller,
    /// Strength used when the controller is a bot.
    pub bot_difficulty: BotDifficulty,
}

/// Logical token position, independent of GUI coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenPosition {
    /// Token has not entered the board.
    Yard,
    /// Player-relative progress. Values 0–51 are the shared track and 52–56
    /// are the home lane.
    Path(u8),
    /// Token completed its path.
    Finished,
}

/// Runtime token state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    /// ID within the owner.
    pub id: TokenId,
    /// Logical position.
    pub position: TokenPosition,
}

impl Token {
    const fn new(id: TokenId) -> Self {
        Self {
            id,
            position: TokenPosition::Yard,
        }
    }
}

/// Per-player mutable state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerState {
    /// Player setup.
    pub player: Player,
    /// Four owned tokens.
    pub tokens: [Token; TOKENS_PER_PLAYER],
}

/// Which shared-track cells are safe from capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SafeCellRule {
    /// No safe shared-track cells.
    None,
    /// Only the four colored start cells.
    Starts,
    /// Starts plus the four marked star cells.
    StartsAndStars,
}

/// When a match is considered complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WinCondition {
    /// Stop as soon as the first player finishes.
    FirstWinner,
    /// Continue until all placements can be determined.
    RankAll,
}

/// Named rule configurations presented by clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RulePreset {
    /// Common modern rules with blockades and all safe cells.
    Classic,
    /// Faster game without blockades and without exact-home overshoot blocking.
    Quick,
    /// Full placement ranking with conservative turn bonuses.
    Tournament,
}

impl RulePreset {
    /// Presets in display order.
    pub const ALL: [Self; 3] = [Self::Classic, Self::Quick, Self::Tournament];

    /// User-facing name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Quick => "Quick",
            Self::Tournament => "Tournament",
        }
    }

    /// Materializes the preset.
    #[must_use]
    pub const fn rules(self) -> Rules {
        match self {
            Self::Classic => Rules {
                extra_turn_on_six: true,
                extra_turn_on_capture: true,
                extra_turn_on_home: true,
                three_sixes_forfeit: true,
                blockades: true,
                exact_home_roll: true,
                safe_cells: SafeCellRule::StartsAndStars,
                win_condition: WinCondition::FirstWinner,
            },
            Self::Quick => Rules {
                extra_turn_on_six: true,
                extra_turn_on_capture: true,
                extra_turn_on_home: true,
                three_sixes_forfeit: false,
                blockades: false,
                exact_home_roll: false,
                safe_cells: SafeCellRule::Starts,
                win_condition: WinCondition::FirstWinner,
            },
            Self::Tournament => Rules {
                extra_turn_on_six: true,
                extra_turn_on_capture: true,
                extra_turn_on_home: false,
                three_sixes_forfeit: true,
                blockades: true,
                exact_home_roll: true,
                safe_cells: SafeCellRule::StartsAndStars,
                win_condition: WinCondition::RankAll,
            },
        }
    }
}

/// Customizable rules supported by the engine.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent rule switches are serialized and directly editable by rule configuration clients"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rules {
    /// A six grants another turn.
    pub extra_turn_on_six: bool,
    /// Capturing grants another turn.
    pub extra_turn_on_capture: bool,
    /// Reaching home grants another turn.
    pub extra_turn_on_home: bool,
    /// Three consecutive sixes forfeit the turn.
    pub three_sixes_forfeit: bool,
    /// Two same-color tokens form an impassable opponent blockade.
    pub blockades: bool,
    /// Overshooting home is illegal rather than immediately finishing.
    pub exact_home_roll: bool,
    /// Safe-cell configuration.
    pub safe_cells: SafeCellRule,
    /// Match completion behavior.
    pub win_condition: WinCondition,
}

impl Default for Rules {
    fn default() -> Self {
        RulePreset::Classic.rules()
    }
}

impl Rules {
    /// Checks that enabled switches can have an effect together.
    ///
    /// # Errors
    ///
    /// Returns an error when three-sixes forfeiture is enabled while sixes do
    /// not grant another turn, making consecutive sixes impossible.
    pub const fn validate(self) -> Result<Self, DomainError> {
        if self.three_sixes_forfeit && !self.extra_turn_on_six {
            Err(DomainError::InvalidRules)
        } else {
            Ok(self)
        }
    }
}

/// Current interaction expected by the engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnPhase {
    /// Current player must roll.
    AwaitingRoll,
    /// Current player must choose a legal token.
    AwaitingMove {
        /// Rolled value.
        dice: DiceValue,
        /// Tokens permitted to move.
        legal_tokens: Vec<TokenId>,
    },
}

/// Overall match status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameStatus {
    /// Match accepts commands.
    Playing,
    /// All required placements have been determined.
    Finished,
}

/// A validated six-sided dice value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiceValue(u8);

impl DiceValue {
    /// Creates a value from one through six.
    #[must_use]
    pub const fn new(value: u8) -> Option<Self> {
        if value >= 1 && value <= 6 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Raw value.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Commands accepted by a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameCommand {
    /// Record a roll supplied by an outer-layer dice source.
    Roll(DiceValue),
    /// Move a legal token.
    Move(TokenId),
}

/// Facts produced after applying a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameEvent {
    /// Dice result became visible.
    DiceRolled {
        /// Player who rolled.
        player: PlayerId,
        /// Result.
        value: DiceValue,
    },
    /// A token entered or advanced.
    TokenMoved {
        /// Owner.
        player: PlayerId,
        /// Token.
        token: TokenId,
        /// Previous position.
        from: TokenPosition,
        /// New position.
        to: TokenPosition,
    },
    /// An opponent token returned to its yard.
    TokenCaptured {
        /// Capturing player.
        by: PlayerId,
        /// Captured player.
        player: PlayerId,
        /// Captured token.
        token: TokenId,
    },
    /// A player earned a placement.
    PlayerRanked {
        /// Ranked player.
        player: PlayerId,
        /// One-based place.
        place: u8,
    },
    /// Control moved to another player.
    TurnChanged {
        /// New current player.
        player: PlayerId,
    },
    /// Match ended.
    GameFinished {
        /// Final or currently determined placements.
        rankings: Vec<PlayerId>,
    },
}

/// A complete serializable match snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameState {
    schema_version: u16,
    players: Vec<PlayerState>,
    current_player: usize,
    phase: TurnPhase,
    status: GameStatus,
    revision: u64,
    rules: Rules,
    rankings: Vec<PlayerId>,
    consecutive_sixes: u8,
    #[serde(default)]
    team_ids: Option<[u8; 4]>,
}

/// Domain validation failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DomainError {
    /// Unsupported number of players.
    #[error("a game requires between two and four players")]
    InvalidPlayerCount,
    /// Player identities, names, or colors are inconsistent.
    #[error("players must have unique sequential IDs, non-empty names, and unique colors")]
    InvalidPlayers,
    /// Rule switches contain an incompatible combination.
    #[error("three-sixes forfeiture requires bonus turns on six")]
    InvalidRules,
    /// Team setup does not contain two alternating pairs.
    #[error("team games require four players split into two teams of two")]
    InvalidTeams,
    /// Serialized or restored state violates an invariant.
    #[error("the game snapshot is invalid")]
    InvalidSnapshot,
    /// Match already ended.
    #[error("the game is already over")]
    GameOver,
    /// Command does not match the current phase.
    #[error("command is not valid during the current turn phase")]
    WrongPhase,
    /// Selected token cannot use the current roll.
    #[error("that token cannot move")]
    IllegalMove,
}

impl GameState {
    /// Creates a new match with two to four players.
    ///
    /// # Errors
    ///
    /// Returns an error when the player setup is invalid.
    pub fn new(players: Vec<Player>, rules: Rules) -> Result<Self, DomainError> {
        Self::validate_players(&players)?;
        let rules = rules.validate()?;
        let players = players
            .into_iter()
            .map(|player| PlayerState {
                player,
                tokens: std::array::from_fn(|index| {
                    Token::new(TokenId(u8::try_from(index).unwrap_or_default()))
                }),
            })
            .collect();
        Ok(Self {
            schema_version: GAME_SCHEMA_VERSION,
            players,
            current_player: 0,
            phase: TurnPhase::AwaitingRoll,
            status: GameStatus::Playing,
            revision: 0,
            rules,
            rankings: Vec::new(),
            consecutive_sixes: 0,
            team_ids: None,
        })
    }

    /// Creates a two-versus-two match. Players with the same team ID are
    /// allies and cannot capture or blockade one another.
    ///
    /// # Errors
    ///
    /// Returns an error unless exactly four players are split two-and-two
    /// across team IDs zero and one.
    pub fn new_team(
        players: Vec<Player>,
        rules: Rules,
        team_ids: [u8; 4],
    ) -> Result<Self, DomainError> {
        if players.len() != 4 || !valid_team_ids(team_ids) {
            return Err(DomainError::InvalidTeams);
        }
        let mut state = Self::new(players, rules)?;
        state.team_ids = Some(team_ids);
        Ok(state)
    }

    /// Validates a deserialized snapshot before it enters the application.
    ///
    /// # Errors
    ///
    /// Returns an error when any persisted invariant is invalid or incompatible.
    pub fn validated(self) -> Result<Self, DomainError> {
        if self.schema_version != GAME_SCHEMA_VERSION
            || self.current_player >= self.players.len()
            || self.consecutive_sixes > 2
            || self.rules.validate().is_err()
            || self
                .team_ids
                .is_some_and(|teams| self.players.len() != 4 || !valid_team_ids(teams))
        {
            return Err(DomainError::InvalidSnapshot);
        }
        Self::validate_players(
            &self
                .players
                .iter()
                .map(|state| state.player.clone())
                .collect::<Vec<_>>(),
        )
        .map_err(|_| DomainError::InvalidSnapshot)?;
        for state in &self.players {
            for (index, token) in state.tokens.iter().enumerate() {
                if token.id.index() != index
                    || matches!(token.position, TokenPosition::Path(progress) if progress >= FINISH_PROGRESS)
                {
                    return Err(DomainError::InvalidSnapshot);
                }
            }
        }
        if self
            .rankings
            .iter()
            .any(|player| player.index() >= self.players.len())
            || has_duplicates(&self.rankings)
            || (matches!(self.status, GameStatus::Finished) && self.rankings.is_empty())
        {
            return Err(DomainError::InvalidSnapshot);
        }
        if self.rankings.iter().enumerate().any(|(index, player)| {
            let is_determined_last_place = matches!(
                (self.status, self.rules.win_condition),
                (GameStatus::Finished, WinCondition::RankAll)
            ) && index + 1 == self.rankings.len();
            !is_determined_last_place
                && !self.players[player.index()]
                    .tokens
                    .iter()
                    .all(|token| matches!(token.position, TokenPosition::Finished))
        }) || (matches!(self.status, GameStatus::Playing)
            && self.rankings.contains(&self.current().player.id))
            || (matches!(self.status, GameStatus::Finished)
                && !matches!(self.phase, TurnPhase::AwaitingRoll))
            || (matches!(self.status, GameStatus::Playing)
                && self.player_completed(self.current().player.id))
            || (matches!(
                (self.status, self.rules.win_condition),
                (GameStatus::Playing, WinCondition::FirstWinner)
            ) && !self.rankings.is_empty())
        {
            return Err(DomainError::InvalidSnapshot);
        }
        match (self.status, self.rules.win_condition) {
            (GameStatus::Finished, WinCondition::FirstWinner)
                if self.team_ids.is_none() && self.rankings.len() != 1 =>
            {
                return Err(DomainError::InvalidSnapshot);
            }
            (GameStatus::Finished, WinCondition::FirstWinner)
                if self.team_ids.is_some()
                    && (self.rankings.len() != 2
                        || self.team_id(self.rankings[0]) != self.team_id(self.rankings[1])) =>
            {
                return Err(DomainError::InvalidSnapshot);
            }
            (GameStatus::Finished, WinCondition::RankAll)
                if self.rankings.len() != self.players.len() =>
            {
                return Err(DomainError::InvalidSnapshot);
            }
            _ => {}
        }
        if let TurnPhase::AwaitingMove { dice, legal_tokens } = &self.phase
            && *legal_tokens != self.legal_tokens(*dice)
        {
            return Err(DomainError::InvalidSnapshot);
        }
        Ok(self)
    }

    /// Schema version stored in the snapshot.
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    /// Active player states.
    #[must_use]
    pub fn players(&self) -> &[PlayerState] {
        &self.players
    }

    /// Current player state.
    #[must_use]
    pub fn current(&self) -> &PlayerState {
        &self.players[self.current_player]
    }

    /// Current player index.
    #[must_use]
    pub const fn current_player_index(&self) -> usize {
        self.current_player
    }

    /// Current interaction phase.
    #[must_use]
    pub const fn phase(&self) -> &TurnPhase {
        &self.phase
    }

    /// Overall status.
    #[must_use]
    pub const fn status(&self) -> GameStatus {
        self.status
    }

    /// Applied-command revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Active rules.
    #[must_use]
    pub const fn rules(&self) -> Rules {
        self.rules
    }

    /// Determined placements in order.
    #[must_use]
    pub fn rankings(&self) -> &[PlayerId] {
        &self.rankings
    }

    /// Team ID for a player in a 2v2 game.
    #[must_use]
    pub fn team_id(&self, player: PlayerId) -> Option<u8> {
        self.team_ids.map(|teams| teams[player.index()])
    }

    /// Whether this is a 2v2 match.
    #[must_use]
    pub const fn is_team_game(&self) -> bool {
        self.team_ids.is_some()
    }

    /// Returns a player by ID.
    #[must_use]
    pub fn player(&self, id: PlayerId) -> Option<&PlayerState> {
        self.players.get(id.index())
    }

    /// Updates a player's display identity and controller without changing
    /// tokens, turn state, rules, or revision.
    ///
    /// This is used by authoritative lobbies when a human takes over a
    /// computer seat or a disconnected player falls back to AI.
    ///
    /// # Errors
    ///
    /// Returns an error if the player does not exist or the resulting player
    /// setup would violate identity invariants.
    pub fn update_player_control(
        &mut self,
        id: PlayerId,
        name: String,
        controller: Controller,
    ) -> Result<(), DomainError> {
        let mut players = self
            .players
            .iter()
            .map(|state| state.player.clone())
            .collect::<Vec<_>>();
        let Some(player) = players.get_mut(id.index()) else {
            return Err(DomainError::InvalidPlayers);
        };
        player.name = name;
        player.controller = controller;
        Self::validate_players(&players)?;
        let replacement = players
            .into_iter()
            .nth(id.index())
            .ok_or(DomainError::InvalidPlayers)?;
        self.players[id.index()].player = replacement;
        Ok(())
    }

    /// Applies one command and returns resulting facts.
    ///
    /// # Errors
    ///
    /// Returns an error when the match is over, the phase is wrong, or the
    /// selected token is illegal.
    pub fn apply(&mut self, command: GameCommand) -> Result<Vec<GameEvent>, DomainError> {
        if !matches!(self.status, GameStatus::Playing) {
            return Err(DomainError::GameOver);
        }
        let events = match command {
            GameCommand::Roll(value) => self.apply_roll(value)?,
            GameCommand::Move(token) => self.apply_move(token)?,
        };
        self.revision = self.revision.saturating_add(1);
        Ok(events)
    }

    /// Returns the global shared-track index for a token, when applicable.
    #[must_use]
    pub fn global_track_index(&self, player: PlayerId, token: TokenId) -> Option<u8> {
        let state = self.players.get(player.index())?;
        global_index(
            state.player.color,
            state.tokens.get(token.index())?.position,
        )
    }

    /// Calculates legal tokens for a dice result without mutation.
    #[must_use]
    pub fn legal_tokens(&self, dice: DiceValue) -> Vec<TokenId> {
        let player = self.current().player.id;
        self.current()
            .tokens
            .iter()
            .filter(|token| self.destination(player, token.position, dice).is_some())
            .map(|token| token.id)
            .collect()
    }

    /// Whether a global shared-track index is protected by the active rules.
    #[must_use]
    pub fn is_safe_track_index(&self, global: u8) -> bool {
        if global >= TRACK_LEN {
            return false;
        }
        match self.rules.safe_cells {
            SafeCellRule::None => false,
            SafeCellRule::Starts => START_CELLS.contains(&global),
            SafeCellRule::StartsAndStars => {
                START_CELLS.contains(&global) || STAR_CELLS.contains(&global)
            }
        }
    }

    fn validate_players(players: &[Player]) -> Result<(), DomainError> {
        if !(2..=4).contains(&players.len()) {
            return Err(DomainError::InvalidPlayerCount);
        }
        for (index, player) in players.iter().enumerate() {
            let expected_id = u8::try_from(index)
                .ok()
                .and_then(PlayerId::new)
                .ok_or(DomainError::InvalidPlayers)?;
            if player.id != expected_id
                || player.name.trim().is_empty()
                || players
                    .iter()
                    .filter(|other| other.color == player.color)
                    .count()
                    != 1
            {
                return Err(DomainError::InvalidPlayers);
            }
        }
        Ok(())
    }

    fn apply_roll(&mut self, value: DiceValue) -> Result<Vec<GameEvent>, DomainError> {
        if !matches!(self.phase, TurnPhase::AwaitingRoll) {
            return Err(DomainError::WrongPhase);
        }
        let player = self.current().player.id;
        let mut events = vec![GameEvent::DiceRolled { player, value }];
        self.consecutive_sixes = if value.get() == 6 {
            self.consecutive_sixes.saturating_add(1)
        } else {
            0
        };
        if self.rules.three_sixes_forfeit && self.consecutive_sixes == 3 {
            self.consecutive_sixes = 0;
            self.advance_turn(&mut events);
            return Ok(events);
        }
        let legal_tokens = self.legal_tokens(value);
        if legal_tokens.is_empty() {
            if value.get() == 6 && self.rules.extra_turn_on_six {
                self.phase = TurnPhase::AwaitingRoll;
            } else {
                self.advance_turn(&mut events);
            }
        } else {
            self.phase = TurnPhase::AwaitingMove {
                dice: value,
                legal_tokens,
            };
        }
        Ok(events)
    }

    fn apply_move(&mut self, token: TokenId) -> Result<Vec<GameEvent>, DomainError> {
        let dice = match &self.phase {
            TurnPhase::AwaitingMove { dice, legal_tokens } if legal_tokens.contains(&token) => {
                *dice
            }
            TurnPhase::AwaitingMove { .. } => return Err(DomainError::IllegalMove),
            TurnPhase::AwaitingRoll => return Err(DomainError::WrongPhase),
        };
        let player = self.current().player.id;
        let from = self.players[self.current_player].tokens[token.index()].position;
        let to = self
            .destination(player, from, dice)
            .ok_or(DomainError::IllegalMove)?;
        self.players[self.current_player].tokens[token.index()].position = to;
        let reached_home = matches!(to, TokenPosition::Finished);
        let mut events = vec![GameEvent::TokenMoved {
            player,
            token,
            from,
            to,
        }];
        let captured = self.capture_at_destination(player, token, &mut events);

        let player_completed = self.player_completed(player);
        if player_completed
            && self.team_ids.is_some()
            && matches!(self.rules.win_condition, WinCondition::FirstWinner)
        {
            if self.team_completed(player) {
                let winning_team = self.team_id(player);
                self.rankings = self
                    .players
                    .iter()
                    .filter(|state| self.team_id(state.player.id) == winning_team)
                    .map(|state| state.player.id)
                    .collect();
                for (index, winner) in self.rankings.iter().copied().enumerate() {
                    events.push(GameEvent::PlayerRanked {
                        player: winner,
                        place: u8::try_from(index + 1).unwrap_or(u8::MAX),
                    });
                }
                self.finish(&mut events);
                return Ok(events);
            }
        } else if player_completed && !self.rankings.contains(&player) {
            self.rankings.push(player);
            events.push(GameEvent::PlayerRanked {
                player,
                place: u8::try_from(self.rankings.len()).unwrap_or(u8::MAX),
            });
            if matches!(self.rules.win_condition, WinCondition::FirstWinner) {
                self.finish(&mut events);
                return Ok(events);
            }
            if self.rankings.len() + 1 == self.players.len() {
                if let Some(last) = self
                    .players
                    .iter()
                    .map(|state| state.player.id)
                    .find(|id| !self.rankings.contains(id))
                {
                    self.rankings.push(last);
                    events.push(GameEvent::PlayerRanked {
                        player: last,
                        place: u8::try_from(self.rankings.len()).unwrap_or(u8::MAX),
                    });
                }
                self.finish(&mut events);
                return Ok(events);
            }
        }

        let keeps_turn = (dice.get() == 6 && self.rules.extra_turn_on_six)
            || (captured && self.rules.extra_turn_on_capture)
            || (reached_home && self.rules.extra_turn_on_home);
        if keeps_turn && !player_completed {
            self.phase = TurnPhase::AwaitingRoll;
        } else {
            self.advance_turn(&mut events);
        }
        Ok(events)
    }

    fn destination(
        &self,
        player: PlayerId,
        position: TokenPosition,
        dice: DiceValue,
    ) -> Option<TokenPosition> {
        let destination = match position {
            TokenPosition::Yard if dice.get() == 6 => TokenPosition::Path(0),
            TokenPosition::Path(progress) => {
                let next = progress.checked_add(dice.get())?;
                if next < FINISH_PROGRESS {
                    TokenPosition::Path(next)
                } else if next == FINISH_PROGRESS || !self.rules.exact_home_roll {
                    TokenPosition::Finished
                } else {
                    return None;
                }
            }
            TokenPosition::Yard | TokenPosition::Finished => return None,
        };
        if self.path_crosses_opponent_blockade(player, position, destination) {
            None
        } else {
            Some(destination)
        }
    }

    fn path_crosses_opponent_blockade(
        &self,
        player: PlayerId,
        from: TokenPosition,
        to: TokenPosition,
    ) -> bool {
        if !self.rules.blockades {
            return false;
        }
        let color = self.players[player.index()].player.color;
        let (start, end) = match (from, to) {
            (TokenPosition::Yard, TokenPosition::Path(0)) => (0, 0),
            (TokenPosition::Path(start), TokenPosition::Path(end)) => {
                (start.saturating_add(1), end.min(TRACK_LEN - 1))
            }
            (TokenPosition::Path(start), TokenPosition::Finished) => {
                (start.saturating_add(1), TRACK_LEN - 1)
            }
            _ => return false,
        };
        (start..=end).any(|progress| {
            let global = (color.start_index() + progress) % TRACK_LEN;
            self.opponent_count_at(player, global) >= 2
        })
    }

    fn opponent_count_at(&self, player: PlayerId, global: u8) -> usize {
        self.players
            .iter()
            .filter(|state| self.are_opponents(player, state.player.id))
            .flat_map(|state| {
                state.tokens.iter().filter(move |token| {
                    global_index(state.player.color, token.position) == Some(global)
                })
            })
            .count()
    }

    fn capture_at_destination(
        &mut self,
        player: PlayerId,
        token: TokenId,
        events: &mut Vec<GameEvent>,
    ) -> bool {
        let Some(global) = self.global_track_index(player, token) else {
            return false;
        };
        if self.is_safe_track_index(global) {
            return false;
        }
        let mut captured = false;
        for opponent_index in 0..self.players.len() {
            if !self.are_opponents(player, self.players[opponent_index].player.id) {
                continue;
            }
            let opponent_id = self.players[opponent_index].player.id;
            let color = self.players[opponent_index].player.color;
            for opponent_token in &mut self.players[opponent_index].tokens {
                if global_index(color, opponent_token.position) == Some(global) {
                    opponent_token.position = TokenPosition::Yard;
                    events.push(GameEvent::TokenCaptured {
                        by: player,
                        player: opponent_id,
                        token: opponent_token.id,
                    });
                    captured = true;
                }
            }
        }
        captured
    }

    fn are_opponents(&self, left: PlayerId, right: PlayerId) -> bool {
        left != right
            && self
                .team_ids
                .is_none_or(|teams| teams[left.index()] != teams[right.index()])
    }

    fn team_completed(&self, player: PlayerId) -> bool {
        let Some(team) = self.team_id(player) else {
            return false;
        };
        self.players
            .iter()
            .filter(|state| self.team_id(state.player.id) == Some(team))
            .all(|state| {
                state
                    .tokens
                    .iter()
                    .all(|token| matches!(token.position, TokenPosition::Finished))
            })
    }

    fn player_completed(&self, player: PlayerId) -> bool {
        self.players[player.index()]
            .tokens
            .iter()
            .all(|token| matches!(token.position, TokenPosition::Finished))
    }

    fn finish(&mut self, events: &mut Vec<GameEvent>) {
        self.status = GameStatus::Finished;
        self.phase = TurnPhase::AwaitingRoll;
        events.push(GameEvent::GameFinished {
            rankings: self.rankings.clone(),
        });
    }

    fn advance_turn(&mut self, events: &mut Vec<GameEvent>) {
        for _ in 0..self.players.len() {
            self.current_player = (self.current_player + 1) % self.players.len();
            if !self.rankings.contains(&self.current().player.id)
                && !self.player_completed(self.current().player.id)
            {
                break;
            }
        }
        self.phase = TurnPhase::AwaitingRoll;
        self.consecutive_sixes = 0;
        events.push(GameEvent::TurnChanged {
            player: self.current().player.id,
        });
    }
}

fn global_index(color: PlayerColor, position: TokenPosition) -> Option<u8> {
    match position {
        TokenPosition::Path(progress) if progress < TRACK_LEN => {
            Some((color.start_index() + progress) % TRACK_LEN)
        }
        TokenPosition::Yard | TokenPosition::Path(_) | TokenPosition::Finished => None,
    }
}

fn has_duplicates(values: &[PlayerId]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[index + 1..].contains(value))
}

fn valid_team_ids(teams: [u8; 4]) -> bool {
    teams[0] == teams[2]
        && teams[1] == teams[3]
        && teams[0] != teams[1]
        && teams.iter().all(|team| *team <= 1)
}

/// Creates the standard desktop and simulation setup.
#[must_use]
pub fn standard_players() -> Vec<Player> {
    PlayerColor::ALL
        .into_iter()
        .zip([PlayerId(0), PlayerId(1), PlayerId(2), PlayerId(3)])
        .enumerate()
        .map(|(index, (color, id))| Player {
            id,
            name: if index == 0 {
                "You".to_owned()
            } else {
                format!("{} Bot", color.name())
            },
            color,
            controller: if index == 0 {
                Controller::Human
            } else {
                Controller::Bot
            },
            bot_difficulty: BotDifficulty::Hard,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn dice(value: u8) -> DiceValue {
        DiceValue::new(value).unwrap_or_else(|| std::process::abort())
    }

    fn game() -> GameState {
        GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort())
    }

    #[test]
    fn only_six_releases_token() {
        let state = game();
        assert!(state.legal_tokens(dice(3)).is_empty());
        assert_eq!(state.legal_tokens(dice(6)).len(), TOKENS_PER_PLAYER);
    }

    #[test]
    fn released_token_advances_by_roll() {
        let mut state = game();
        let token = TokenId(0);
        assert!(state.apply(GameCommand::Roll(dice(6))).is_ok());
        assert!(state.apply(GameCommand::Move(token)).is_ok());
        assert_eq!(state.players[0].tokens[0].position, TokenPosition::Path(0));
        assert!(state.apply(GameCommand::Roll(dice(4))).is_ok());
        assert!(state.apply(GameCommand::Move(token)).is_ok());
        assert_eq!(state.players[0].tokens[0].position, TokenPosition::Path(4));
    }

    #[test]
    fn opponent_blockade_prevents_passing() {
        let mut state = game();
        state.players[0].tokens[0].position = TokenPosition::Path(3);
        state.players[1].tokens[0].position = TokenPosition::Path(44);
        state.players[1].tokens[1].position = TokenPosition::Path(44);
        assert!(!state.legal_tokens(dice(3)).contains(&TokenId(0)));
    }

    #[test]
    fn quick_rules_allow_home_overshoot() {
        let mut state = GameState::new(standard_players(), RulePreset::Quick.rules())
            .unwrap_or_else(|_| std::process::abort());
        state.players[0].tokens[0].position = TokenPosition::Path(55);
        assert!(state.legal_tokens(dice(6)).contains(&TokenId(0)));
    }

    #[test]
    fn unsafe_opponent_is_captured() {
        let mut state = game();
        state.players[0].tokens[0].position = TokenPosition::Path(4);
        state.players[1].tokens[0].position = TokenPosition::Path(44);
        state.phase = TurnPhase::AwaitingMove {
            dice: dice(1),
            legal_tokens: vec![TokenId(0)],
        };
        let events = state
            .apply(GameCommand::Move(TokenId(0)))
            .unwrap_or_default();
        assert_eq!(state.players[1].tokens[0].position, TokenPosition::Yard);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, GameEvent::TokenCaptured { .. }))
        );
    }

    #[test]
    fn safe_cell_policy_controls_capture() {
        let mut safe_state = game();
        safe_state.players[1].tokens[0].position = TokenPosition::Path(39);
        safe_state.phase = TurnPhase::AwaitingMove {
            dice: dice(6),
            legal_tokens: vec![TokenId(0)],
        };
        assert!(safe_state.apply(GameCommand::Move(TokenId(0))).is_ok());
        assert_eq!(
            safe_state.players[1].tokens[0].position,
            TokenPosition::Path(39)
        );

        let mut unsafe_rules = RulePreset::Classic.rules();
        unsafe_rules.safe_cells = SafeCellRule::None;
        let mut unsafe_state = GameState::new(standard_players(), unsafe_rules)
            .unwrap_or_else(|_| std::process::abort());
        unsafe_state.players[1].tokens[0].position = TokenPosition::Path(39);
        unsafe_state.phase = TurnPhase::AwaitingMove {
            dice: dice(6),
            legal_tokens: vec![TokenId(0)],
        };
        assert!(unsafe_state.apply(GameCommand::Move(TokenId(0))).is_ok());
        assert_eq!(
            unsafe_state.players[1].tokens[0].position,
            TokenPosition::Yard
        );
    }

    #[test]
    fn first_winner_finishes_classic_game() {
        let mut state = game();
        for token in &mut state.players[0].tokens[..3] {
            token.position = TokenPosition::Finished;
        }
        state.players[0].tokens[3].position = TokenPosition::Path(56);
        state.phase = TurnPhase::AwaitingMove {
            dice: dice(1),
            legal_tokens: vec![TokenId(3)],
        };
        assert!(state.apply(GameCommand::Move(TokenId(3))).is_ok());
        assert_eq!(state.status(), GameStatus::Finished);
        assert_eq!(state.rankings(), &[PlayerId(0)]);
    }

    #[test]
    fn third_consecutive_six_forfeits_turn() {
        let mut state = game();
        for _ in 0..2 {
            assert!(state.apply(GameCommand::Roll(dice(6))).is_ok());
            assert!(state.apply(GameCommand::Move(TokenId(0))).is_ok());
        }
        assert!(state.apply(GameCommand::Roll(dice(6))).is_ok());
        assert_eq!(state.current_player_index(), 1);
        assert!(matches!(state.phase(), TurnPhase::AwaitingRoll));
    }

    #[test]
    fn serialized_state_requires_validation() {
        let state = game();
        let encoded = serde_json::to_vec(&state).unwrap_or_default();
        let decoded: Result<GameState, _> = serde_json::from_slice(&encoded);
        assert!(decoded.is_ok_and(|snapshot| snapshot.validated().is_ok()));
    }

    #[test]
    fn tournament_continues_after_first_placement() {
        let mut state = GameState::new(standard_players(), RulePreset::Tournament.rules())
            .unwrap_or_else(|_| std::process::abort());
        for token in &mut state.players[0].tokens[..3] {
            token.position = TokenPosition::Finished;
        }
        state.players[0].tokens[3].position = TokenPosition::Path(56);
        state.phase = TurnPhase::AwaitingMove {
            dice: dice(1),
            legal_tokens: vec![TokenId(3)],
        };
        assert!(state.apply(GameCommand::Move(TokenId(3))).is_ok());
        assert_eq!(state.status(), GameStatus::Playing);
        assert_eq!(state.rankings(), &[PlayerId(0)]);
        assert_ne!(state.current().player.id, PlayerId(0));
    }

    #[test]
    fn team_allies_cannot_capture_each_other() {
        let mut state = GameState::new_team(standard_players(), Rules::default(), [0, 1, 0, 1])
            .unwrap_or_else(|_| std::process::abort());
        state.players[0].tokens[0].position = TokenPosition::Path(4);
        state.players[2].tokens[0].position = TokenPosition::Path(31);
        assert!(state.apply(GameCommand::Roll(dice(1))).is_ok());
        assert!(state.apply(GameCommand::Move(TokenId(0))).is_ok());
        assert_eq!(state.players[2].tokens[0].position, TokenPosition::Path(31));
    }

    #[test]
    fn team_finishes_only_after_both_allies_complete() {
        let mut state = GameState::new_team(standard_players(), Rules::default(), [0, 1, 0, 1])
            .unwrap_or_else(|_| std::process::abort());
        for token in &mut state.players[0].tokens {
            token.position = TokenPosition::Finished;
        }
        state.players[0].tokens[0].position = TokenPosition::Path(56);
        assert!(state.apply(GameCommand::Roll(dice(1))).is_ok());
        assert!(state.apply(GameCommand::Move(TokenId(0))).is_ok());
        assert!(matches!(state.status(), GameStatus::Playing));

        state.current_player = 2;
        state.phase = TurnPhase::AwaitingRoll;
        for token in &mut state.players[2].tokens {
            token.position = TokenPosition::Finished;
        }
        state.players[2].tokens[0].position = TokenPosition::Path(56);
        assert!(state.apply(GameCommand::Roll(dice(1))).is_ok());
        assert!(state.apply(GameCommand::Move(TokenId(0))).is_ok());
        assert!(matches!(state.status(), GameStatus::Finished));
        assert!(state.clone().validated().is_ok());
    }

    #[test]
    fn interleaved_team_completions_rank_only_the_winning_pair() {
        let mut state = GameState::new_team(standard_players(), Rules::default(), [0, 1, 0, 1])
            .unwrap_or_else(|_| std::process::abort());
        for player_index in [0_usize, 1, 2] {
            state.current_player = player_index;
            state.phase = TurnPhase::AwaitingRoll;
            for token in &mut state.players[player_index].tokens {
                token.position = TokenPosition::Finished;
            }
            state.players[player_index].tokens[0].position = TokenPosition::Path(56);
            assert!(state.apply(GameCommand::Roll(dice(1))).is_ok());
            let events = state
                .apply(GameCommand::Move(TokenId(0)))
                .unwrap_or_default();
            if player_index < 2 {
                assert!(state.rankings().is_empty());
                assert!(
                    events
                        .iter()
                        .all(|event| !matches!(event, GameEvent::PlayerRanked { .. }))
                );
            }
        }
        assert!(matches!(state.status(), GameStatus::Finished));
        assert_eq!(state.rankings(), &[PlayerId(0), PlayerId(2)]);
        assert!(state.clone().validated().is_ok());
    }

    #[test]
    fn team_members_must_use_opposite_board_colors() {
        assert!(matches!(
            GameState::new_team(standard_players(), Rules::default(), [0, 0, 1, 1]),
            Err(DomainError::InvalidTeams)
        ));
        assert!(GameState::new_team(standard_players(), Rules::default(), [1, 0, 1, 0]).is_ok());
    }

    #[test]
    fn rejected_commands_leave_the_state_byte_for_byte_unchanged() {
        let mut state = game();
        let initial = state.clone();
        assert!(matches!(
            state.apply(GameCommand::Move(TokenId(0))),
            Err(DomainError::WrongPhase)
        ));
        assert_eq!(state, initial);

        state.players[0].tokens[0].position = TokenPosition::Path(0);
        for token in &mut state.players[0].tokens[1..] {
            token.position = TokenPosition::Finished;
        }
        state.phase = TurnPhase::AwaitingMove {
            dice: dice(1),
            legal_tokens: vec![TokenId(0)],
        };
        let awaiting_move = state.clone();
        assert!(matches!(
            state.apply(GameCommand::Move(TokenId(1))),
            Err(DomainError::IllegalMove)
        ));
        assert_eq!(state, awaiting_move);
    }

    #[test]
    fn dice_and_identity_constructors_enforce_board_bounds() {
        assert!(DiceValue::new(0).is_none());
        assert!(DiceValue::new(7).is_none());
        assert!(PlayerId::new(4).is_none());
        assert!(TokenId::new(4).is_none());
        assert_eq!(DiceValue::new(1).map(DiceValue::get), Some(1));
        assert_eq!(DiceValue::new(6).map(DiceValue::get), Some(6));
    }

    #[test]
    fn global_track_coordinates_rotate_with_player_color() {
        let mut state = game();
        for player in &mut state.players {
            player.tokens[0].position = TokenPosition::Path(0);
        }
        assert_eq!(state.global_track_index(PlayerId(0), TokenId(0)), Some(0));
        assert_eq!(state.global_track_index(PlayerId(1), TokenId(0)), Some(13));
        assert_eq!(state.global_track_index(PlayerId(2), TokenId(0)), Some(26));
        assert_eq!(state.global_track_index(PlayerId(3), TokenId(0)), Some(39));
    }

    proptest! {
        #[test]
        fn arbitrary_dice_streams_preserve_snapshot_invariants(
            rolls in prop::collection::vec(1_u8..=6, 1..500)
        ) {
            let mut state = GameState::new(
                standard_players(),
                RulePreset::Tournament.rules(),
            ).unwrap_or_else(|_| std::process::abort());

            for roll in rolls {
                if matches!(state.status(), GameStatus::Finished) {
                    break;
                }
                if matches!(state.phase(), TurnPhase::AwaitingRoll) {
                    let value = DiceValue::new(roll).unwrap_or_else(|| std::process::abort());
                    prop_assert!(state.apply(GameCommand::Roll(value)).is_ok());
                }
                if let TurnPhase::AwaitingMove { legal_tokens, .. } = state.phase() {
                    let selected = legal_tokens.first().copied();
                    prop_assert!(selected.is_some());
                    if let Some(token) = selected {
                        prop_assert!(state.apply(GameCommand::Move(token)).is_ok());
                    }
                }
                prop_assert!(
                    state.clone().validated().is_ok(),
                    "invalid generated state: {state:#?}"
                );
            }
        }
    }
}
