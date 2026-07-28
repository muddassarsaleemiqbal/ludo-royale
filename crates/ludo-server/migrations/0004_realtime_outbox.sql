CREATE TABLE realtime_outbox (
    id BIGSERIAL PRIMARY KEY,
    channel TEXT NOT NULL,
    event_name TEXT NOT NULL,
    payload JSONB NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    published_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX realtime_outbox_pending_idx
    ON realtime_outbox (next_attempt_at, id)
    WHERE published_at IS NULL;

CREATE TABLE processed_commands (
    command_id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX processed_commands_expiry_idx
    ON processed_commands (processed_at);

ALTER TABLE match_results
    ADD COLUMN lobby_id UUID UNIQUE REFERENCES game_lobbies(id) ON DELETE SET NULL;
