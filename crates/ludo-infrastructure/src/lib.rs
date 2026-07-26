//! Concrete outer-layer adapters.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use flume::{Receiver, Sender, TrySendError};
use ludo_application::{
    DiceSource, GameRepository, SoundCue, SoundPlayer,
    profiles::{PROFILE_SCHEMA_VERSION, ProfileBook, ProfileRepository},
    replay::{Replay, ReplayRepository},
    rule_presets::{NamedRulePreset, RulePresetRepository},
};
use ludo_domain::{DiceValue, GameState};
use rand::RngExt;
use rodio::Source;
use serde::{Deserialize, Serialize};

const SAVE_FORMAT_VERSION: u16 = 1;

/// Thread-local, operating-system-seeded dice source.
#[derive(Debug, Default)]
pub struct RandomDice;

impl DiceSource for RandomDice {
    fn roll(&self) -> DiceValue {
        let value = rand::rng().random_range(1..=6);
        DiceValue::new(value).unwrap_or_else(|| std::process::abort())
    }
}

/// Synthesized, asset-free sound effects on a dedicated audio thread.
#[derive(Debug, Clone)]
pub struct AudioWorker {
    sender: Sender<SoundCue>,
    enabled: Arc<AtomicBool>,
}

impl AudioWorker {
    /// Starts the audio worker. Lack of an output device degrades silently.
    #[must_use]
    pub fn new() -> Self {
        // Sound is ephemeral. A bounded queue prevents an inactive audio
        // device or burst of animation events from retaining memory forever.
        let (sender, receiver) = flume::bounded(32);
        let enabled = Arc::new(AtomicBool::new(true));
        let worker_enabled = enabled.clone();
        let _ = std::thread::Builder::new()
            .name("ludo-audio-worker".to_owned())
            .spawn(move || {
                let Ok(stream) = rodio::DeviceSinkBuilder::open_default_sink() else {
                    while receiver.recv().is_ok() {}
                    return;
                };
                let player = rodio::Player::connect_new(stream.mixer());
                while let Ok(cue) = receiver.recv() {
                    if worker_enabled.load(Ordering::Acquire) {
                        append_cue(&player, cue);
                    }
                }
            });
        Self { sender, enabled }
    }
}

impl Default for AudioWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundPlayer for AudioWorker {
    fn play(&self, cue: SoundCue) {
        let _ = self.sender.try_send(cue);
    }

    fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
    }
}

fn append_cue(player: &rodio::Player, cue: SoundCue) {
    let tones: &[(f32, u64, f32)] = match cue {
        SoundCue::Dice => &[(240.0, 45, 0.08), (320.0, 45, 0.08), (410.0, 55, 0.09)],
        SoundCue::Move => &[(520.0, 42, 0.06)],
        SoundCue::Capture => &[(220.0, 90, 0.10), (130.0, 130, 0.09)],
        SoundCue::Home => &[(520.0, 70, 0.08), (700.0, 120, 0.10)],
        SoundCue::Turn => &[(390.0, 70, 0.06)],
        SoundCue::Victory => &[(440.0, 100, 0.09), (554.0, 100, 0.09), (659.0, 220, 0.11)],
    };
    for (frequency, milliseconds, volume) in tones {
        player.append(
            rodio::source::SineWave::new(*frequency)
                .take_duration(Duration::from_millis(*milliseconds))
                .amplify(*volume),
        );
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SaveEnvelope {
    format_version: u16,
    game: GameState,
}

#[derive(Serialize)]
struct SaveEnvelopeRef<'a> {
    format_version: u16,
    game: &'a GameState,
}

#[derive(Debug)]
enum SaveWorkerRequest {
    Save {
        generation: u64,
        state: GameState,
    },
    Delete {
        generation: u64,
        response: Sender<Result<(), String>>,
    },
}

/// Versioned JSON repository that serializes and writes snapshots on a
/// dedicated worker thread.
#[derive(Debug, Clone)]
pub struct BackgroundGameRepository {
    path: PathBuf,
    sender: Sender<SaveWorkerRequest>,
    pending: Receiver<SaveWorkerRequest>,
    enqueue_lock: Arc<Mutex<()>>,
    enqueued_generation: Arc<AtomicU64>,
    saved_generation: Arc<AtomicU64>,
}

impl BackgroundGameRepository {
    /// Starts a persistence worker for an explicit path.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let worker_path = path.clone();
        // Only the newest not-yet-started snapshot matters. This keeps rapid
        // moves from building an unbounded queue behind slow storage.
        let (sender, receiver) = flume::bounded::<SaveWorkerRequest>(1);
        let pending = receiver.clone();
        let enqueued_generation = Arc::new(AtomicU64::new(0));
        let saved_generation = Arc::new(AtomicU64::new(0));
        let worker_generation = saved_generation.clone();
        let _ = std::thread::Builder::new()
            .name("ludo-save-worker".to_owned())
            .spawn(move || {
                let mut deferred = None;
                loop {
                    let Ok(request) = deferred.take().map_or_else(|| receiver.recv(), Ok) else {
                        break;
                    };
                    match request {
                        SaveWorkerRequest::Save {
                            mut generation,
                            mut state,
                        } => {
                            while let Ok(newer) = receiver.try_recv() {
                                match newer {
                                    SaveWorkerRequest::Save {
                                        generation: newer_generation,
                                        state: newer_state,
                                    } => {
                                        generation = newer_generation;
                                        state = newer_state;
                                    }
                                    delete @ SaveWorkerRequest::Delete { .. } => {
                                        // Deletion supersedes every preceding
                                        // snapshot, including this one.
                                        deferred = Some(delete);
                                        break;
                                    }
                                }
                            }
                            if deferred.is_none() && write_snapshot(&worker_path, &state).is_ok() {
                                worker_generation.store(generation, Ordering::Release);
                            }
                        }
                        SaveWorkerRequest::Delete {
                            generation,
                            response,
                        } => {
                            let result = remove_if_present(&worker_path);
                            if result.is_ok() {
                                worker_generation.store(generation, Ordering::Release);
                            }
                            let _ = response.send(result);
                        }
                    }
                }
            });
        Self {
            path,
            sender,
            pending,
            enqueue_lock: Arc::new(Mutex::new(())),
            enqueued_generation,
            saved_generation,
        }
    }

    /// Configured save-file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns whether a save is available.
    #[must_use]
    pub fn exists(&self) -> bool {
        self.path.is_file()
    }

    /// Waits for the worker to persist at least the requested revision.
    ///
    /// This is intended for orderly shutdown and tests, not normal UI updates.
    #[must_use]
    pub fn flush(&self, _revision: u64, timeout: Duration) -> bool {
        let target = self.enqueued_generation.load(Ordering::Acquire);
        let deadline = Instant::now() + timeout;
        loop {
            if self.saved_generation.load(Ordering::Acquire) >= target {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Removes the active save.
    ///
    /// # Errors
    ///
    /// Returns an I/O message when deletion fails for a reason other than a
    /// missing file.
    pub fn delete(&self) -> Result<(), String> {
        let _guard = self
            .enqueue_lock
            .lock()
            .map_err(|_| "save queue lock was poisoned".to_owned())?;
        while self.pending.try_recv().is_ok() {}
        let generation = self
            .enqueued_generation
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        let (response_sender, response_receiver) = flume::bounded(1);
        self.sender
            .send(SaveWorkerRequest::Delete {
                generation,
                response: response_sender,
            })
            .map_err(|_| "save worker is unavailable".to_owned())?;
        response_receiver
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "save worker did not confirm deletion".to_owned())?
    }
}

impl GameRepository for BackgroundGameRepository {
    fn save(&self, state: &GameState) -> Result<(), String> {
        let _guard = self
            .enqueue_lock
            .lock()
            .map_err(|_| "save queue lock was poisoned".to_owned())?;
        let generation = self
            .enqueued_generation
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        let request = SaveWorkerRequest::Save {
            generation,
            state: state.clone(),
        };
        match self.sender.try_send(request) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(request)) => {
                let _ = self.pending.try_recv();
                self.sender
                    .try_send(request)
                    .map_err(|_| "save worker is unavailable".to_owned())
            }
            Err(TrySendError::Disconnected(_)) => Err("save worker is unavailable".to_owned()),
        }
    }

    fn load(&self) -> Result<Option<GameState>, String> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        let envelope: SaveEnvelope = match serde_json::from_slice(&bytes) {
            Ok(envelope) => envelope,
            Err(error) => {
                let recovered = quarantine_path(&self.path);
                let _ = fs::rename(&self.path, &recovered);
                return Err(format!(
                    "save was corrupt and moved to {}: {error}",
                    recovered.display()
                ));
            }
        };
        if envelope.format_version != SAVE_FORMAT_VERSION {
            return Err(format!(
                "save format {} is not supported by format {}",
                envelope.format_version, SAVE_FORMAT_VERSION
            ));
        }
        envelope
            .game
            .validated()
            .map(Some)
            .map_err(|error| error.to_string())
    }
}

fn remove_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn write_snapshot(path: &Path, state: &GameState) -> Result<(), String> {
    let envelope = SaveEnvelopeRef {
        format_version: SAVE_FORMAT_VERSION,
        game: state,
    };
    let bytes = serde_json::to_vec(&envelope).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

fn quarantine_path(path: &Path) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    path.with_extension(format!("corrupt-{timestamp}.json"))
}

/// Versioned JSON replay file adapter.
#[derive(Debug, Clone)]
pub struct ReplayFileRepository {
    path: PathBuf,
}

impl ReplayFileRepository {
    /// Uses an explicit replay file.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl ReplayRepository for ReplayFileRepository {
    fn save_replay(&self, replay: &Replay) -> Result<(), String> {
        replay.validate().map_err(|error| error.to_string())?;
        write_json(&self.path, replay)
    }

    fn load_replay(&self) -> Result<Option<Replay>, String> {
        let Some(replay) = read_json::<Replay>(&self.path)? else {
            return Ok(None);
        };
        replay.validate().map_err(|error| error.to_string())?;
        Ok(Some(replay))
    }
}

/// Atomic JSON player-profile adapter.
#[derive(Debug, Clone)]
pub struct JsonProfileRepository {
    path: PathBuf,
}

impl JsonProfileRepository {
    /// Uses an explicit profile database file.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl ProfileRepository for JsonProfileRepository {
    fn load_profiles(&self) -> Result<ProfileBook, String> {
        let book = read_json::<ProfileBook>(&self.path)?.unwrap_or_default();
        if book.schema_version != PROFILE_SCHEMA_VERSION {
            return Err(format!(
                "profile schema {} is not supported",
                book.schema_version
            ));
        }
        Ok(book)
    }

    fn save_profiles(&self, profiles: &ProfileBook) -> Result<(), String> {
        if profiles.schema_version != PROFILE_SCHEMA_VERSION {
            return Err("refusing to save an unsupported profile schema".to_owned());
        }
        write_json(&self.path, profiles)
    }
}

/// JSON storage plus portable import/export for named rule presets.
#[derive(Debug, Clone)]
pub struct JsonRulePresetRepository {
    collection_path: PathBuf,
    exchange_path: PathBuf,
}

impl JsonRulePresetRepository {
    /// Uses explicit collection and import/export files.
    #[must_use]
    pub fn new(collection_path: impl Into<PathBuf>, exchange_path: impl Into<PathBuf>) -> Self {
        Self {
            collection_path: collection_path.into(),
            exchange_path: exchange_path.into(),
        }
    }
}

impl RulePresetRepository for JsonRulePresetRepository {
    fn load_rule_presets(&self) -> Result<Vec<NamedRulePreset>, String> {
        read_json::<Vec<NamedRulePreset>>(&self.collection_path)?
            .unwrap_or_default()
            .into_iter()
            .map(|preset| preset.validated().map_err(|error| error.to_string()))
            .collect()
    }

    fn save_rule_presets(&self, presets: &[NamedRulePreset]) -> Result<(), String> {
        for preset in presets {
            preset
                .clone()
                .validated()
                .map_err(|error| error.to_string())?;
        }
        write_json(&self.collection_path, presets)
    }

    fn export_rule_preset(&self, preset: &NamedRulePreset) -> Result<(), String> {
        let preset = preset
            .clone()
            .validated()
            .map_err(|error| error.to_string())?;
        write_json(&self.exchange_path, &preset)
    }

    fn import_rule_preset(&self) -> Result<Option<NamedRulePreset>, String> {
        read_json::<NamedRulePreset>(&self.exchange_path)?
            .map(NamedRulePreset::validated)
            .transpose()
            .map_err(|error| error.to_string())
    }
}

fn write_json(path: &Path, value: &(impl Serialize + ?Sized)) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ludo_application::{
        GameRepository, GameSession,
        profiles::{ProfileBook, ProfileRepository},
        replay::{Replay, ReplayRepository},
        rule_presets::{NamedRulePreset, RulePresetRepository},
    };
    use ludo_domain::{Controller, GameState, PlayerId, RulePreset, standard_players};

    use super::*;

    #[test]
    fn versioned_snapshot_round_trips() {
        let path = std::env::temp_dir().join(format!("ludo-save-test-{}.json", std::process::id()));
        let repository = Arc::new(BackgroundGameRepository::new(&path));
        let state = GameState::new(standard_players(), RulePreset::Classic.rules())
            .unwrap_or_else(|_| std::process::abort());
        assert!(repository.save(&state).is_ok());
        assert!(repository.flush(state.revision(), Duration::from_secs(1)));
        assert!(repository.load().is_ok_and(|loaded| loaded == Some(state)));
        assert!(repository.delete().is_ok());
    }

    #[test]
    fn repository_satisfies_application_port() {
        let path =
            std::env::temp_dir().join(format!("ludo-session-test-{}.json", std::process::id()));
        let repository = Arc::new(BackgroundGameRepository::new(&path));
        let state = GameState::new(standard_players(), RulePreset::Classic.rules())
            .unwrap_or_else(|_| std::process::abort());
        let session =
            GameSession::new(state, Arc::new(RandomDice)).with_repository(repository.clone());
        assert!(session.save().is_ok());
        assert!(repository.flush(session.state().revision(), Duration::from_secs(1)));
        assert!(repository.delete().is_ok());
    }

    #[test]
    fn corrupt_save_is_quarantined() {
        let path =
            std::env::temp_dir().join(format!("ludo-corrupt-test-{}.json", std::process::id()));
        assert!(fs::write(&path, b"not valid JSON").is_ok());
        let quarantined = quarantine_path(&path);
        let repository = BackgroundGameRepository::new(&path);
        assert!(repository.load().is_err());
        assert!(!repository.exists());
        assert!(quarantined.exists());
        assert!(fs::remove_file(quarantined).is_ok());
    }

    #[test]
    fn p2_json_adapters_round_trip() {
        let root = std::env::temp_dir().join(format!("ludo-p2-test-{}", std::process::id()));
        assert!(fs::create_dir_all(&root).is_ok());
        let state = GameState::new(standard_players(), RulePreset::Classic.rules())
            .unwrap_or_else(|_| std::process::abort());

        let replay_path = root.join("match.json");
        let replay_repository = ReplayFileRepository::new(&replay_path);
        let replay = Replay::new(state);
        assert!(replay_repository.save_replay(&replay).is_ok());
        assert!(
            replay_repository
                .load_replay()
                .is_ok_and(|loaded| loaded == Some(replay))
        );

        let profile_path = root.join("profiles.json");
        let profiles = JsonProfileRepository::new(&profile_path);
        assert!(profiles.save_profiles(&ProfileBook::default()).is_ok());
        assert!(
            profiles
                .load_profiles()
                .is_ok_and(|book| book == ProfileBook::default())
        );

        let collection = root.join("rules.json");
        let exchange = root.join("exchange.json");
        let rules = JsonRulePresetRepository::new(&collection, &exchange);
        let preset = NamedRulePreset::new("Fast", RulePreset::Quick.rules())
            .unwrap_or_else(|_| std::process::abort());
        assert!(
            rules
                .save_rule_presets(std::slice::from_ref(&preset))
                .is_ok()
        );
        assert!(rules.export_rule_preset(&preset).is_ok());
        assert!(
            rules
                .import_rule_preset()
                .is_ok_and(|loaded| loaded == Some(preset))
        );

        assert!(fs::remove_dir_all(root).is_ok());
    }

    #[test]
    fn rapid_same_revision_saves_are_coalesced_to_the_latest_snapshot() {
        let path =
            std::env::temp_dir().join(format!("ludo-coalesce-test-{}.json", std::process::id()));
        let repository = BackgroundGameRepository::new(&path);
        let mut state = GameState::new(standard_players(), RulePreset::Classic.rules())
            .unwrap_or_else(|_| std::process::abort());
        let player = PlayerId::new(0).unwrap_or_else(|| std::process::abort());
        for index in 0..100 {
            assert!(
                state
                    .update_player_control(player, format!("Host {index}"), Controller::Human)
                    .is_ok()
            );
            assert!(repository.save(&state).is_ok());
        }
        assert!(repository.flush(state.revision(), Duration::from_secs(2)));
        assert!(repository.load().is_ok_and(|loaded| loaded == Some(state)));
        assert!(repository.delete().is_ok());
    }

    #[test]
    fn confirmed_delete_cannot_be_undone_by_an_older_queued_save() {
        let path =
            std::env::temp_dir().join(format!("ludo-delete-race-test-{}.json", std::process::id()));
        let repository = BackgroundGameRepository::new(&path);
        let state = GameState::new(standard_players(), RulePreset::Classic.rules())
            .unwrap_or_else(|_| std::process::abort());
        for _ in 0..100 {
            assert!(repository.save(&state).is_ok());
        }
        assert!(repository.delete().is_ok());
        assert!(!repository.exists());
        std::thread::sleep(Duration::from_millis(25));
        assert!(!repository.exists());
    }

    #[test]
    fn unsupported_save_versions_are_rejected() {
        let path =
            std::env::temp_dir().join(format!("ludo-version-test-{}.json", std::process::id()));
        let state = GameState::new(standard_players(), RulePreset::Classic.rules())
            .unwrap_or_else(|_| std::process::abort());
        let bytes = serde_json::to_vec(&serde_json::json!({
            "format_version": SAVE_FORMAT_VERSION.saturating_add(1),
            "game": state,
        }))
        .unwrap_or_else(|_| std::process::abort());
        assert!(fs::write(&path, bytes).is_ok());
        let repository = BackgroundGameRepository::new(&path);
        assert!(
            repository
                .load()
                .is_err_and(|error| error.contains("not supported"))
        );
        assert!(repository.delete().is_ok());
    }

    #[test]
    fn missing_optional_json_files_load_as_empty_values() {
        let root =
            std::env::temp_dir().join(format!("ludo-missing-adapter-test-{}", std::process::id()));
        let profiles = JsonProfileRepository::new(root.join("profiles.json"));
        let replay = ReplayFileRepository::new(root.join("replay.json"));
        let rules =
            JsonRulePresetRepository::new(root.join("rules.json"), root.join("exchange.json"));
        assert!(
            profiles
                .load_profiles()
                .is_ok_and(|book| book == ProfileBook::default())
        );
        assert!(replay.load_replay().is_ok_and(|value| value.is_none()));
        assert!(
            rules
                .load_rule_presets()
                .is_ok_and(|presets| presets.is_empty())
        );
        assert!(
            rules
                .import_rule_preset()
                .is_ok_and(|preset| preset.is_none())
        );
    }
}
