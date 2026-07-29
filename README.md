# Ludo

A fast cross-platform Ludo game with a deterministic Rust engine and one
React/TypeScript interface. Browsers run the core in WebAssembly; Tauri desktop
builds run the same core natively with parallel Rayon AI.

## Web development

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.126 --locked
sh scripts/build-web.sh
pnpm --dir apps/client dev
```

Open `http://127.0.0.1:5173`.

The production web build is written to `apps/client/dist`. Game rules and
orchestration run in WebAssembly; AI evaluation runs in a dedicated Web Worker.

## Native desktop

Tauri uses the same React UI while running the game runtime, randomness, and AI
natively:

```sh
sh scripts/build-web.sh
cd apps/tauri
pnpm install
pnpm tauri dev
```

Run a headless stress simulation:

```sh
cargo run -p ludo-simulation --release -- 10000
```

## Online server

Online play requires an account. The Rust server owns dice rolls, validates
every action against an exact revision, persists canonical state in PostgreSQL,
and publishes realtime updates through Ably. Clients receive short-lived,
read-only Ably JWTs scoped to their own private event channel; the Ably API key
never reaches the browser.

```sh
DATABASE_URL=postgresql://postgres:password@localhost:5432/ludo \
ABLY_API_KEY=your-app-id.your-key-id:your-key-secret \
  cargo run -p ludo-server
```

Create a free Ably app, copy a server API key from **API Keys**, and place it
in `ABLY_API_KEY`. The key needs `publish` for server fan-out and `subscribe`
so the server can mint narrowly scoped, read-only client tokens. It does not
need presence, history, or channel-management permissions. The existing Axum
WebSocket carries authenticated player commands; Ably provides scalable
cross-instance event delivery. Critical game events use a PostgreSQL outbox,
so a temporary Ably failure is retried instead of losing an accepted move.
Online turns have a 30-second server-authoritative deadline. A disconnected or
inactive player keeps their seat during that grace period, after which the
configured AI advances the turn so a match cannot be held indefinitely.
`GET /health/ready` verifies database readiness for Railway health checks.
Optionally set `LUDO_ALERT_WEBHOOK` to receive a JSON alert after repeated Ably
delivery failures.

Multiplayer tables support public discovery or private invite links, ready
checks, configurable 15–60 second turn clocks, database-backed presence,
automatic host transfer, quick matching, spectators, moderated match chat,
emoji reactions, activity history, and unanimous same-lineup rematches.
Waiting rooms with no recently connected members are removed automatically.

For local development, the client defaults to `http://localhost:8080`. For
production, deploy the `ludo-server` Rust binary with a PostgreSQL
`DATABASE_URL`, set `LUDO_ALLOWED_ORIGINS` to the web URL plus
`tauri://localhost,http://tauri.localhost,https://tauri.localhost`, and define
the GitHub Actions repository variable `LUDO_API_URL` with the public HTTPS
server URL (for example, the Railway service domain without a trailing slash).
CI embeds that URL as `VITE_API_URL` in the single web artifact reused by
Vercel and desktop installers. Production builds now fail when that variable is
missing or invalid instead of silently shipping desktop multiplayer pointed at
localhost.

## Project structure

For a deeper walkthrough of the architecture, platform behavior, and runtime flow, see [HOW-THE-APP-WORKS.md](HOW-THE-APP-WORKS.md).
Production health checks, metrics, backups, restore drills, feature flags,
privacy workflows, load tests, and incident procedures are documented in
[docs/operations.md](docs/operations.md).

## Quality checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

## Continuous delivery

Every pull request starts four independent CI lanes in parallel:

- Rust formatting, Clippy, tests, documentation, and a reusable server binary;
- TypeScript, unit/component tests, and the JavaScript dependency audit;
- the production Rust/WebAssembly build and bundle budgets;
- the Rust dependency security audit.

The PostgreSQL and browser lane starts only after the Rust, frontend, and web
lanes pass. It downloads the verified server binary and generated WASM package,
then rebuilds only the API-URL-specific browser shell. This avoids recompiling
the server, regenerating WASM, or reinstalling `wasm-bindgen`. A final CI gate
requires every lane to succeed and provides one stable branch-protection check.

CI stores the verified production bundle as `web-dist`. After a successful CI
run on `main`, the delivery workflow downloads that exact artifact and:

- builds a universal macOS DMG;
- builds Windows x64 NSIS (`.exe`) and WiX (`.msi`) installers;
- deploys the production web build to Vercel.

The web artifact contains `build-provenance.json` and `SHA256SUMS.txt`.
Every installer and deployment job verifies both before using the artifact.
The production smoke check waits until the public Vercel alias serves the exact
CI source SHA, preventing a stale deployment from passing merely because the
homepage is reachable. Published releases include installer checksums and
`RELEASE-PROVENANCE.json`, linking the source commit, release commit, CI run,
desktop artifacts, and production URL. The delivery gate accepts only a fully
published release or an intentional release-loop skip.

After every successful non-release push to `main`, delivery calculates the next
patch version, builds and deploys everything, then atomically pushes the version
commit and tag back to `main` and publishes the GitHub Release. The release
commit is detected on its follow-up CI run so it cannot create a release loop.
Installers are also retained as workflow artifacts. They are unsigned until
Apple and Windows signing credentials are added.

Create a Vercel project for the web client, then add these repository or
`production` environment secrets in GitHub:

- `VERCEL_TOKEN`
- `VERCEL_ORG_ID`
- `VERCEL_PROJECT_ID`

The IDs can be copied from the Vercel project's `.vercel/project.json` after
running `vercel link` locally. No Vercel build command is needed: GitHub builds
the Rust/WASM application once, then Vercel and both Tauri installer jobs reuse
the resulting static `apps/client/dist` directory.
