//! Pure, deterministic Ludo rules.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Number of tokens owned by each player.
pub const TOKENS_PER_PLAYER: usize = 4;
/// Number of positions on the shared track.
pub const TRACK_LEN: u8 = 52;
/// The exact progress at which a token finishes.
pub const FINISH_PROGRESS: u8 = 57;
const SAFE_TRACK_CELLS: [u8; 8] = [0, 8, 13, 21, 26, 34, 39, 47];

/// Stable player identity and array index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlayerId(u8);

impl PlayerId {
    /// Creates a player ID when the index belongs to a four-player board.
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
    /// Creates a token ID when the index belongs to a player's token set.
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

/// The four Ludo colors in clockwise turn order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlayerColor {
    /// Top-left player.
    Red,
    /// Top-right player.
    Green,
    /// Bottom-right player.
    Yellow,
    /// Bottom-left player.
    Blue,
}

impl PlayerColor {
    /// All colors in turn order.
    pub const ALL: [Self; 4] = [Self::Red, Self::Green, Self::Yellow, Self::Blue];

    /// Start index on the shared 52-cell track.
    #[must_use]
    pub const fn start_index(self) -> u8 {
        match self {
            Self::Red => 0,
            Self::Green => 13,
            Self::Yellow => 26,
            Self::Blue => 39,
        }
    }

    /// User-facing color name.
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

/// Whether a seat is controlled locally or by a computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Controller {
    /// Local human player.
    Human,
    /// Computer player.
    Bot,
}

/// A player's immutable setup information.
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
}

/// Logical token position, independent of any GUI coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenPosition {
    /// Token has not entered the board.
    Yard,
    /// Progress from the player's start. Values 0–51 are shared track and
    /// 52–56 are the player's home lane.
    Path(u8),
    /// Token completed the path.
    Finished,
}

/// Runtime token state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    /// ID within its owner.
    pub id: TokenId,
    /// Current logical position.
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

/// Customizable rules supported by the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rules {
    /// A six grants another roll or move.
    pub extra_turn_on_six: bool,
    /// Capturing a token grants another turn.
    pub extra_turn_on_capture: bool,
    /// Three consecutive sixes forfeit the turn.
    pub three_sixes_forfeit: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            extra_turn_on_six: true,
            extra_turn_on_capture: true,
            three_sixes_forfeit: true,
        }
    }
}

/// Current interaction expected by the rules engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnPhase {
    /// The current player must roll.
    AwaitingRoll,
    /// The current player rolled and must choose one of these tokens.
    AwaitingMove {
        /// Rolled value.
        dice: DiceValue,
        /// Tokens that may legally move.
        legal_tokens: Vec<TokenId>,
    },
}

/// Overall match status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameStatus {
    /// Match accepts commands.
    Playing,
    /// A player brought all tokens home.
    Won(PlayerId),
}

/// A validated dice value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiceValue(u8);

impl DiceValue {
    /// Creates a standard six-sided dice value.
    #[must_use]
    pub const fn new(value: u8) -> Option<Self> {
        if value >= 1 && value <= 6 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Raw value from one through six.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Commands accepted by a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Control moved to another player.
    TurnChanged {
        /// New current player.
        player: PlayerId,
    },
    /// Match ended.
    GameWon {
        /// Winner.
        player: PlayerId,
    },
}

/// A complete serializable match snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameState {
    /// Active seats.
    pub players: Vec<PlayerState>,
    /// Zero-based index into `players`.
    pub current_player: usize,
    /// Current input phase.
    pub phase: TurnPhase,
    /// Match status.
    pub status: GameStatus,
    /// Applied-command revision.
    pub revision: u64,
    /// Rules for this match.
    pub rules: Rules,
    consecutive_sixes: u8,
}

/// Domain validation failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DomainError {
    /// Player configuration is outside the supported range.
    #[error("a game requires between two and four players")]
    InvalidPlayerCount,
    /// Player identities or colors are inconsistent.
    #[error("players must have unique sequential IDs and colors")]
    InvalidPlayers,
    /// Match already ended.
    #[error("the game is already over")]
    GameOver,
    /// Command does not match the current turn phase.
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
    /// Returns an error when the player count, identities, or colors do not
    /// describe a valid board.
    pub fn new(players: Vec<Player>, rules: Rules) -> Result<Self, DomainError> {
        if !(2..=4).contains(&players.len()) {
            return Err(DomainError::InvalidPlayerCount);
        }
        for (index, player) in players.iter().enumerate() {
            let expected_id = u8::try_from(index)
                .ok()
                .and_then(PlayerId::new)
                .ok_or(DomainError::InvalidPlayers)?;
            if player.id != expected_id
                || players
                    .iter()
                    .filter(|other| other.color == player.color)
                    .count()
                    != 1
            {
                return Err(DomainError::InvalidPlayers);
            }
        }
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
            players,
            current_player: 0,
            phase: TurnPhase::AwaitingRoll,
            status: GameStatus::Playing,
            revision: 0,
            rules,
            consecutive_sixes: 0,
        })
    }

    /// Current player state.
    #[must_use]
    pub fn current(&self) -> &PlayerState {
        &self.players[self.current_player]
    }

    /// Applies one command and returns the resulting facts.
    ///
    /// # Errors
    ///
    /// Returns an error when the match is over, the command is for the wrong
    /// phase, or the selected token is not legal.
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
        match state.tokens.get(token.index())?.position {
            TokenPosition::Path(progress) if progress < TRACK_LEN => {
                Some((state.player.color.start_index() + progress) % TRACK_LEN)
            }
            TokenPosition::Yard | TokenPosition::Path(_) | TokenPosition::Finished => None,
        }
    }

    /// Calculates legal tokens for a dice result without mutating state.
    #[must_use]
    pub fn legal_tokens(&self, dice: DiceValue) -> Vec<TokenId> {
        self.current()
            .tokens
            .iter()
            .filter(|token| Self::destination(token.position, dice).is_some())
            .map(|token| token.id)
            .collect()
    }

    fn apply_roll(&mut self, value: DiceValue) -> Result<Vec<GameEvent>, DomainError> {
        if !matches!(self.phase, TurnPhase::AwaitingRoll) {
            return Err(DomainError::WrongPhase);
        }
        let player = self.current().player.id;
        let mut events = vec![GameEvent::DiceRolled { player, value }];
        if value.get() == 6 {
            self.consecutive_sixes = self.consecutive_sixes.saturating_add(1);
        } else {
            self.consecutive_sixes = 0;
        }
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
        let (dice, legal_tokens) = match &self.phase {
            TurnPhase::AwaitingMove { dice, legal_tokens } => (*dice, legal_tokens),
            TurnPhase::AwaitingRoll => return Err(DomainError::WrongPhase),
        };
        if !legal_tokens.contains(&token) {
            return Err(DomainError::IllegalMove);
        }
        let player = self.current().player.id;
        let token_state = &mut self.players[self.current_player].tokens[token.index()];
        let from = token_state.position;
        let to = Self::destination(from, dice).ok_or(DomainError::IllegalMove)?;
        token_state.position = to;
        let mut events = vec![GameEvent::TokenMoved {
            player,
            token,
            from,
            to,
        }];
        let captured = self.capture_at_destination(player, token, &mut events);
        if self.players[self.current_player]
            .tokens
            .iter()
            .all(|owned| matches!(owned.position, TokenPosition::Finished))
        {
            self.status = GameStatus::Won(player);
            self.phase = TurnPhase::AwaitingRoll;
            events.push(GameEvent::GameWon { player });
            return Ok(events);
        }
        let keeps_turn = (dice.get() == 6 && self.rules.extra_turn_on_six)
            || (captured && self.rules.extra_turn_on_capture);
        if keeps_turn {
            self.phase = TurnPhase::AwaitingRoll;
        } else {
            self.advance_turn(&mut events);
        }
        Ok(events)
    }

    fn destination(position: TokenPosition, dice: DiceValue) -> Option<TokenPosition> {
        match position {
            TokenPosition::Yard if dice.get() == 6 => Some(TokenPosition::Path(0)),
            TokenPosition::Path(progress) => {
                let next = progress.checked_add(dice.get())?;
                match next.cmp(&FINISH_PROGRESS) {
                    std::cmp::Ordering::Less => Some(TokenPosition::Path(next)),
                    std::cmp::Ordering::Equal => Some(TokenPosition::Finished),
                    std::cmp::Ordering::Greater => None,
                }
            }
            TokenPosition::Yard | TokenPosition::Finished => None,
        }
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
        if SAFE_TRACK_CELLS.contains(&global) {
            return false;
        }
        let mut captured = false;
        for opponent_index in 0..self.players.len() {
            if opponent_index == player.index() {
                continue;
            }
            let opponent_id = self.players[opponent_index].player.id;
            for token_index in 0..TOKENS_PER_PLAYER {
                let opponent_token = self.players[opponent_index].tokens[token_index].id;
                if self.global_track_index(opponent_id, opponent_token) == Some(global) {
                    self.players[opponent_index].tokens[token_index].position = TokenPosition::Yard;
                    events.push(GameEvent::TokenCaptured {
                        by: player,
                        player: opponent_id,
                        token: opponent_token,
                    });
                    captured = true;
                }
            }
        }
        captured
    }

    fn advance_turn(&mut self, events: &mut Vec<GameEvent>) {
        self.current_player = (self.current_player + 1) % self.players.len();
        self.phase = TurnPhase::AwaitingRoll;
        self.consecutive_sixes = 0;
        events.push(GameEvent::TurnChanged {
            player: self.current().player.id,
        });
    }
}

/// Creates the standard local setup used by the desktop app and simulations.
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
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dice(value: u8) -> DiceValue {
        DiceValue::new(value).unwrap_or(DiceValue(1))
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
        let token = TokenId::new(0).unwrap_or(TokenId(0));
        assert!(state.apply(GameCommand::Roll(dice(6))).is_ok());
        assert!(state.apply(GameCommand::Move(token)).is_ok());
        assert_eq!(state.players[0].tokens[0].position, TokenPosition::Path(0));
        assert!(state.apply(GameCommand::Roll(dice(4))).is_ok());
        assert!(state.apply(GameCommand::Move(token)).is_ok());
        assert_eq!(state.players[0].tokens[0].position, TokenPosition::Path(4));
    }

    #[test]
    fn non_six_without_move_advances_turn() {
        let mut state = game();
        assert!(state.apply(GameCommand::Roll(dice(2))).is_ok());
        assert_eq!(state.current_player, 1);
        assert!(matches!(state.phase, TurnPhase::AwaitingRoll));
    }

    #[test]
    fn oversized_home_roll_is_not_legal() {
        let mut state = game();
        state.players[0].tokens[0].position = TokenPosition::Path(55);
        assert!(state.legal_tokens(dice(3)).is_empty());
        assert_eq!(state.legal_tokens(dice(2)).len(), 1);
    }

    #[test]
    fn invalid_dice_values_are_rejected() {
        assert!(DiceValue::new(0).is_none());
        assert!(DiceValue::new(7).is_none());
    }

    #[test]
    fn landing_on_an_unsafe_opponent_captures_it() {
        let mut state = game();
        let red_token = TokenId(0);
        let green_token = TokenId(0);
        state.players[0].tokens[0].position = TokenPosition::Path(4);
        state.players[1].tokens[0].position = TokenPosition::Path(44);
        state.phase = TurnPhase::AwaitingMove {
            dice: dice(1),
            legal_tokens: vec![red_token],
        };

        let events = state
            .apply(GameCommand::Move(red_token))
            .unwrap_or_default();

        assert_eq!(
            state.players[1].tokens[green_token.index()].position,
            TokenPosition::Yard
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, GameEvent::TokenCaptured { .. }))
        );
    }

    #[test]
    fn exact_final_roll_wins_the_game() {
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
        assert_eq!(state.status, GameStatus::Won(PlayerId(0)));
    }

    #[test]
    fn third_consecutive_six_forfeits_turn() {
        let mut state = game();
        let token = TokenId(0);
        for _ in 0..2 {
            assert!(state.apply(GameCommand::Roll(dice(6))).is_ok());
            assert!(state.apply(GameCommand::Move(token)).is_ok());
        }

        assert!(state.apply(GameCommand::Roll(dice(6))).is_ok());
        assert_eq!(state.current_player, 1);
        assert!(matches!(state.phase, TurnPhase::AwaitingRoll));
    }
}
