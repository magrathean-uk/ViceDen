-- EXPLAIN hot forum read paths against a seeded disposable database.
--
-- Usage:
--   psql "$VICEDEN_DATABASE__URL" -f scripts/explain-hot-pages.sql
--
-- Threshold for new indexes:
--   - hot query execution time > 5ms, or
--   - sequential scan over more than 1000 rows.

\set ON_ERROR_STOP on

DO $$
BEGIN
    IF current_database() NOT LIKE '%test%' THEN
        RAISE EXCEPTION 'refusing to explain non-test database: %', current_database();
    END IF;
END
$$;

VACUUM (ANALYZE) threads;
VACUUM (ANALYZE) posts;
VACUUM (ANALYZE) votes;
VACUUM (ANALYZE) thread_tags;

\echo
\echo === hot path: board thread previews ===
EXPLAIN (ANALYZE, BUFFERS)
SELECT t.id, t.board_slug, t.user_id, u.handle, t.title, t.created_at, t.expires_at,
       t.mod_state, t.mod_state_reason, t.mod_state_changed_at,
       stats.score,
       stats.reply_count,
       stats.last_activity
FROM threads t
JOIN users u ON u.id = t.user_id
JOIN LATERAL (
    SELECT GREATEST(COUNT(p.id) - 1, 0)::BIGINT AS reply_count,
           COALESCE(MAX(p.created_at), t.created_at) AS last_activity,
           COALESCE(SUM(v.value), 0)::BIGINT AS score
    FROM posts p
    LEFT JOIN votes v ON v.post_id = p.id
    WHERE p.thread_id = t.id
) stats ON true
WHERE t.board_slug = 'general'
  AND t.expires_at > now()
  AND t.mod_state = 'visible'
ORDER BY t.created_at DESC
LIMIT 28;

\echo
\echo === hot path: thread page posts ===
EXPLAIN (ANALYZE, BUFFERS)
SELECT p.id, p.thread_id, p.user_id, u.handle, p.body, p.created_at,
       p.mod_state, p.mod_state_reason, p.mod_state_changed_at,
       COALESCE(SUM(v.value), 0) AS score
FROM posts p
JOIN users u ON u.id = p.user_id
LEFT JOIN votes v ON v.post_id = p.id
WHERE p.thread_id = (
    SELECT id FROM threads
    WHERE board_slug = 'general'
      AND expires_at > now()
      AND mod_state = 'visible'
    ORDER BY created_at DESC
    LIMIT 1
)
  AND p.mod_state = 'visible'
GROUP BY p.id, u.handle
ORDER BY p.created_at ASC;

\echo
\echo === hot path: front page visible ids ===
EXPLAIN (ANALYZE, BUFFERS)
SELECT id, board_slug, user_id, title, created_at, expires_at,
       mod_state, mod_state_reason, mod_state_changed_at
FROM threads
WHERE board_slug = 'general'
  AND expires_at > now()
  AND mod_state = 'visible'
ORDER BY created_at DESC
LIMIT 28;

\echo
\echo === hot path: search ===
EXPLAIN (ANALYZE, BUFFERS)
WITH q AS (SELECT websearch_to_tsquery('english', 'searchneedle') AS query)
SELECT *
FROM (
    SELECT t.id AS thread_id,
           NULL::uuid AS post_id,
           t.board_slug,
           t.title AS thread_title,
           t.user_id AS author_id,
           COALESCE(root.body, '') AS body_snippet,
           t.created_at,
           COALESCE(score.score, 0)::BIGINT AS score,
           'thread' AS kind
    FROM threads t
    JOIN q ON true
    LEFT JOIN LATERAL (
        SELECT body FROM posts WHERE thread_id = t.id ORDER BY created_at ASC LIMIT 1
    ) root ON true
    LEFT JOIN LATERAL (
        SELECT COALESCE(SUM(v.value), 0)::BIGINT AS score
        FROM posts p
        LEFT JOIN votes v ON v.post_id = p.id
        WHERE p.thread_id = t.id
    ) score ON true
    WHERE to_tsvector('english', t.title) @@ q.query
    UNION ALL
    SELECT t.id AS thread_id,
           NULL::uuid AS post_id,
           t.board_slug,
           t.title AS thread_title,
           t.user_id AS author_id,
           COALESCE(root.body, '') AS body_snippet,
           t.created_at,
           COALESCE(score.score, 0)::BIGINT AS score,
           'thread' AS kind
    FROM threads t
    JOIN (
        SELECT DISTINCT thread_id
        FROM thread_tags
        WHERE tag ILIKE '%' || 'searchneedle' || '%'
    ) tag_match ON tag_match.thread_id = t.id
    LEFT JOIN LATERAL (
        SELECT body FROM posts WHERE thread_id = t.id ORDER BY created_at ASC LIMIT 1
    ) root ON true
    LEFT JOIN LATERAL (
        SELECT COALESCE(SUM(v.value), 0)::BIGINT AS score
        FROM posts p
        LEFT JOIN votes v ON v.post_id = p.id
        WHERE p.thread_id = t.id
    ) score ON true
    UNION ALL
    SELECT t.id AS thread_id,
           p.id AS post_id,
           t.board_slug,
           t.title AS thread_title,
           p.user_id AS author_id,
           p.body AS body_snippet,
           p.created_at,
           COALESCE(SUM(v.value), 0)::BIGINT AS score,
           'post' AS kind
    FROM posts p
    JOIN threads t ON t.id = p.thread_id
    JOIN q ON true
    LEFT JOIN votes v ON v.post_id = p.id
    WHERE to_tsvector('english', p.body) @@ q.query
    GROUP BY t.id, p.id
) hits
ORDER BY created_at DESC, score DESC, thread_title ASC
LIMIT 50;
