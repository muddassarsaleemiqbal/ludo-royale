import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const wasmOutput = resolve(repoRoot, "apps/client/src/wasm/pkg");
const executable = process.platform === "win32" ? "pnpm.cmd" : "pnpm";

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    stdio: "inherit",
    shell: false
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
run(executable, ["--dir", "apps/client", "build"]);
