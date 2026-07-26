//! WebAssembly bindings for the portable Ludo runtime.

#[cfg(target_arch = "wasm32")]
mod wasm {
    use ludo_ai::{BotRequest, ParallelBot};
    use ludo_runtime::{GameRuntime, UiAction};
    use wasm_bindgen::prelude::*;

    /// Browser-owned portable game instance.
    #[wasm_bindgen]
    pub struct WasmGame {
        runtime: GameRuntime,
    }

    #[wasm_bindgen]
    impl WasmGame {
        /// Creates a standard local game.
        #[wasm_bindgen(constructor)]
        #[must_use]
        pub fn new() -> Self {
            Self {
                runtime: GameRuntime::standard(),
            }
        }

        /// Returns the current framework-neutral snapshot as JSON.
        ///
        /// # Errors
        ///
        /// Returns a JavaScript error if serialization fails.
        pub fn snapshot_json(&self) -> Result<String, JsValue> {
            serde_json::to_string(&self.runtime.model()).map_err(js_error)
        }

        /// Dispatches one JSON action and returns the complete update as JSON.
        ///
        /// # Errors
        ///
        /// Returns a JavaScript error for invalid JSON or unavailable actions.
        pub fn dispatch_json(&mut self, action: &str) -> Result<String, JsValue> {
            let action: UiAction = serde_json::from_str(action).map_err(js_error)?;
            let update = self.runtime.dispatch(action).map_err(js_error)?;
            serde_json::to_string(&update).map_err(js_error)
        }
    }

    /// Evaluates a bot request. The browser calls this from a dedicated worker.
    ///
    /// # Errors
    ///
    /// Returns a JavaScript error for malformed input or serialization failure.
    #[wasm_bindgen]
    pub fn evaluate_bot_json(request: &str) -> Result<String, JsValue> {
        let request: BotRequest = serde_json::from_str(request).map_err(js_error)?;
        serde_json::to_string(&ParallelBot::choose(&request)).map_err(js_error)
    }

    fn js_error(error: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&error.to_string())
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::*;
