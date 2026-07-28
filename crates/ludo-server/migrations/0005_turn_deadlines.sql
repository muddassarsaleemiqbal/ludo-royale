ALTER TABLE game_lobbies
    ADD COLUMN turn_deadline TIMESTAMPTZ;

CREATE INDEX game_lobbies_turn_deadline_idx
    ON game_lobbies (turn_deadline)
    WHERE status = 'playing';
