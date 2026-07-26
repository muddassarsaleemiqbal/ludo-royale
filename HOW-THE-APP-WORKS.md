# How the app is structured

This project is a cross-platform Ludo game with a shared Rust core and a shared React/TypeScript interface. The main idea is to keep the game rules deterministic and platform-independent in Rust, while letting the UI adapt to the browser or desktop runtime.

## High-level shape

- The game engine lives in Rust crates under the `crates/` directory.
- The user interface lives in the web client under `apps/client`.
- The desktop app uses the Tauri shell in `apps/tauri`.
- The web build runs the Rust engine in WebAssembly, while the desktop build runs it natively.

## Repository layout

### Frontend

- `apps/client`: the main React/Vite app, including the board UI, game controls, styling, and client-side state.
- `apps/tauri`: the desktop wrapper for macOS, Windows, and Linux. It hosts the same UI, but with native Rust integration.

### Core game engine

- `crates/ludo-domain`: deterministic game rules, entities, state transitions, and domain logic.
- `crates/ludo-application`: use cases and application-level orchestration.
- `crates/ludo-presentation`: view models and presentation-facing abstractions.
- `crates/ludo-runtime`: the action/model/effect runtime that coordinates the game flow.
- `crates/ludo-ai`: AI evaluation logic and platform-specific adapters.
- `crates/ludo-infrastructure`: persistence and randomization adapters.
- `crates/ludo-web`: browser bindings for WebAssembly.
- `crates/ludo-simulation`: headless stress and simulation tooling.
- `crates/ludo-network` and `crates/ludo-server`: networking and server-side pieces for future multiplayer support.

## How it works on different platforms

### Web

The web version compiles the Rust core to WebAssembly. The browser loads the Wasm module and runs the game logic inside the page. AI work is moved to a Web Worker so the UI stays responsive while the engine evaluates moves.

This is the best option for quick access and easy distribution through a browser.

### Desktop

The desktop version uses the same React UI, but the game runtime runs natively in Rust through Tauri. This allows the app to use native threads and avoid the browser sandbox for some workloads, especially AI evaluation.

This is the best option when you want a more native experience on macOS, Windows, or Linux.

### Future multiplayer

The project already includes networking and server crates, which suggests a path toward online or authoritative multiplayer. At the moment, the main experience is still local or single-player-focused, but those pieces are already being prepared for broader transport support.

## Runtime flow

1. The UI sends a player action or command.
2. The runtime processes that action against the game state.
3. Platform-specific effects such as dice rolls, AI evaluation, or persistence are executed.
4. The updated state is returned to the UI and rendered.

This keeps the rules engine predictable while letting the platform layer decide how effects should be performed.

## Development workflow

### Web development

```sh
sh scripts/build-web.sh
pnpm --dir apps/client dev
```

### Desktop development

```sh
pnpm --dir apps/tauri install
pnpm --dir apps/tauri tauri dev
```

### Simulation / stress testing

```sh
cargo run -p ludo-simulation --release -- 10000
```

## Why the architecture looks this way

The project is designed to share as much logic as possible across platforms:

- the same rules engine can run in the browser or on the desktop;
- the UI can be reused across environments;
- platform-specific code is isolated to adapters and hosts instead of being mixed into the core game logic.

That makes the app easier to extend, test, and port while keeping behavior consistent.
