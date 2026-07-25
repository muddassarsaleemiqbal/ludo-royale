# Architecture

The dependency rule is inward-only:

```text
ludo-desktop ──> ludo-presentation ──> ludo-application ──> ludo-domain
       │                                      ▲
       ├──> ludo-ai ──────────────────────────┤
       ├──> ludo-network ──────────────────────┤
       └──> ludo-infrastructure ──────────────┘
```

## Crates

- `ludo-domain`: deterministic state machine, entities, commands, events,
  validated versioned snapshots, presets, blockades, rankings, and rules. It has
  no GUI, threading, random-number, filesystem, or audio dependency.
- `ludo-application`: use cases and the `DiceSource`, `GameRepository`, and
  `SoundPlayer` ports.
- `ludo-presentation`: framework-neutral view models and deterministic
  event-to-animation timelines.
- `ludo-ai`: immutable snapshot evaluation on Rayon, plus a bounded worker
  adapter that tags decisions with their source revision. Hard AI adds parallel
  bounded Monte Carlo rollouts, threat/exposure scoring, blockades, and opponent
  modelling.
- `ludo-network`: versioned, size-bounded newline-delimited JSON protocol,
  authoritative lobby/game state, private codes, reconnect tokens, independent
  game/lobby revision checks, TCP host/client adapters, and mDNS/DNS-SD
  discovery.
- `ludo-infrastructure`: random dice, versioned atomic JSON persistence, and a
  non-blocking synthesized-audio adapter. Save serialization and disk writes run
  on a dedicated coalescing worker.
- `ludo-desktop`: GPUI Component composition root and native desktop interface.
- `ludo-simulation`: parallel, seeded, headless match stress runner.

## Runtime threading

GPUI owns the main thread and performs only input handling, view-model rendering,
and animation scheduling. Bot requests use a one-snapshot bounded queue. One
dispatcher evaluates at a time while candidate scoring, future-roll analysis,
and bounded Monte Carlo rollouts use Rayon's shared work-stealing pool. This
prevents unbounded outer tasks and nested-pool oversubscription. Decisions are
applied only when their game revision and player still match.

Autosave requests enqueue immutable snapshots and return immediately. The
single-slot queue retains only the latest pending state, and the save worker
writes a temporary file before atomically renaming it. Save deletion is an
ordered, acknowledged worker command, so an older in-flight save cannot recreate
a deleted game. Malformed saves are quarantined instead of entering the
application.

Sound cues enqueue onto a dedicated Rodio worker. Domain events are converted to
semantic animation frames by the presentation crate; GPUI only schedules and
renders those frames. Reduced-motion mode applies the same final domain state
without timing delays.

Replay files store an initial validated snapshot plus every deterministic command
and resulting event list. Loading verifies the complete stream against the domain
engine. Undo stores bounded in-memory command snapshots and is disabled by policy
for competitive modes. Profiles and named rule presets use application ports with
atomic JSON adapters.

The LAN host is the sole dice and state authority. Clients submit intent against
an exact revision; accepted responses include the canonical command, events, and
snapshot. Stale requests are rejected with synchronization state. Lobby presence
is transported beside game snapshots: the host occupies the first human seat,
remaining seats begin as computers, and host-approved named remote players
atomically replace computer seats. DNS-SD discovers an address and public lobby
metadata so nearby players can request access without seeing a secret. Each
request has a client-generated opaque ID, expires if unanswered, and becomes a
seat only after the host accepts it. Approval decisions remain retryable for a
bounded period, so a lost response cannot strand an accepted seat. Idle sync
uses a small revision-only heartbeat; full snapshots are sent only after game or
lobby changes. Frames, pending joins, retained decisions, and simultaneous
connections are all bounded. Per-connection generations prevent an old socket
closing from marking a newer reconnection offline. The private room code remains
a discovery fallback; the issued reconnect token authenticates a claimed seat.

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
