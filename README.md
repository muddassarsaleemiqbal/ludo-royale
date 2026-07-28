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

For local development, the client defaults to `http://localhost:8080`. For
production, deploy the `ludo-server` Rust binary with a PostgreSQL
`DATABASE_URL`, set `LUDO_ALLOWED_ORIGINS` to the Vercel URL, and define the
GitHub Actions repository variable `LUDO_API_URL` with the public HTTPS server
URL. CI embeds that URL as `VITE_API_URL` in the single web artifact reused by
Vercel and desktop installers.

## Project structure

For a deeper walkthrough of the architecture, platform behavior, and runtime flow, see [HOW-THE-APP-WORKS.md](HOW-THE-APP-WORKS.md).

## Quality checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

## Continuous delivery

Every pull request runs the Rust, TypeScript, and WebAssembly checks. CI builds
the production web bundle once and stores it as `web-dist`. After a successful
CI run on `main`, the delivery workflow downloads that exact verified artifact
and:

- builds a universal macOS DMG;
- builds Windows x64 NSIS (`.exe`) and WiX (`.msi`) installers;
- deploys the production web build to Vercel.

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
