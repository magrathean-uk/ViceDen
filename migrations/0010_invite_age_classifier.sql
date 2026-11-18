-- 0010_invite_age_classifier.sql
-- M1+M2 schema deltas. See spec §7.

-- ─── Invites: expand single-shot into quota-based with revocation/expiry ───
ALTER TABLE invites
    ADD COLUMN IF NOT EXISTS expires_at        TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS max_redemptions   INT NOT NULL DEFAULT 1
                                              CHECK (max_redemptions > 0),
    ADD COLUMN IF NOT EXISTS redemption_count  INT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS revoked_at        TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS revoke_reason     TEXT,
    ADD COLUMN IF NOT EXISTS note              TEXT;

CREATE TABLE IF NOT EXISTS invite_redemptions (
    invite_code      TEXT NOT NULL REFERENCES invites(code),
    redeemed_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    redeemed_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    redeemed_from_ip INET,
    PRIMARY KEY (invite_code, redeemed_user_id)
);
CREATE INDEX IF NOT EXISTS idx_invite_redemptions_user
    ON invite_redemptions (redeemed_user_id);

-- Backfill existing single-shot redemptions into the new shape.
INSERT INTO invite_redemptions (invite_code, redeemed_user_id, redeemed_at)
SELECT code, redeemed_by, COALESCE(redeemed_at, created_at)
FROM invites
WHERE redeemed_by IS NOT NULL
ON CONFLICT DO NOTHING;

UPDATE invites SET redemption_count =
    (SELECT count(*)::int FROM invite_redemptions r WHERE r.invite_code = invites.code);

-- Drop deprecated columns in the same migration (Risk 3 fix in spec §8).
-- Trade-off: rollback past 0010 requires DB restore, not just symlink swap.
ALTER TABLE invites
    DROP COLUMN IF EXISTS redeemed_by,
    DROP COLUMN IF EXISTS redeemed_at;

-- ─── Age attestations (write-once at app layer) ───
CREATE TABLE IF NOT EXISTS age_attestations (
    user_id     UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    dob         DATE NOT NULL,
    attested_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    attested_ip INET NOT NULL,
    attested_ua TEXT NOT NULL,
    invite_code TEXT NOT NULL REFERENCES invites(code)
);

-- ─── AI classifier audit log ───
CREATE TABLE IF NOT EXISTS content_classifications (
    id            BIGSERIAL PRIMARY KEY,
    target_kind   TEXT NOT NULL CHECK (target_kind IN ('post', 'thread')),
    target_id     UUID NOT NULL,
    verdict       TEXT NOT NULL
                  CHECK (verdict IN ('allow', 'quarantine', 'reject', 'error')),
    categories    TEXT[] NOT NULL DEFAULT '{}',
    confidence    REAL,
    model         TEXT NOT NULL,
    request_id    TEXT,
    latency_ms    INT,
    classified_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    raw_response  JSONB
);
CREATE INDEX IF NOT EXISTS idx_content_class_target
    ON content_classifications (target_kind, target_id, classified_at DESC);

-- ─── Moderation state on posts and threads, additive ───
ALTER TABLE posts
    ADD COLUMN IF NOT EXISTS mod_state            TEXT NOT NULL DEFAULT 'visible'
        CHECK (mod_state IN ('pending','visible','quarantined','removed')),
    ADD COLUMN IF NOT EXISTS mod_state_reason     TEXT,
    ADD COLUMN IF NOT EXISTS mod_state_changed_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS idx_posts_mod_state
    ON posts (mod_state) WHERE mod_state <> 'visible';

ALTER TABLE threads
    ADD COLUMN IF NOT EXISTS mod_state            TEXT NOT NULL DEFAULT 'visible'
        CHECK (mod_state IN ('pending','visible','quarantined','removed')),
    ADD COLUMN IF NOT EXISTS mod_state_reason     TEXT,
    ADD COLUMN IF NOT EXISTS mod_state_changed_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS idx_threads_mod_state
    ON threads (mod_state) WHERE mod_state <> 'visible';

-- ─── Cauterise tracking ───
ALTER TABLE users
    ADD COLUMN IF NOT EXISTS deactivated_at     TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS deactivated_by     UUID REFERENCES users(id),
    ADD COLUMN IF NOT EXISTS deactivated_reason TEXT;

-- ─── Handle blocklist (founder-managed) ───
CREATE TABLE IF NOT EXISTS handle_blocklist (
    handle_pattern TEXT PRIMARY KEY,
    is_regex       BOOLEAN NOT NULL DEFAULT FALSE,
    added_by       UUID NOT NULL REFERENCES users(id),
    added_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    note           TEXT
);
