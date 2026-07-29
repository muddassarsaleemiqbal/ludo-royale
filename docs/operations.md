# Multiplayer operations

## Required production configuration

Set `DATABASE_URL`, `LUDO_ENV=production`, `LUDO_ALLOWED_ORIGINS`, and a strong
`LUDO_ADMIN_TOKEN`. Pool sizing and timeouts are controlled with
`LUDO_DB_MAX_CONNECTIONS`, `LUDO_DB_MIN_CONNECTIONS`,
`LUDO_DB_ACQUIRE_TIMEOUT_SECONDS`, and `LUDO_DB_IDLE_TIMEOUT_SECONDS`.

Feature flags are `LUDO_FEATURE_RANKED`, `LUDO_FEATURE_SOCIAL`, and
`LUDO_FEATURE_REPLAYS`. Disable a feature before a risky rollout without
shipping a different client.

## Observability

- `/health` is a process liveness probe.
- `/health/ready` verifies database readiness.
- `/metrics` exposes Prometheus counters and gauges.
- Logs include user, lobby, command and database failure context without
  passwords or session tokens.

Alert on readiness failures, outbound message drops, command error growth,
database pool exhaustion, and restart loops.

## Backups and restoration

Run `scripts/ops/backup-database.sh` on a schedule and copy both the custom
archive and checksum to separate durable storage. Run
`scripts/ops/restore-drill.sh <archive>` regularly. A backup is not considered
valid until a restore drill succeeds.

## Deployments and rollback

Run migrations before routing traffic. Execute `scripts/ops/smoke-test.sh`
against the candidate instance. Keep the previous application image available;
database migrations are additive so the previous server remains compatible.
Disable affected feature flags first, then roll back the application if smoke
checks or production metrics regress.

## Privacy and administration

Authenticated players may schedule or cancel account deletion at
`/api/me/deletion`. Deletion has a 24-hour cancellation window and waits for
active games to finish. Admin endpoints require `x-ludo-admin-token`; mutations
are recorded in `admin_audit_log`.

Transient commands, delivered outbox messages, expired invitations, and old
lobby events are pruned automatically. Match history remains durable.

## Capacity and resilience

Run `scripts/ops/load-test.mjs` against staging and
`scripts/ops/chaos-recovery.sh` with an isolated database before major releases.
The WebSocket outbound queue is bounded; `ludo_outbound_dropped_total` indicates
slow consumers or insufficient capacity.
