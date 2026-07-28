ALTER TABLE game_lobbies
    ADD COLUMN invite_code TEXT UNIQUE,
    ADD COLUMN turn_seconds SMALLINT NOT NULL DEFAULT 30
        CHECK (turn_seconds IN (15, 30, 45, 60)),
    ADD COLUMN rematch_of UUID REFERENCES game_lobbies(id) ON DELETE SET NULL;

UPDATE game_lobbies
SET invite_code = upper(substr(replace(id::text, '-', ''), 1, 8))
WHERE invite_code IS NULL;

ALTER TABLE game_lobbies
    ALTER COLUMN invite_code SET NOT NULL;

ALTER TABLE lobby_members
    ADD COLUMN ready BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN last_seen_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP;

UPDATE lobby_members m
SET ready = TRUE
FROM game_lobbies l
WHERE l.id = m.lobby_id AND l.host_user_id = m.user_id;

CREATE TABLE lobby_spectators (
    lobby_id UUID NOT NULL REFERENCES game_lobbies(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (lobby_id, user_id)
);

CREATE TABLE lobby_events (
    id BIGSERIAL PRIMARY KEY,
    lobby_id UUID NOT NULL REFERENCES game_lobbies(id) ON DELETE CASCADE,
    actor_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    kind TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX lobby_events_recent_idx
    ON lobby_events (lobby_id, id DESC);

CREATE TABLE rematch_votes (
    lobby_id UUID NOT NULL REFERENCES game_lobbies(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (lobby_id, user_id)
);

CREATE INDEX lobby_members_presence_idx
    ON lobby_members (last_seen_at);
