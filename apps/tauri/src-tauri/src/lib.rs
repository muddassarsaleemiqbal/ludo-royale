use std::sync::Mutex;

use ludo_ai::{BotDecision, BotRequest, ParallelBot};
use ludo_application::DiceSource;
use ludo_domain::GameState;
use ludo_infrastructure::RandomDice;
use ludo_presentation::GameViewModel;
use ludo_runtime::{GameRuntime, RuntimeUpdate, UiAction};
use tauri::State;

struct RuntimeState(Mutex<GameRuntime>);

#[tauri::command]
fn snapshot(state: State<'_, RuntimeState>) -> Result<GameViewModel, String> {
    state
        .0
        .lock()
        .map(|runtime| runtime.model())
        .map_err(|_| "game runtime lock is unavailable".to_owned())
}

#[tauri::command]
fn state_json(state: State<'_, RuntimeState>) -> Result<String, String> {
    state
        .0
        .lock()
        .map_err(|_| "game runtime lock is unavailable".to_owned())
        .and_then(|runtime| {
            serde_json::to_string(runtime.state()).map_err(|error| error.to_string())
        })
}

#[tauri::command]
fn restore_state(
    snapshot: String,
    state: State<'_, RuntimeState>,
) -> Result<GameViewModel, String> {
    let restored: GameState = serde_json::from_str(&snapshot).map_err(|error| error.to_string())?;
    let restored = restored.validated().map_err(|error| error.to_string())?;
    let mut runtime = state
        .0
        .lock()
        .map_err(|_| "game runtime lock is unavailable".to_owned())?;
    *runtime = GameRuntime::from_state(restored);
    Ok(runtime.model())
}

#[tauri::command]
fn dispatch(action: UiAction, state: State<'_, RuntimeState>) -> Result<RuntimeUpdate, String> {
    state
        .0
        .lock()
        .map_err(|_| "game runtime lock is unavailable".to_owned())?
        .dispatch(action)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn random_dice() -> u8 {
    RandomDice.roll().get()
}

#[tauri::command]
async fn evaluate_bot(request: BotRequest) -> Result<BotDecision, String> {
    tauri::async_runtime::spawn_blocking(move || ParallelBot::choose(&request))
        .await
        .map_err(|error| error.to_string())
}

/// Starts the shared Tauri application on desktop or mobile.
pub fn run() {
    let result = tauri::Builder::default()
        .manage(RuntimeState(Mutex::new(GameRuntime::standard())))
        .invoke_handler(tauri::generate_handler![
            snapshot,
            state_json,
            restore_state,
            dispatch,
            random_dice,
            evaluate_bot
        ])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        eprintln!("failed to run Tauri application: {error}");
    }
}
