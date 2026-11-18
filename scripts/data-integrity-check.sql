-- Viceden data-integrity check.
--
-- Run against a Postgres instance with the viceden migrations applied:
--
--   psql "$VICEDEN_DATABASE__URL" -f scripts/data-integrity-check.sql
--
-- Each query reports zero rows when the invariant holds. Non-zero rows
-- mean an integrity drift — investigate before continuing.
--
-- Read-only. Safe against production (only SELECTs).

\echo
\echo === 1. Orphaned posts (post.thread_id has no matching threads row) ===
SELECT p.id AS post_id, p.thread_id, p.created_at
FROM posts p
LEFT JOIN threads t ON t.id = p.thread_id
WHERE t.id IS NULL
ORDER BY p.created_at DESC
LIMIT 50;

\echo
\echo === 2. Posts with orphaned author (user_id has no matching users row) ===
SELECT p.id AS post_id, p.user_id, p.created_at
FROM posts p
LEFT JOIN users u ON u.id = p.user_id
WHERE u.id IS NULL
ORDER BY p.created_at DESC
LIMIT 50;

\echo
\echo === 3. Threads with orphaned author ===
SELECT t.id AS thread_id, t.user_id, t.created_at
FROM threads t
LEFT JOIN users u ON u.id = t.user_id
WHERE u.id IS NULL
ORDER BY t.created_at DESC
LIMIT 50;

\echo
\echo === 4. Users without an age_attestation row ===
-- Boot policy requires every user to have one. Existing users predating
-- migration 0010 are exempt; new signups should always have a row.
SELECT u.id, u.handle, u.created_at
FROM users u
LEFT JOIN age_attestations a ON a.user_id = u.id
WHERE a.user_id IS NULL
  AND u.created_at >= '2026-04-22'  -- approx M1 cutover; tune as needed
ORDER BY u.created_at DESC
LIMIT 50;

\echo
\echo === 5. Expired sessions still in DB (cleanup gap) ===
SELECT token_hash, user_id, expires_at, token_digest_version
FROM sessions
WHERE expires_at < now()
ORDER BY expires_at ASC
LIMIT 50;

\echo
\echo === 6. Invites with redemption_count exceeding max_redemptions ===
-- Migration 0011 added a CHECK constraint preventing this; query verifies
-- the constraint is intact and no row slipped through before it landed.
SELECT code, redemption_count, max_redemptions, created_at
FROM invites
WHERE redemption_count > max_redemptions
LIMIT 50;

\echo
\echo === 7. Invite_redemptions referencing missing invite codes ===
SELECT ir.invite_code, ir.redeemed_user_id, ir.redeemed_at
FROM invite_redemptions ir
LEFT JOIN invites i ON i.code = ir.invite_code
WHERE i.code IS NULL
ORDER BY ir.redeemed_at DESC
LIMIT 50;

\echo
\echo === 8. Content classifications without matching post or thread ===
SELECT cc.id, cc.target_kind, cc.target_id, cc.classified_at
FROM content_classifications cc
LEFT JOIN posts p ON cc.target_kind = 'post' AND p.id = cc.target_id
LEFT JOIN threads t ON cc.target_kind = 'thread' AND t.id = cc.target_id
WHERE
  (cc.target_kind = 'post' AND p.id IS NULL)
  OR (cc.target_kind = 'thread' AND t.id IS NULL)
ORDER BY cc.classified_at DESC
LIMIT 50;

\echo
\echo === 9. Reports referencing missing post / thread ===
SELECT r.id, r.target_post_id, r.target_thread_id, r.created_at
FROM reports r
LEFT JOIN posts p ON p.id = r.target_post_id
LEFT JOIN threads t ON t.id = r.target_thread_id
WHERE
  (r.target_post_id IS NOT NULL AND p.id IS NULL)
  OR (r.target_thread_id IS NOT NULL AND t.id IS NULL)
  OR (r.target_post_id IS NULL AND r.target_thread_id IS NULL)
ORDER BY r.created_at DESC
LIMIT 50;

\echo
\echo === 10. Users with case-insensitive duplicate handles ===
-- The handle column has a UNIQUE constraint but it's case-sensitive on
-- some PG configurations. Lowercase-collision detection here.
SELECT lower(handle) AS handle_lc, COUNT(*) AS dup_count, array_agg(id) AS user_ids
FROM users
GROUP BY lower(handle)
HAVING COUNT(*) > 1
ORDER BY dup_count DESC
LIMIT 50;

\echo
\echo === 11. Users with NULL handle (NOT NULL constraint should prevent) ===
SELECT id, created_at FROM users WHERE handle IS NULL OR handle = '' LIMIT 50;

\echo
\echo === 12. Threads marked Removed but still attached to posts (stale) ===
-- Posts under a Removed thread should themselves be removed. Surface
-- mismatches so the moderator can clean up.
SELECT t.id AS thread_id, t.title, t.mod_state, p.id AS post_id, p.mod_state AS post_state
FROM threads t
JOIN posts p ON p.thread_id = t.id
WHERE t.mod_state = 'removed' AND p.mod_state != 'removed'
ORDER BY t.created_at DESC
LIMIT 50;

\echo
\echo === 13. Saved/hidden threads referencing missing thread rows ===
SELECT s.user_id, s.thread_id, 'saved_threads' AS table_name
FROM saved_threads s
LEFT JOIN threads t ON t.id = s.thread_id
WHERE t.id IS NULL
UNION ALL
SELECT h.user_id, h.thread_id, 'hidden_threads'
FROM hidden_threads h
LEFT JOIN threads t ON t.id = h.thread_id
WHERE t.id IS NULL
LIMIT 50;

\echo
\echo === 14. Votes referencing missing posts ===
SELECT v.user_id, v.post_id, v.value
FROM votes v
LEFT JOIN posts p ON p.id = v.post_id
WHERE p.id IS NULL
LIMIT 50;

\echo
\echo === 15. Thread-tags referencing missing threads ===
SELECT tt.thread_id, tt.tag, tt.position
FROM thread_tags tt
LEFT JOIN threads t ON t.id = tt.thread_id
WHERE t.id IS NULL
LIMIT 50;

\echo
\echo === 16. Notifications referencing missing recipient / actor ===
SELECT n.id, n.recipient_user_id, n.actor_id, n.kind
FROM notifications n
LEFT JOIN users r ON r.id = n.recipient_user_id
LEFT JOIN users a ON a.id = n.actor_id
WHERE r.id IS NULL
   OR (n.actor_id IS NOT NULL AND a.id IS NULL)
LIMIT 50;

\echo
\echo === 17. Founder count (sanity check — should be small, typically 1) ===
SELECT COUNT(*) AS founder_count, array_agg(handle) AS handles
FROM users
WHERE role = 'founder' AND deactivated_at IS NULL;

\echo
\echo === Integrity check complete. ===
\echo Re-run after each batch of cleanup. Most queries should report 0 rows
\echo on a healthy database.
