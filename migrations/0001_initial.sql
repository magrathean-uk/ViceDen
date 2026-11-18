-- Initial schema for VICEDEN Forum.
-- Keep this migration small and explicit.  Add follow-up migrations for PATs,
-- invite codes, moderation events, and payments rather than editing history.

CREATE TABLE IF NOT EXISTS boards (
    slug TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    ttl_days INTEGER NOT NULL CHECK (ttl_days > 0),
    description TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY,
    handle TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    passkey_pubkey TEXT,
    subscription_expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    banned BOOLEAN NOT NULL DEFAULT FALSE,
    ban_reason TEXT,
    CONSTRAINT handle_format CHECK (handle ~ '^[a-z0-9_-]{3,16}$')
);

CREATE TABLE IF NOT EXISTS threads (
    id UUID PRIMARY KEY,
    board_slug TEXT NOT NULL REFERENCES boards(slug) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL CHECK (char_length(title) <= 64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT expires_after_created CHECK (expires_at > created_at)
);

CREATE INDEX IF NOT EXISTS idx_threads_board_created
    ON threads (board_slug, created_at DESC);

CREATE TABLE IF NOT EXISTS posts (
    id UUID PRIMARY KEY,
    thread_id UUID NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    state INTEGER NOT NULL DEFAULT 0,
    CONSTRAINT state_valid CHECK (state IN (0, 1, 2))
);

CREATE INDEX IF NOT EXISTS idx_posts_thread_created
    ON posts (thread_id, created_at ASC);
