# Ludo

A fast, native Ludo game written entirely in Rust. The deterministic rules engine is
independent of the GPUI desktop interface, and computer players evaluate moves in
parallel with Rayon.

## Run

```sh
cargo run -p ludo-desktop --release
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
