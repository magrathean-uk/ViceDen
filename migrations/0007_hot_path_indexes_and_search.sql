CREATE INDEX IF NOT EXISTS idx_threads_board_created_id
    ON threads (board_slug, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_posts_thread_created_id
    ON posts (thread_id, created_at ASC, id ASC);

CREATE INDEX IF NOT EXISTS idx_posts_user_created_id
    ON posts (user_id, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_votes_post_id
    ON votes (post_id);

CREATE INDEX IF NOT EXISTS idx_votes_user_id
    ON votes (user_id);

CREATE INDEX IF NOT EXISTS idx_notifications_recipient_read_created
    ON notifications (recipient_user_id, is_read, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_reports_resolved_created_id
    ON reports (resolved, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_saved_threads_thread_user
    ON saved_threads (thread_id, user_id);

CREATE INDEX IF NOT EXISTS idx_hidden_threads_thread_user
    ON hidden_threads (thread_id, user_id);

CREATE INDEX IF NOT EXISTS idx_threads_title_fts
    ON threads USING GIN (to_tsvector('english', title));

CREATE INDEX IF NOT EXISTS idx_posts_body_fts
    ON posts USING GIN (to_tsvector('english', body));
