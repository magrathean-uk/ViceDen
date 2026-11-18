-- 0011_invite_redemption_check.sql
-- Belt-and-braces invariant for invites: redemption_count must never exceed
-- max_redemptions. The application path now relies on
--   UPDATE invites SET redemption_count = redemption_count + 1
--   WHERE code = $1 AND redemption_count < max_redemptions
-- to win the double-spend race atomically; this CHECK guards against any
-- direct SQL ever drifting past capacity.
ALTER TABLE invites
    ADD CONSTRAINT invites_redemption_count_le_max
    CHECK (redemption_count <= max_redemptions);
