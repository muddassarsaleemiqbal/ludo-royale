import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const wasmOutput = resolve(repoRoot, "apps/client/src/wasm/pkg");
function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    stdio: "inherit",
    // Windows resolves pnpm through its .cmd shim, which Node cannot execute
    // directly with shell disabled (spawnSync returns EINVAL).
    shell: process.platform === "win32" && command === "pnpm"
  });

  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

run("cargo", [
  "build",
  "-p",
  "ludo-web",
  "--target",
  "wasm32-unknown-unknown",
  "--release"
]);

mkdirSync(wasmOutput, { recursive: true });
run("wasm-bindgen", [
  resolve(
    repoRoot,
    "target/wasm32-unknown-unknown/release/ludo_web.wasm"
  ),
  "--out-dir",
  wasmOutput,
  "--target",
  "web",
  "--no-typescript"
]);
run("pnpm", ["--dir", "apps/client", "build"]);
