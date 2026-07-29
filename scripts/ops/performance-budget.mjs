import fs from "node:fs";
import path from "node:path";

const dist = path.resolve("apps/client/dist/assets");
const limits = { javascript: 600_000, css: 75_000, wasm: 350_000 };
const totals = { javascript: 0, css: 0, wasm: 0 };
for (const entry of fs.readdirSync(dist)) {
  const size = fs.statSync(path.join(dist, entry)).size;
  if (entry.endsWith(".js")) totals.javascript += size;
  if (entry.endsWith(".css")) totals.css += size;
  if (entry.endsWith(".wasm")) totals.wasm += size;
}
let failed = false;
for (const [kind, limit] of Object.entries(limits)) {
  console.log(`${kind}: ${totals[kind]} / ${limit} bytes`);
  if (totals[kind] > limit) failed = true;
}
if (failed) process.exitCode = 1;
