# Ludo

A fast, native Ludo game written entirely in Rust. The deterministic rules engine is
independent of the GPUI desktop interface, and computer players evaluate moves in
parallel with Rayon.

## Included

- Configurable 2–4 player matches with editable names and colors.
- Any mix of local humans and Easy, Medium, or Hard bots.
- Classic, Quick, and Tournament rule presets.
- Blockades, configurable safe cells, exact-home rules, turn bonuses, and
  multi-player placement rankings.
- Privacy-safe hot-seat transitions.
- Responsive native GPUI Component interface with Royale, Classic, and
  Midnight themes.
- Event-driven dice and token animation with a reduced-motion mode.
- Pause, destructive-action confirmation, rules/onboarding, and final
  standings screens.
- High-contrast token labels and synthesized, optional sound effects.
- Versioned command/event replays with play, pause, seek, speed, and JSON files.
- Confirmed local undo with competitive-mode restrictions.
- Persistent profiles, match history, detailed statistics, streaks, and
  achievements.
- Validated named custom rules with JSON import/export.
- Threat-aware, blockade-aware, opponent-modeling AI with configurable
  parallel Monte Carlo work budgets.
- True 2v2 ally rules, round-robin leagues, and elimination brackets.
- Zero-configuration LAN discovery with host-approved named join requests,
  room-code fallback, synchronized lobbies, connection health, authoritative
  play, retry-safe approvals, bounded protocol frames, lightweight revision
  heartbeats, and race-safe reconnection tokens.
- Versioned, coalescing background autosave and resume with ordered deletion and
  corrupt-save quarantine.
- Encapsulated, validated snapshots and property-tested domain invariants.
- Rust 2024 on Rust 1.97.1, current GPUI Component 0.5.1, Rand 0.10, strict
  workspace Clippy lints, thin LTO, and single-codegen-unit release builds.
- Branded native macOS and Windows icons, signed-distribution hooks, universal
  DMG, NSIS, and MSI packaging, release checksums, and a CycloneDX SBOM.

## Run

```sh
cargo run -p ludo-desktop --release
```

Run a headless stress simulation:

```sh
cargo run -p ludo-simulation --release -- 10000
```

Run the AI performance benchmark:

```sh
cargo run -p ludo-simulation --release -- --ai-bench 50
```

## Quality checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
```

GitHub Actions runs formatting, compiler, Clippy, test, documentation, Windows,
and dependency-security checks for every push and pull request. Once a
`main`-branch push passes, the release workflow increments the patch version
and publishes macOS and Windows installers.

See [the distribution guide](docs/DISTRIBUTION.md) for signing secrets,
repository settings, release behavior, and local packaging commands.
