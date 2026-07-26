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

Desktop installers are attached to the GitHub Release matching the Tauri
application version (for example, `v0.1.1`) and retained as GitHub Actions
workflow artifacts. They are unsigned until Apple and Windows signing
credentials are added. Bump the workspace and Tauri application version before
merging the changes that should create the next release.

Create a Vercel project for the web client, then add these repository or
`production` environment secrets in GitHub:

- `VERCEL_TOKEN`
- `VERCEL_ORG_ID`
- `VERCEL_PROJECT_ID`

The IDs can be copied from the Vercel project's `.vercel/project.json` after
running `vercel link` locally. No Vercel build command is needed: GitHub builds
the Rust/WASM application once, then Vercel and both Tauri installer jobs reuse
the resulting static `apps/client/dist` directory.
