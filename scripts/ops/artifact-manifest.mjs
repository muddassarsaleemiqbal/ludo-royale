import { createHash } from "node:crypto";
import { readdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";

const [mode, directoryArgument, expectedSha] = process.argv.slice(2);
if (!["create", "verify"].includes(mode) || !directoryArgument || !expectedSha) {
  console.error("Usage: artifact-manifest.mjs <create|verify> <directory> <source-sha>");
  process.exit(2);
}

const directory = path.resolve(directoryArgument);
const manifestName = "SHA256SUMS.txt";
const provenanceName = "build-provenance.json";

async function filesBelow(root, prefix = "") {
  const names = await readdir(path.join(root, prefix));
  const files = [];
  for (const name of names.sort()) {
    const relative = path.posix.join(prefix.split(path.sep).join(path.posix.sep), name);
    const absolute = path.join(root, ...relative.split("/"));
    if ((await stat(absolute)).isDirectory()) files.push(...await filesBelow(root, relative));
    else if (relative !== manifestName) files.push(relative);
  }
  return files;
}

async function digest(relative) {
  const bytes = await readFile(path.join(directory, ...relative.split("/")));
  return createHash("sha256").update(bytes).digest("hex");
}

if (mode === "create") {
  await writeFile(
    path.join(directory, provenanceName),
    `${JSON.stringify({
      source_sha: expectedSha,
      ci_run_id: process.env.GITHUB_RUN_ID ?? null,
      repository: process.env.GITHUB_REPOSITORY ?? null
    }, null, 2)}\n`
  );
  const files = await filesBelow(directory);
  const entries = await Promise.all(files.map(async relative =>
    `${await digest(relative)}  ${relative}`
  ));
  await writeFile(path.join(directory, manifestName), `${entries.join("\n")}\n`);
  console.log(`Created provenance and ${entries.length} checksums for ${expectedSha}.`);
} else {
  const provenance = JSON.parse(await readFile(
    path.join(directory, provenanceName), "utf8"
  ));
  if (provenance.source_sha !== expectedSha) {
    throw new Error(
      `Artifact source ${String(provenance.source_sha)} does not match ${expectedSha}`
    );
  }
  const manifest = await readFile(path.join(directory, manifestName), "utf8");
  const expected = new Map(manifest.trim().split("\n").map(line => {
    const match = /^([a-f0-9]{64})  (.+)$/.exec(line);
    if (!match) throw new Error(`Invalid checksum entry: ${line}`);
    return [match[2], match[1]];
  }));
  const files = await filesBelow(directory);
  if (files.length !== expected.size || files.some(file => !expected.has(file))) {
    throw new Error("Artifact contents do not match the checksum manifest");
  }
  for (const relative of files) {
    if (await digest(relative) !== expected.get(relative))
      throw new Error(`Checksum mismatch: ${relative}`);
  }
  console.log(`Verified ${files.length} files from ${expectedSha}.`);
}
