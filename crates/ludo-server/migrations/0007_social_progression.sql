CREATE TABLE player_progress (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    xp BIGINT NOT NULL DEFAULT 0 CHECK (xp >= 0),
    matches INTEGER NOT NULL DEFAULT 0 CHECK (matches >= 0),
    wins INTEGER NOT NULL DEFAULT 0 CHECK (wins >= 0),
    current_streak INTEGER NOT NULL DEFAULT 0 CHECK (current_streak >= 0),
    best_streak INTEGER NOT NULL DEFAULT 0 CHECK (best_streak >= 0),
    selected_dice TEXT NOT NULL DEFAULT 'ivory'
        CHECK (selected_dice IN ('ivory', 'obsidian', 'emerald', 'royal')),
    selected_tokens TEXT NOT NULL DEFAULT 'classic'
        CHECK (selected_tokens IN ('classic', 'neon', 'marble', 'metallic')),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO player_progress(user_id)
SELECT id FROM users
ON CONFLICT(user_id) DO NOTHING;

CREATE TABLE friendships (
    requester_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    addressee_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'accepted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(requester_id, addressee_id),
    CHECK (requester_id <> addressee_id)
);
CREATE INDEX friendships_addressee_idx ON friendships(addressee_id, status);

CREATE TABLE seasons (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    starts_at TIMESTAMPTZ NOT NULL,
    ends_at TIMESTAMPTZ NOT NULL,
    active BOOLEAN NOT NULL DEFAULT TRUE
);
INSERT INTO seasons(id,name,starts_at,ends_at,active)
VALUES(
    '00000000-0000-0000-0000-000000000001',
    'Founders Season',
    date_trunc('month', CURRENT_TIMESTAMP),
    date_trunc('month', CURRENT_TIMESTAMP) + INTERVAL '3 months',
    TRUE
) ON CONFLICT(id) DO NOTHING;

CREATE TABLE season_ratings (
    season_id UUID NOT NULL REFERENCES seasons(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    rating INTEGER NOT NULL DEFAULT 1000 CHECK (rating BETWEEN 0 AND 5000),
    matches INTEGER NOT NULL DEFAULT 0,
    wins INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(season_id,user_id)
);

CREATE TABLE player_achievements (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    achievement_key TEXT NOT NULL,
    unlocked_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(user_id,achievement_key)
);

CREATE TABLE daily_progress (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    challenge_date DATE NOT NULL DEFAULT CURRENT_DATE,
    challenge_key TEXT NOT NULL,
    progress INTEGER NOT NULL DEFAULT 0,
    claimed BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY(user_id,challenge_date,challenge_key)
);

CREATE TABLE friend_invites (
    id UUID PRIMARY KEY,
    lobby_id UUID NOT NULL REFERENCES game_lobbies(id) ON DELETE CASCADE,
    sender_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    recipient_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'accepted', 'declined')),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP + INTERVAL '30 minutes',
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(lobby_id,recipient_id)
);
CREATE INDEX friend_invites_recipient_idx
    ON friend_invites(recipient_id,status,expires_at);

ALTER TABLE game_lobbies
    ADD COLUMN ranked BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN rematch_mode TEXT NOT NULL DEFAULT 'vote'
        CHECK (rematch_mode IN ('vote', 'host', 'automatic')),
    ADD COLUMN replay_states JSONB NOT NULL DEFAULT '[]'::jsonb;

ALTER TABLE match_results
    ADD COLUMN replay_states JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN ranked BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN season_id UUID REFERENCES seasons(id) ON DELETE SET NULL;

ALTER TABLE match_results DROP CONSTRAINT match_results_lobby_id_key;
CREATE INDEX match_results_lobby_idx ON match_results(lobby_id,completed_at DESC);

CREATE TABLE match_participants (
    match_id UUID NOT NULL REFERENCES match_results(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    seat SMALLINT NOT NULL,
    placement SMALLINT NOT NULL,
    xp_earned INTEGER NOT NULL DEFAULT 0,
    rating_delta INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(match_id,user_id)
);
CREATE INDEX match_participants_user_idx
    ON match_participants(user_id,match_id);
