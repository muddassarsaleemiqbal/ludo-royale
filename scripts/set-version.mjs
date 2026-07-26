import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  throw new Error("usage: node scripts/set-version.mjs <major.minor.patch>");
}

const repoRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const cargo = spawnSync(
  "cargo",
  ["run", "--quiet", "-p", "xtask", "--", "set-version", version],
  { cwd: repoRoot, stdio: "inherit" }
);
if (cargo.error) throw cargo.error;
if (cargo.status !== 0) process.exit(cargo.status ?? 1);

for (const relativePath of [
  "apps/client/package.json",
  "apps/tauri/package.json",
  "apps/tauri/src-tauri/tauri.conf.json"
]) {
  const path = resolve(repoRoot, relativePath);
  const document = JSON.parse(readFileSync(path, "utf8"));
  document.version = version;
  writeFileSync(path, `${JSON.stringify(document, null, 2)}\n`);
}
