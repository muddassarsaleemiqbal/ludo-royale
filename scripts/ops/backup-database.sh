#!/usr/bin/env bash
set -euo pipefail

: "${DATABASE_URL:?DATABASE_URL is required}"
backup_dir="${1:-backups}"
mkdir -p "$backup_dir"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
archive="$backup_dir/ludo-$timestamp.dump"

pg_dump --dbname="$DATABASE_URL" --format=custom --compress=9 --no-owner --file="$archive"
sha256sum "$archive" >"$archive.sha256"
pg_restore --list "$archive" >/dev/null
echo "Created and verified $archive"
