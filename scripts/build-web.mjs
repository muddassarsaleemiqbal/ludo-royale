import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const wasmOutput = resolve(repoRoot, "apps/client/src/wasm/pkg");

if (process.argv.includes("--require-api-url")) {
  const configured = process.env.VITE_API_URL?.trim();
  if (!configured) {
    console.error("VITE_API_URL must be set for production web and desktop builds.");
    process.exit(1);
  }
  try {
    const url = new URL(configured);
    const local = ["localhost", "127.0.0.1", "::1"].includes(url.hostname);
    if (!["http:", "https:"].includes(url.protocol) || (!local && url.protocol !== "https:"))
      throw new Error("public endpoints must use HTTPS");
  } catch (error) {
    console.error(`VITE_API_URL is invalid: ${error instanceof Error ? error.message : error}`);
    process.exit(1);
  }
}
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
if (process.argv.includes("--skip-typecheck")) {
  run("pnpm", ["--dir", "apps/client", "exec", "vite", "build"]);
} else {
  run("pnpm", ["--dir", "apps/client", "build"]);
}
