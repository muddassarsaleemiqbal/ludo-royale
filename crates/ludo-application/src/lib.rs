//! GUI-independent Ludo use cases and infrastructure ports.

pub mod profiles;
pub mod replay;
pub mod rule_presets;

use std::{collections::VecDeque, sync::Arc};

use ludo_domain::{DiceValue, DomainError, GameCommand, GameEvent, GameState, TokenId};
use replay::{Replay, ReplayRecord};
use thiserror::Error;

const MAX_UNDO_SNAPSHOTS: usize = 512;

/// Supplies dice rolls to application use cases.
pub trait DiceSource: Send + Sync {
    /// Produces one valid roll.
    fn roll(&self) -> DiceValue;
}

/// Short semantic sound effects requested by application events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundCue {
    /// Dice rolling.
    Dice,
    /// One token step.
    Move,
    /// Opponent capture.
    Capture,
    /// Token reached home.
    Home,
    /// Turn passed to another player.
    Turn,
    /// Match completion.
    Victory,
}

/// Non-blocking sound output port.
pub trait SoundPlayer: Send + Sync {
    /// Queues a semantic effect.
    fn play(&self, cue: SoundCue);

    /// Enables or suppresses effects.
    fn set_enabled(&self, enabled: bool);
}

/// Persists complete match snapshots.
pub trait GameRepository: Send + Sync {
    /// Stores the latest snapshot.
    ///
    /// # Errors
    ///
    /// Returns an adapter-specific message when persistence fails.
    fn save(&self, state: &GameState) -> Result<(), String>;

    /// Loads a previously stored snapshot.
    ///
    /// # Errors
    ///
    /// Returns an adapter-specific message when reading or decoding fails.
    fn load(&self) -> Result<Option<GameState>, String>;
}

/// Use-case failures with domain errors kept distinct from external failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ApplicationError {
    /// Rules rejected a command.
    #[error(transparent)]
    Domain(#[from] DomainError),
    /// Persistence adapter failed.
    #[error("persistence failed: {0}")]
    Persistence(String),
    /// No prior local command is available.
    #[error("there is no command to undo")]
    UndoUnavailable,
    /// Match policy forbids undo.
    #[error("undo is disabled for competitive matches")]
    UndoDisabled,
}

/// Whether local snapshots may be restored through undo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoPolicy {
    /// Casual local play.
    Allowed,
    /// Tournament, team, or network-authoritative play.
    Competitive,
}

/// Stateful application facade used by any presentation technology.
pub struct GameSession {
    state: GameState,
    dice: Arc<dyn DiceSource>,
    repository: Option<Arc<dyn GameRepository>>,
    history: VecDeque<GameState>,
    undo_policy: UndoPolicy,
    replay: Replay,
}

impl GameSession {
    /// Creates a session around a new or restored match.
    #[must_use]
    pub fn new(state: GameState, dice: Arc<dyn DiceSource>) -> Self {
        let replay = Replay::new(state.clone());
        Self {
            state,
            dice,
            repository: None,
            history: VecDeque::new(),
            undo_policy: UndoPolicy::Allowed,
            replay,
        }
    }

    /// Installs an optional persistence adapter.
    #[must_use]
    pub fn with_repository(mut self, repository: Arc<dyn GameRepository>) -> Self {
        self.repository = Some(repository);
        self
    }

    /// Applies the undo policy for this match.
    #[must_use]
    pub const fn with_undo_policy(mut self, policy: UndoPolicy) -> Self {
        self.undo_policy = policy;
        self
    }

    /// Updates the undo policy when starting a differently classified match.
    pub const fn set_undo_policy(&mut self, policy: UndoPolicy) {
        self.undo_policy = policy;
    }

    /// Current undo policy.
    #[must_use]
    pub const fn undo_policy(&self) -> UndoPolicy {
        self.undo_policy
    }

    /// Current immutable snapshot.
    #[must_use]
    pub const fn state(&self) -> &GameState {
        &self.state
    }

    /// Replaces the current state with a restored snapshot.
    pub fn restore(&mut self, state: GameState) {
        self.replay = Replay::new(state.clone());
        self.history.clear();
        self.state = state;
    }

    /// Current in-memory replay recording.
    #[must_use]
    pub const fn replay(&self) -> &Replay {
        &self.replay
    }

    /// Whether an undo operation is currently available.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        matches!(self.undo_policy, UndoPolicy::Allowed) && !self.history.is_empty()
    }

    /// Restores the state before the latest local command.
    ///
    /// # Errors
    ///
    /// Returns an error when policy forbids undo, history is empty, or
    /// persistence fails.
    pub fn undo(&mut self) -> Result<(), ApplicationError> {
        if matches!(self.undo_policy, UndoPolicy::Competitive) {
            return Err(ApplicationError::UndoDisabled);
        }
        let restored = self
            .history
            .back()
            .cloned()
            .ok_or(ApplicationError::UndoUnavailable)?;
        if let Some(repository) = &self.repository {
            repository
                .save(&restored)
                .map_err(ApplicationError::Persistence)?;
        }
        self.state = restored;
        let _ = self.history.pop_back();
        let _ = self.replay.records.pop();
        Ok(())
    }

    /// Executes the roll-dice use case.
    ///
    /// # Errors
    ///
    /// Returns a domain error when the current phase does not allow rolling.
    pub fn roll(&mut self) -> Result<Vec<GameEvent>, ApplicationError> {
        let value = self.dice.roll();
        self.apply(GameCommand::Roll(value))
    }

    /// Executes the move-token use case.
    ///
    /// # Errors
    ///
    /// Returns a domain error when the token is not currently legal.
    pub fn move_token(&mut self, token: TokenId) -> Result<Vec<GameEvent>, ApplicationError> {
        self.apply(GameCommand::Move(token))
    }

    /// Executes a deterministic command supplied by replay or network clients.
    ///
    /// # Errors
    ///
    /// Returns a domain or persistence error.
    pub fn execute(&mut self, command: GameCommand) -> Result<Vec<GameEvent>, ApplicationError> {
        self.apply(command)
    }

    /// Saves the current snapshot if a repository is configured.
    ///
    /// # Errors
    ///
    /// Returns a persistence error from the configured adapter.
    pub fn save(&self) -> Result<(), ApplicationError> {
        if let Some(repository) = &self.repository {
            repository
                .save(&self.state)
                .map_err(ApplicationError::Persistence)?;
        }
        Ok(())
    }

    /// Attempts to load a stored snapshot.
    ///
    /// # Errors
    ///
    /// Returns a persistence error from the configured adapter.
    pub fn load(&mut self) -> Result<bool, ApplicationError> {
        let Some(repository) = &self.repository else {
            return Ok(false);
        };
        let loaded = repository.load().map_err(ApplicationError::Persistence)?;
        if let Some(state) = loaded {
            self.restore(state.validated()?);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn apply(&mut self, command: GameCommand) -> Result<Vec<GameEvent>, ApplicationError> {
        let previous = self.state.clone();
        let events = self.state.apply(command)?;
        if let Some(repository) = &self.repository
            && let Err(error) = repository.save(&self.state)
        {
            // A command must never be reported as failed while remaining
            // applied: callers may safely retry after a persistence failure.
            self.state = previous;
            return Err(ApplicationError::Persistence(error));
        }
        if self.history.len() == MAX_UNDO_SNAPSHOTS {
            let _ = self.history.pop_front();
        }
        self.history.push_back(previous);
        self.replay.records.push(ReplayRecord {
            command,
            events: events.clone(),
        });
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    use ludo_domain::{Rules, TurnPhase, standard_players};

    use super::*;

    struct FixedDice(DiceValue);

    impl DiceSource for FixedDice {
        fn roll(&self) -> DiceValue {
            self.0
        }
    }

    #[derive(Default)]
    struct MemoryRepository(Mutex<Option<GameState>>);

    impl GameRepository for MemoryRepository {
        fn save(&self, state: &GameState) -> Result<(), String> {
            self.0
                .lock()
                .map_err(|_| "memory repository lock poisoned".to_owned())?
                .replace(state.clone());
            Ok(())
        }

        fn load(&self) -> Result<Option<GameState>, String> {
            self.0
                .lock()
                .map(|state| state.clone())
                .map_err(|_| "memory repository lock poisoned".to_owned())
        }
    }

    #[derive(Default)]
    struct ToggleRepository {
        fail: AtomicBool,
        state: Mutex<Option<GameState>>,
    }

    impl GameRepository for ToggleRepository {
        fn save(&self, state: &GameState) -> Result<(), String> {
            if self.fail.load(Ordering::Acquire) {
                return Err("injected write failure".to_owned());
            }
            self.state
                .lock()
                .map_err(|_| "toggle repository lock poisoned".to_owned())?
                .replace(state.clone());
            Ok(())
        }

        fn load(&self) -> Result<Option<GameState>, String> {
            self.state
                .lock()
                .map(|state| state.clone())
                .map_err(|_| "toggle repository lock poisoned".to_owned())
        }
    }

    #[test]
    fn roll_use_case_updates_and_saves() {
        let Ok(state) = GameState::new(standard_players(), Rules::default()) else {
            return;
        };
        let six = DiceValue::new(6).unwrap_or_else(|| std::process::abort());
        let repository = Arc::new(MemoryRepository::default());
        let mut session =
            GameSession::new(state, Arc::new(FixedDice(six))).with_repository(repository.clone());
        assert!(session.roll().is_ok());
        assert!(matches!(
            session.state().phase(),
            TurnPhase::AwaitingMove { .. }
        ));
        assert!(repository.load().is_ok_and(|stored| stored.is_some()));
    }

    #[test]
    fn undo_restores_exact_snapshot_and_updates_replay() {
        let Ok(state) = GameState::new(standard_players(), Rules::default()) else {
            return;
        };
        let initial = state.clone();
        let six = DiceValue::new(6).unwrap_or_else(|| std::process::abort());
        let mut session = GameSession::new(state, Arc::new(FixedDice(six)));
        assert!(session.roll().is_ok());
        assert_eq!(session.replay().records.len(), 1);
        assert!(session.undo().is_ok());
        assert_eq!(session.state(), &initial);
        assert!(session.replay().records.is_empty());
    }

    #[test]
    fn competitive_policy_rejects_undo() {
        let Ok(state) = GameState::new(standard_players(), Rules::default()) else {
            return;
        };
        let six = DiceValue::new(6).unwrap_or_else(|| std::process::abort());
        let mut session = GameSession::new(state, Arc::new(FixedDice(six)))
            .with_undo_policy(UndoPolicy::Competitive);
        assert!(session.roll().is_ok());
        assert!(matches!(
            session.undo(),
            Err(ApplicationError::UndoDisabled)
        ));
    }

    #[test]
    fn persistence_failure_rolls_back_the_entire_command_transaction() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let initial = state.clone();
        let six = DiceValue::new(6).unwrap_or_else(|| std::process::abort());
        let repository = Arc::new(ToggleRepository::default());
        repository.fail.store(true, Ordering::Release);
        let mut session =
            GameSession::new(state, Arc::new(FixedDice(six))).with_repository(repository);

        assert!(matches!(
            session.roll(),
            Err(ApplicationError::Persistence(_))
        ));
        assert_eq!(session.state(), &initial);
        assert!(session.replay().records.is_empty());
        assert!(!session.can_undo());
    }

    #[test]
    fn failed_undo_preserves_current_state_history_and_replay() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let six = DiceValue::new(6).unwrap_or_else(|| std::process::abort());
        let repository = Arc::new(ToggleRepository::default());
        let mut session =
            GameSession::new(state, Arc::new(FixedDice(six))).with_repository(repository.clone());
        assert!(session.roll().is_ok());
        let after_roll = session.state().clone();
        let replay = session.replay().clone();
        repository.fail.store(true, Ordering::Release);

        assert!(matches!(
            session.undo(),
            Err(ApplicationError::Persistence(_))
        ));
        assert_eq!(session.state(), &after_roll);
        assert_eq!(session.replay(), &replay);
        assert!(session.can_undo());
    }

    #[test]
    fn restore_starts_a_new_replay_and_clears_undo_history() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let six = DiceValue::new(6).unwrap_or_else(|| std::process::abort());
        let mut session = GameSession::new(state.clone(), Arc::new(FixedDice(six)));
        assert!(session.roll().is_ok());
        session.restore(state.clone());
        assert_eq!(session.state(), &state);
        assert_eq!(session.replay().initial_state, state);
        assert!(session.replay().records.is_empty());
        assert!(!session.can_undo());
    }
}
