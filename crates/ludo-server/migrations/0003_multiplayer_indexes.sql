CREATE INDEX IF NOT EXISTS lobby_members_user_idx
    ON lobby_members (user_id, joined_at DESC);

CREATE INDEX IF NOT EXISTS lobby_join_requests_user_idx
    ON lobby_join_requests (user_id, status, created_at DESC);

CREATE INDEX IF NOT EXISTS game_lobbies_host_status_idx
    ON game_lobbies (host_user_id, status, updated_at DESC);

CREATE INDEX IF NOT EXISTS sessions_expiry_idx
    ON sessions (expires_at);
