CREATE TABLE IF NOT EXISTS thread_states (
    thread_id UUID PRIMARY KEY REFERENCES threads(id) ON DELETE CASCADE,
    pinned BOOLEAN NOT NULL DEFAULT FALSE,
    locked BOOLEAN NOT NULL DEFAULT FALSE,
    answered BOOLEAN NOT NULL DEFAULT FALSE,
    closed BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE IF NOT EXISTS post_edits (
    post_id UUID NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    editor_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    previous_body TEXT NOT NULL,
    new_body TEXT NOT NULL,
    edited_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (post_id, edited_at)
);

CREATE TABLE IF NOT EXISTS muted_users (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    muted_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, muted_user_id)
);

CREATE TABLE IF NOT EXISTS tag_subscriptions (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    PRIMARY KEY (user_id, tag)
);

CREATE TABLE IF NOT EXISTS invites (
    code TEXT PRIMARY KEY,
    label TEXT NOT NULL DEFAULT '',
    inviter_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL,
    redeemed_by UUID REFERENCES users(id) ON DELETE SET NULL,
    redeemed_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_post_edits_post_time
    ON post_edits (post_id, edited_at DESC);

CREATE INDEX IF NOT EXISTS idx_tag_subscriptions_tag_user
    ON tag_subscriptions (tag, user_id);
