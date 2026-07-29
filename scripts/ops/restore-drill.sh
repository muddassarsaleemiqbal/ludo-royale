#!/usr/bin/env bash
set -euo pipefail

: "${DATABASE_URL:?DATABASE_URL is required}"
archive="${1:?Pass a pg_dump custom archive}"
drill_db="ludo_restore_drill_$(date +%s)"
admin_url="${DATABASE_ADMIN_URL:-$DATABASE_URL}"

createdb --maintenance-db="$admin_url" "$drill_db"
cleanup() { dropdb --maintenance-db="$admin_url" --if-exists "$drill_db"; }
trap cleanup EXIT
pg_restore --dbname="${admin_url%/*}/$drill_db" --no-owner "$archive"
psql "${admin_url%/*}/$drill_db" --set=ON_ERROR_STOP=1 --command \
  "SELECT count(*) AS migrations FROM _sqlx_migrations; SELECT count(*) AS users FROM users;"
echo "Restore drill passed for $archive"
