# Architecture

The dependency rule is inward-only:

```text
React UI -> GameClient -> ludo-runtime -> ludo-application -> ludo-domain
                    │           │
                    │           └-> ludo-presentation
                    ├-> browser WASM + Worker AI
                    └-> Tauri native host + Rayon AI
```

The current delivery boundary is an action/model/effect runtime:

```text
WasmLocalGameClient ───┐
TauriNativeGameClient ─┼──> React store -> shared React component tree
OnlineGameClient* ─────┘

* future authoritative multiplayer transport
```

`ludo-runtime`, `ludo-presentation`, `ludo-application`, `ludo-domain`, and the
pure AI evaluator compile for `wasm32-unknown-unknown`. Platform adapters execute
effects using native timers/randomness/Rayon or browser timers/crypto/Web
Workers. Native adapters use typed Rust calls internally; serialization occurs
only across WebAssembly, worker, or webview boundaries.

## Crates

- `ludo-domain`: deterministic state machine, entities, commands, events, and
  rules. It has no GUI, threading, random-number, filesystem, or audio dependency.
- `ludo-application`: use cases and the `GameRepository` port.
- `ludo-presentation`: framework-neutral view models.
- `ludo-ai`: immutable snapshot evaluation on Rayon, plus a bounded worker
  adapter that tags decisions with their source revision.
- `ludo-infrastructure`: random dice and atomic JSON persistence adapters.
- `ludo-runtime`: platform-neutral action/model/effect orchestration.
- `ludo-web`: WebAssembly bindings for the browser transport.
- `apps/client`: shared React, TypeScript, Vite, and shadcn-style interface.
- `apps/tauri`: native desktop host for the shared interface.
- `ludo-simulation`: parallel, seeded, headless match stress runner.

## Runtime threading

The browser keeps AI work off the UI thread in a Web Worker. Tauri evaluates AI
on the native runtime using Rayon. Both transports apply decisions only when
their effect ID, game revision, and current player still match.

The domain remains synchronous because a normal rules transition is substantially
smaller than the cost of scheduling work on another thread.

## Definition of done

```sh
cargo fmt --all --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
```
