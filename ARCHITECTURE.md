# Architecture

The dependency rule is inward-only:

```text
ludo-desktop ──> ludo-presentation ──> ludo-application ──> ludo-domain
       │                                      ▲
       ├──> ludo-ai ──────────────────────────┤
       └──> ludo-infrastructure ──────────────┘
```

## Crates

- `ludo-domain`: deterministic state machine, entities, commands, events, and
  rules. It has no GUI, threading, random-number, filesystem, or audio dependency.
- `ludo-application`: use cases and the `DiceSource`/`GameRepository` ports.
- `ludo-presentation`: framework-neutral view models.
- `ludo-ai`: immutable snapshot evaluation on Rayon, plus a bounded worker
  adapter that tags decisions with their source revision.
- `ludo-infrastructure`: random dice and atomic JSON persistence adapters.
- `ludo-desktop`: GPUI Component composition root and native desktop interface.
- `ludo-simulation`: parallel, seeded, headless match stress runner.

## Runtime threading

GPUI owns the main thread and performs only input handling, view-model rendering,
and animation scheduling. Bot requests are sent through a bounded channel to a
dispatcher; independent candidate evaluations execute on Rayon's global pool.
Decisions are applied only when their game revision and player still match.

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
