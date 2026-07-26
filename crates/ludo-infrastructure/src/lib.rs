//! Concrete outer-layer adapters.

use std::{
    fs,
    path::{Path, PathBuf},
};

use ludo_application::GameRepository;
use ludo_domain::{DiceValue, GameState};
use rand::Rng;

/// Thread-local, operating-system-seeded dice source.
#[derive(Debug, Default)]
pub struct RandomDice;

impl RandomDice {
    /// Generates one valid native random roll.
    #[must_use]
    pub fn roll(&self) -> DiceValue {
        let value = rand::rng().random_range(1..=6);
        DiceValue::new(value).unwrap_or_else(|| std::process::abort())
    }
}

/// Human-readable JSON snapshot repository.
#[derive(Debug, Clone)]
pub struct JsonGameRepository {
    path: PathBuf,
}

impl JsonGameRepository {
    /// Creates a repository at an explicit path.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Configured file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl GameRepository for JsonGameRepository {
    fn save(&self, state: &GameState) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
        let temporary = self.path.with_extension("tmp");
        fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
        fs::rename(&temporary, &self.path).map_err(|error| error.to_string())
    }

    fn load(&self) -> Result<Option<GameState>, String> {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| error.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }
}
