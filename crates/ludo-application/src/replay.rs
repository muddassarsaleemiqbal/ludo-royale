//! Versioned, deterministic match recording and playback.

use ludo_domain::{DomainError, GameCommand, GameEvent, GameState};
use serde::{Deserialize, Serialize};

/// Current replay file schema.
pub const REPLAY_SCHEMA_VERSION: u16 = 1;

/// One verified command and its resulting facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayRecord {
    /// Applied deterministic command.
    pub command: GameCommand,
    /// Domain facts produced by the command.
    pub events: Vec<GameEvent>,
}

/// Portable replay payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replay {
    /// Schema used to decode the payload.
    pub schema_version: u16,
    /// State before the first recorded command.
    pub initial_state: GameState,
    /// Commands and facts in application order.
    pub records: Vec<ReplayRecord>,
}

impl Replay {
    /// Starts an empty recording.
    #[must_use]
    pub fn new(initial_state: GameState) -> Self {
        Self {
            schema_version: REPLAY_SCHEMA_VERSION,
            initial_state,
            records: Vec::new(),
        }
    }

    /// Replays every command and verifies stored events.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported schemas, invalid snapshots, illegal
    /// commands, or event data that does not match the deterministic engine.
    pub fn validate(&self) -> Result<GameState, ReplayError> {
        if self.schema_version != REPLAY_SCHEMA_VERSION {
            return Err(ReplayError::UnsupportedSchema(self.schema_version));
        }
        let mut state = self.initial_state.clone().validated()?;
        for record in &self.records {
            let actual = state.apply(record.command)?;
            if actual != record.events {
                return Err(ReplayError::EventMismatch);
            }
        }
        Ok(state)
    }
}

/// Replay playback rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaySpeed {
    /// Half speed.
    Half,
    /// Normal speed.
    Normal,
    /// Double speed.
    Double,
    /// Four times speed.
    Quadruple,
}

impl ReplaySpeed {
    /// Speeds in UI cycle order.
    pub const ALL: [Self; 4] = [Self::Half, Self::Normal, Self::Double, Self::Quadruple];

    /// Delay multiplier represented as numerator and denominator.
    #[must_use]
    pub const fn ratio(self) -> (u64, u64) {
        match self {
            Self::Half => (2, 1),
            Self::Normal => (1, 1),
            Self::Double => (1, 2),
            Self::Quadruple => (1, 4),
        }
    }

    /// User-facing label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Half => "0.5×",
            Self::Normal => "1×",
            Self::Double => "2×",
            Self::Quadruple => "4×",
        }
    }
}

/// Seekable deterministic replay cursor.
#[derive(Debug, Clone)]
pub struct ReplayPlayer {
    replay: Replay,
    state: GameState,
    cursor: usize,
    playing: bool,
    speed: ReplaySpeed,
}

impl ReplayPlayer {
    /// Opens and validates a replay.
    ///
    /// # Errors
    ///
    /// Returns a replay validation error.
    pub fn new(replay: Replay) -> Result<Self, ReplayError> {
        replay.validate()?;
        let state = replay.initial_state.clone();
        Ok(Self {
            replay,
            state,
            cursor: 0,
            playing: false,
            speed: ReplaySpeed::Normal,
        })
    }

    /// Current reconstructed state.
    #[must_use]
    pub const fn state(&self) -> &GameState {
        &self.state
    }

    /// Current command cursor.
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// Number of recorded commands.
    #[must_use]
    pub fn len(&self) -> usize {
        self.replay.records.len()
    }

    /// Whether there are no commands.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.replay.records.is_empty()
    }

    /// Whether timed playback is active.
    #[must_use]
    pub const fn is_playing(&self) -> bool {
        self.playing
    }

    /// Current playback speed.
    #[must_use]
    pub const fn speed(&self) -> ReplaySpeed {
        self.speed
    }

    /// Sets timed playback state.
    pub const fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    /// Cycles the playback rate.
    pub fn cycle_speed(&mut self) {
        let next = ReplaySpeed::ALL
            .iter()
            .position(|speed| *speed == self.speed)
            .map_or(0, |index| (index + 1) % ReplaySpeed::ALL.len());
        self.speed = ReplaySpeed::ALL[next];
    }

    /// Applies the next recorded command.
    ///
    /// # Errors
    ///
    /// Returns an error if replay data became invalid.
    pub fn step(&mut self) -> Result<Option<&ReplayRecord>, ReplayError> {
        let Some(record) = self.replay.records.get(self.cursor) else {
            self.playing = false;
            return Ok(None);
        };
        let actual = self.state.apply(record.command)?;
        if actual != record.events {
            return Err(ReplayError::EventMismatch);
        }
        self.cursor += 1;
        Ok(Some(record))
    }

    /// Reconstructs state at a command boundary.
    ///
    /// # Errors
    ///
    /// Returns an error when the cursor is out of range or replay data fails.
    pub fn seek(&mut self, cursor: usize) -> Result<(), ReplayError> {
        if cursor > self.replay.records.len() {
            return Err(ReplayError::CursorOutOfRange);
        }
        self.state = self.replay.initial_state.clone().validated()?;
        self.cursor = 0;
        while self.cursor < cursor {
            let _ = self.step()?;
        }
        Ok(())
    }
}

/// Replay validation and playback failures.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ReplayError {
    /// File schema is newer or otherwise unsupported.
    #[error("replay schema {0} is not supported")]
    UnsupportedSchema(u16),
    /// Embedded snapshot or command failed domain validation.
    #[error(transparent)]
    Domain(#[from] DomainError),
    /// Stored events were altered or came from a different engine.
    #[error("replay events do not match deterministic command output")]
    EventMismatch,
    /// Seek target exceeds the replay length.
    #[error("replay cursor is out of range")]
    CursorOutOfRange,
}

/// Storage boundary for portable replay files.
pub trait ReplayRepository: Send + Sync {
    /// Writes one replay.
    ///
    /// # Errors
    ///
    /// Returns an adapter-specific persistence message.
    fn save_replay(&self, replay: &Replay) -> Result<(), String>;

    /// Reads one replay.
    ///
    /// # Errors
    ///
    /// Returns an adapter-specific persistence or validation message.
    fn load_replay(&self) -> Result<Option<Replay>, String>;
}

#[cfg(test)]
mod tests {
    use ludo_domain::{DiceValue, GameCommand, GameState, Rules, standard_players};

    use super::*;

    fn recorded_replay() -> Replay {
        let initial = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut replay = Replay::new(initial.clone());
        let mut state = initial;
        let commands = [
            GameCommand::Roll(DiceValue::new(6).unwrap_or_else(|| std::process::abort())),
            GameCommand::Move(
                ludo_domain::TokenId::new(0).unwrap_or_else(|| std::process::abort()),
            ),
            GameCommand::Roll(DiceValue::new(2).unwrap_or_else(|| std::process::abort())),
            GameCommand::Move(
                ludo_domain::TokenId::new(0).unwrap_or_else(|| std::process::abort()),
            ),
        ];
        for command in commands {
            let events = state
                .apply(command)
                .unwrap_or_else(|_| std::process::abort());
            replay.records.push(ReplayRecord { command, events });
        }
        replay
    }

    #[test]
    fn replay_can_seek_and_detect_tampering() {
        let initial = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut replay = Replay::new(initial.clone());
        let mut state = initial;
        let command = GameCommand::Roll(DiceValue::new(6).unwrap_or_else(|| std::process::abort()));
        let events = state
            .apply(command)
            .unwrap_or_else(|_| std::process::abort());
        replay.records.push(ReplayRecord { command, events });

        let mut player =
            ReplayPlayer::new(replay.clone()).unwrap_or_else(|_| std::process::abort());
        assert!(player.step().is_ok());
        assert_eq!(player.cursor(), 1);
        assert!(player.seek(0).is_ok());
        replay.records[0].events.clear();
        assert!(matches!(replay.validate(), Err(ReplayError::EventMismatch)));
    }

    #[test]
    fn seeking_backward_and_forward_reconstructs_exact_snapshots() {
        let replay = recorded_replay();
        let mut expected = replay.initial_state.clone();
        for record in &replay.records[..3] {
            let _ = expected
                .apply(record.command)
                .unwrap_or_else(|_| std::process::abort());
        }
        let mut player = ReplayPlayer::new(replay).unwrap_or_else(|_| std::process::abort());
        assert!(player.seek(3).is_ok());
        assert_eq!(player.state(), &expected);
        assert!(player.seek(1).is_ok());
        assert_eq!(player.cursor(), 1);
        assert!(player.seek(0).is_ok());
        assert_eq!(player.cursor(), 0);
    }

    #[test]
    fn cursor_bounds_and_end_of_stream_are_explicit() {
        let replay = recorded_replay();
        let length = replay.records.len();
        let mut player = ReplayPlayer::new(replay).unwrap_or_else(|_| std::process::abort());
        assert!(matches!(
            player.seek(length + 1),
            Err(ReplayError::CursorOutOfRange)
        ));
        assert_eq!(player.cursor(), 0);
        assert!(player.seek(length).is_ok());
        player.set_playing(true);
        assert!(player.step().is_ok_and(|record| record.is_none()));
        assert!(!player.is_playing());
    }

    #[test]
    fn playback_speed_cycles_in_display_order_and_wraps() {
        let replay = recorded_replay();
        let mut player = ReplayPlayer::new(replay).unwrap_or_else(|_| std::process::abort());
        assert_eq!(player.speed(), ReplaySpeed::Normal);
        player.cycle_speed();
        assert_eq!(player.speed(), ReplaySpeed::Double);
        player.cycle_speed();
        assert_eq!(player.speed(), ReplaySpeed::Quadruple);
        player.cycle_speed();
        assert_eq!(player.speed(), ReplaySpeed::Half);
        player.cycle_speed();
        assert_eq!(player.speed(), ReplaySpeed::Normal);
    }

    #[test]
    fn unsupported_schema_is_rejected_before_playback() {
        let mut replay = recorded_replay();
        replay.schema_version = REPLAY_SCHEMA_VERSION.saturating_add(1);
        assert!(matches!(
            ReplayPlayer::new(replay),
            Err(ReplayError::UnsupportedSchema(_))
        ));
    }
}
