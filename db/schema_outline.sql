-- Schema Outline for VICEDEN Forum
--
-- This file is a textual outline of the intended database schema,
-- adapted from the planning documents.  It is not intended to be
-- executed directly; instead use SQLx migrations (see
-- migrations/README.md) or apply this schema manually via psql.

-- Users table.  Stores registration and authentication data.
CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY,
    handle TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    passkey_pubkey TEXT,
    subscription_expires_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT now(),
    banned BOOLEAN NOT NULL DEFAULT FALSE,
    ban_reason TEXT,
    -- Additional fields: invite codes remaining, etc.
    CONSTRAINT handle_format CHECK (handle ~ '^[a-z0-9_-]{3,16}$')
);

-- Boards table.  Although boards are configured via TOML, storing them in
-- the database allows migrations to add fields such as creation time,
-- statistics or dynamic ordering.  The slug is unique.
CREATE TABLE IF NOT EXISTS boards (
    slug TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    ttl_days INTEGER NOT NULL CHECK (ttl_days > 0),
    description TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT now()
);

-- Threads table.  Each thread belongs to a board and a user.
CREATE TABLE IF NOT EXISTS threads (
    id UUID PRIMARY KEY,
    board_slug TEXT NOT NULL REFERENCES boards(slug) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL CHECK (char_length(title) <= 64),
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT now(),
    expires_at TIMESTAMP WITH TIME ZONE NOT NULL,
    -- index for listing threads by board
    CONSTRAINT expires_after_created CHECK (expires_at > created_at)
);
CREATE INDEX IF NOT EXISTS idx_threads_board ON threads (board_slug, created_at DESC);

-- Posts table.  Each post belongs to a thread and a user.
CREATE TABLE IF NOT EXISTS posts (
    id UUID PRIMARY KEY,
    thread_id UUID NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT now(),
    state INTEGER NOT NULL DEFAULT 0,
    CONSTRAINT state_valid CHECK (state IN (0, 1, 2))
);
CREATE INDEX IF NOT EXISTS idx_posts_thread ON posts (thread_id, created_at ASC);

-- Audit table.  Records administrative actions such as bans and
-- deletions.  Not implemented in code yet.
CREATE TABLE IF NOT EXISTS audit_events (
    id SERIAL PRIMARY KEY,
    event_type TEXT NOT NULL,
    subject_id UUID,
    operator_id UUID,
    details JSONB,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT now()
);

-- Invite codes table.  Stores invite codes issued by users.  Each
-- code is single‑use and has an expiry.  Not implemented in code yet.
CREATE TABLE IF NOT EXISTS invite_codes (
    code TEXT PRIMARY KEY,
    issuer_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TIMESTAMP WITH TIME ZONE,
    consumed_by UUID REFERENCES users(id),
    consumed_at TIMESTAMP WITH TIME ZONE
);

-- Future tables: tag taxonomy, rate limit counters, payment
-- transactions, AI moderation logs.