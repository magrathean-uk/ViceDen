ALTER TABLE sessions
    ADD COLUMN IF NOT EXISTS token_digest_version TEXT NOT NULL DEFAULT 'sha256';

ALTER TABLE api_tokens
    ADD COLUMN IF NOT EXISTS token_digest_version TEXT NOT NULL DEFAULT 'sha256';

CREATE INDEX IF NOT EXISTS idx_sessions_digest_version_expires
    ON sessions (token_digest_version, expires_at);

CREATE INDEX IF NOT EXISTS idx_api_tokens_user_created
    ON api_tokens (user_id, created_at DESC);

DELETE FROM sessions WHERE expires_at <= now();
