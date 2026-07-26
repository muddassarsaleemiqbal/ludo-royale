//! GUI-independent Ludo use cases and infrastructure ports.

use std::sync::Arc;

use ludo_domain::{DiceValue, DomainError, GameCommand, GameEvent, GameState, TokenId};
use thiserror::Error;

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
}

/// Stateful application facade used by any presentation technology.
pub struct GameSession {
    state: GameState,
    repository: Option<Arc<dyn GameRepository>>,
}

impl GameSession {
    /// Creates a session around a new or restored match.
    #[must_use]
    pub const fn new(state: GameState) -> Self {
        Self {
            state,
            repository: None,
        }
    }

    /// Installs an optional persistence adapter.
    #[must_use]
    pub fn with_repository(mut self, repository: Arc<dyn GameRepository>) -> Self {
        self.repository = Some(repository);
        self
    }

    /// Current immutable snapshot.
    #[must_use]
    pub const fn state(&self) -> &GameState {
        &self.state
    }

    /// Replaces the current state with a restored snapshot.
    pub fn restore(&mut self, state: GameState) {
        self.state = state;
    }

    /// Executes the roll-dice use case.
    ///
    /// # Errors
    ///
    /// Returns a domain error when the current phase does not allow rolling.
    pub fn roll(&mut self, value: DiceValue) -> Result<Vec<GameEvent>, ApplicationError> {
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
            self.restore(state);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn apply(&mut self, command: GameCommand) -> Result<Vec<GameEvent>, ApplicationError> {
        let events = self.state.apply(command)?;
        if let Some(repository) = &self.repository {
            repository
                .save(&self.state)
                .map_err(ApplicationError::Persistence)?;
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ludo_domain::{Rules, TurnPhase, standard_players};

    use super::*;

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

    #[test]
    fn roll_use_case_updates_and_saves() {
        let Ok(state) = GameState::new(standard_players(), Rules::default()) else {
            return;
        };
        let six = DiceValue::new(6).unwrap_or_else(|| std::process::abort());
        let repository = Arc::new(MemoryRepository::default());
        let mut session = GameSession::new(state).with_repository(repository.clone());
        assert!(session.roll(six).is_ok());
        assert!(matches!(
            session.state().phase,
            TurnPhase::AwaitingMove { .. }
        ));
        assert!(repository.load().is_ok_and(|stored| stored.is_some()));
    }
}
