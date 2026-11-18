ALTER TABLE users
    ADD COLUMN IF NOT EXISTS role TEXT NOT NULL DEFAULT 'member',
    ADD COLUMN IF NOT EXISTS probation_until TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS last_known_ip TEXT;

CREATE TABLE IF NOT EXISTS sessions (
    token_hash TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    csrf_token TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_user_id
    ON sessions (user_id);

CREATE INDEX IF NOT EXISTS idx_sessions_expires_at
    ON sessions (expires_at);

CREATE TABLE IF NOT EXISTS api_tokens (
    token_hash TEXT PRIMARY KEY,
    id UUID NOT NULL UNIQUE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    preview TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    last_used_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_api_tokens_user_id
    ON api_tokens (user_id);
