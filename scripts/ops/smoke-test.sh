#!/usr/bin/env bash
set -euo pipefail

base_url="${1:-http://127.0.0.1:8080}"
curl --fail --silent --show-error "$base_url/health" | grep -qx ok
curl --fail --silent --show-error "$base_url/health/ready" | grep -qx ready
curl --fail --silent --show-error "$base_url/metrics" | grep -q ludo_commands_total
echo "Smoke test passed: $base_url"
