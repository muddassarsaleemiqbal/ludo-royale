CREATE TYPE lobby_status AS ENUM ('waiting', 'playing', 'finished');
CREATE TYPE join_request_status AS ENUM ('pending', 'accepted', 'declined');

CREATE TABLE game_lobbies (
    id UUID PRIMARY KEY,
    host_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    rule_preset TEXT NOT NULL CHECK (rule_preset IN ('classic', 'quick', 'tournament')),
    bot_difficulty TEXT NOT NULL CHECK (bot_difficulty IN ('easy', 'medium', 'hard')),
    is_public BOOLEAN NOT NULL DEFAULT TRUE,
    status lobby_status NOT NULL DEFAULT 'waiting',
    game_state JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE lobby_members (
    lobby_id UUID NOT NULL REFERENCES game_lobbies(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    seat SMALLINT NOT NULL CHECK (seat BETWEEN 0 AND 3),
    joined_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (lobby_id, user_id),
    UNIQUE (lobby_id, seat)
);

CREATE TABLE lobby_join_requests (
    id UUID PRIMARY KEY,
    lobby_id UUID NOT NULL REFERENCES game_lobbies(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status join_request_status NOT NULL DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (lobby_id, user_id)
);

CREATE INDEX game_lobbies_discovery_idx
    ON game_lobbies (status, is_public, updated_at DESC);
CREATE INDEX lobby_join_requests_host_idx
    ON lobby_join_requests (lobby_id, status, created_at);
