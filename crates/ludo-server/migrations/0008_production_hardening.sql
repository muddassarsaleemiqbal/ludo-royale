-- Stable identity for one play-through, including rematches in the same lobby.
ALTER TABLE game_lobbies
    ADD COLUMN game_instance_id UUID;

ALTER TABLE match_results
    ADD COLUMN game_instance_id UUID;

UPDATE match_results
SET game_instance_id = id
WHERE game_instance_id IS NULL;

UPDATE game_lobbies
SET game_instance_id = id
WHERE game_state IS NOT NULL AND game_instance_id IS NULL;

ALTER TABLE match_results
    ALTER COLUMN game_instance_id SET NOT NULL,
    ADD CONSTRAINT match_results_game_instance_unique UNIQUE(game_instance_id);

-- A friendship may only exist once regardless of request direction.
WITH ranked_friendships AS (
    SELECT ctid,
           row_number() OVER(
             PARTITION BY LEAST(requester_id,addressee_id),GREATEST(requester_id,addressee_id)
             ORDER BY (status='accepted') DESC,updated_at DESC
           ) AS duplicate_rank
    FROM friendships
)
DELETE FROM friendships friendship
USING ranked_friendships ranked
WHERE friendship.ctid=ranked.ctid AND ranked.duplicate_rank>1;

CREATE UNIQUE INDEX friendships_canonical_unique
    ON friendships (
        LEAST(requester_id,addressee_id),
        GREATEST(requester_id,addressee_id)
    );

-- Hot-path indexes verified against lobby discovery, presence, social hub and history queries.
CREATE INDEX match_results_completed_idx
    ON match_results(completed_at DESC,id);
CREATE INDEX friendships_requester_status_idx
    ON friendships(requester_id,status,updated_at DESC);
CREATE INDEX season_ratings_leaderboard_idx
    ON season_ratings(season_id,rating DESC,wins DESC);
CREATE INDEX friend_invites_pending_expiry_idx
    ON friend_invites(recipient_id,expires_at DESC)
    WHERE status='pending';

-- Operational audit trail. Payloads must never contain passwords or session tokens.
CREATE TABLE admin_audit_log (
    id BIGSERIAL PRIMARY KEY,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    target_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX admin_audit_recent_idx ON admin_audit_log(created_at DESC);

-- Privacy workflow: requests are durable and can be audited before execution.
CREATE TABLE account_deletion_requests (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    execute_after TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP + INTERVAL '24 hours',
    cancelled_at TIMESTAMPTZ
);

-- Keep transient tables bounded without deleting durable match history.
CREATE INDEX lobby_events_created_idx ON lobby_events(created_at);
CREATE INDEX sessions_user_idx ON sessions(user_id);

CREATE TABLE user_presence (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX user_presence_seen_idx ON user_presence(last_seen_at);
