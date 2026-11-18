CREATE INDEX IF NOT EXISTS idx_votes_post_value
    ON votes (post_id, value);
