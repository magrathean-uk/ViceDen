CREATE TABLE IF NOT EXISTS notifications (
    id UUID PRIMARY KEY,
    recipient_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    actor_id UUID,
    kind TEXT NOT NULL,
    thread_id UUID,
    thread_title TEXT NOT NULL,
    post_id UUID,
    message TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    is_read BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_notifications_recipient_created
    ON notifications (recipient_user_id, created_at DESC);

CREATE TABLE IF NOT EXISTS reports (
    id UUID PRIMARY KEY,
    actor_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_post_id UUID,
    target_thread_id UUID,
    reason TEXT NOT NULL,
    details TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    resolved BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_reports_resolved_created
    ON reports (resolved, created_at DESC);

CREATE TABLE IF NOT EXISTS moderation_events (
    id UUID PRIMARY KEY,
    actor_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_post_id UUID,
    target_thread_id UUID,
    reason TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_moderation_events_created
    ON moderation_events (created_at DESC);

CREATE TABLE IF NOT EXISTS banned_ips (
    ip TEXT PRIMARY KEY
);
