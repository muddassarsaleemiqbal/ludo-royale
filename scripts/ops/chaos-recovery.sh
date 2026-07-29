#!/usr/bin/env bash
set -euo pipefail

: "${DATABASE_URL:?DATABASE_URL is required}"
server_log="$(mktemp)"
cargo run --quiet -p ludo-server >"$server_log" 2>&1 &
server_pid=$!
cleanup() { kill "$server_pid" 2>/dev/null || true; }
trap cleanup EXIT

for _ in {1..60}; do
  curl --fail --silent http://127.0.0.1:8080/health/ready >/dev/null && break
  sleep 1
done
kill -TERM "$server_pid"
wait "$server_pid" || true
cargo run --quiet -p ludo-server >"$server_log" 2>&1 &
server_pid=$!
for _ in {1..60}; do
  if curl --fail --silent http://127.0.0.1:8080/health/ready >/dev/null; then
    echo "Restart recovery passed"
    exit 0
  fi
  sleep 1
done
cat "$server_log"
exit 1
