-- Seed a disposable Viceden database with a deterministic 10k-post fixture.
--
-- Usage:
--   psql "$VICEDEN_DATABASE__URL" -f scripts/seed-10k-posts.sql
--
-- Safety: refuses to run unless the current database name contains "test".

\set ON_ERROR_STOP on

DO $$
BEGIN
    IF current_database() NOT LIKE '%test%' THEN
        RAISE EXCEPTION 'refusing to seed non-test database: %', current_database();
    END IF;
END
$$;

INSERT INTO boards (slug, name, ttl_days, description, downvotes_enabled)
VALUES
    ('general', 'General', 30, 'Perf fixture board', true),
    ('hobbies', 'Hobbies', 30, 'Perf fixture board', true),
    ('careers', 'Careers', 30, 'Perf fixture board', false)
ON CONFLICT (slug) DO UPDATE
SET name = EXCLUDED.name,
    ttl_days = EXCLUDED.ttl_days,
    description = EXCLUDED.description,
    downvotes_enabled = EXCLUDED.downvotes_enabled;

INSERT INTO users (id, handle, password_hash, role, last_known_ip)
VALUES
    ('00000000-0000-0000-0000-000000000001', 'perffounder', 'perf-hash', 'founder', '127.0.0.1'),
    ('00000000-0000-0000-0000-000000000002', 'perfmember', 'perf-hash', 'member', '127.0.0.1')
ON CONFLICT (id) DO NOTHING;

INSERT INTO users (id, handle, password_hash, role, last_known_ip)
SELECT
    (substr(md5('perf-voter-' || n), 1, 8) || '-' ||
     substr(md5('perf-voter-' || n), 9, 4) || '-' ||
     substr(md5('perf-voter-' || n), 13, 4) || '-' ||
     substr(md5('perf-voter-' || n), 17, 4) || '-' ||
     substr(md5('perf-voter-' || n), 21, 12))::uuid,
    'pv' || lpad(n::text, 3, '0'),
    'perf-hash',
    'member',
    '127.0.0.1'
FROM generate_series(1, 20) AS n
ON CONFLICT (id) DO NOTHING;

INSERT INTO invites (code, label, inviter_id, created_at, max_redemptions, redemption_count)
VALUES ('perf-seed-invite', 'perf seed', '00000000-0000-0000-0000-000000000001', now(), 100, 0)
ON CONFLICT (code) DO NOTHING;

INSERT INTO age_attestations (user_id, dob, attested_ip, attested_ua, invite_code)
VALUES
    ('00000000-0000-0000-0000-000000000001', '1990-01-01', '127.0.0.1', 'perf-seed', 'perf-seed-invite'),
    ('00000000-0000-0000-0000-000000000002', '1990-01-01', '127.0.0.1', 'perf-seed', 'perf-seed-invite')
ON CONFLICT (user_id) DO NOTHING;

INSERT INTO age_attestations (user_id, dob, attested_ip, attested_ua, invite_code)
SELECT
    (substr(md5('perf-voter-' || n), 1, 8) || '-' ||
     substr(md5('perf-voter-' || n), 9, 4) || '-' ||
     substr(md5('perf-voter-' || n), 13, 4) || '-' ||
     substr(md5('perf-voter-' || n), 17, 4) || '-' ||
     substr(md5('perf-voter-' || n), 21, 12))::uuid,
    '1990-01-01',
    '127.0.0.1',
    'perf-seed',
    'perf-seed-invite'
FROM generate_series(1, 20) AS n
ON CONFLICT (user_id) DO NOTHING;

WITH thread_numbers AS (
    SELECT n, (ARRAY['general', 'hobbies', 'careers'])[1 + (n % 3)] AS board_slug
    FROM generate_series(1, 1000) AS n
)
INSERT INTO threads (id, board_slug, user_id, title, created_at, expires_at, mod_state, mod_state_changed_at)
SELECT
    (substr(md5('perf-thread-' || n), 1, 8) || '-' ||
     substr(md5('perf-thread-' || n), 9, 4) || '-' ||
     substr(md5('perf-thread-' || n), 13, 4) || '-' ||
     substr(md5('perf-thread-' || n), 17, 4) || '-' ||
     substr(md5('perf-thread-' || n), 21, 12))::uuid,
    board_slug,
    CASE WHEN n % 11 = 0
        THEN '00000000-0000-0000-0000-000000000001'::uuid
        ELSE '00000000-0000-0000-0000-000000000002'::uuid
    END,
    'perf thread ' || n,
    now() - (n || ' minutes')::interval,
    now() + interval '30 days',
    'visible',
    now()
FROM thread_numbers
ON CONFLICT (id) DO NOTHING;

WITH post_numbers AS (
    SELECT thread_n, post_n
    FROM generate_series(1, 1000) AS thread_n
    CROSS JOIN generate_series(1, 10) AS post_n
)
INSERT INTO posts (id, thread_id, user_id, body, created_at, state, mod_state, mod_state_changed_at)
SELECT
    (substr(md5('perf-post-' || thread_n || '-' || post_n), 1, 8) || '-' ||
     substr(md5('perf-post-' || thread_n || '-' || post_n), 9, 4) || '-' ||
     substr(md5('perf-post-' || thread_n || '-' || post_n), 13, 4) || '-' ||
     substr(md5('perf-post-' || thread_n || '-' || post_n), 17, 4) || '-' ||
     substr(md5('perf-post-' || thread_n || '-' || post_n), 21, 12))::uuid,
    (substr(md5('perf-thread-' || thread_n), 1, 8) || '-' ||
     substr(md5('perf-thread-' || thread_n), 9, 4) || '-' ||
     substr(md5('perf-thread-' || thread_n), 13, 4) || '-' ||
     substr(md5('perf-thread-' || thread_n), 17, 4) || '-' ||
     substr(md5('perf-thread-' || thread_n), 21, 12))::uuid,
    CASE WHEN post_n % 7 = 0
        THEN '00000000-0000-0000-0000-000000000001'::uuid
        ELSE '00000000-0000-0000-0000-000000000002'::uuid
    END,
    'perf body thread ' || thread_n || ' post ' || post_n ||
        CASE WHEN thread_n % 50 = 0 THEN ' searchneedle' ELSE '' END,
    now() - ((thread_n * 10 + post_n) || ' seconds')::interval,
    0,
    'visible',
    now()
FROM post_numbers
ON CONFLICT (id) DO NOTHING;

INSERT INTO thread_tags (thread_id, tag, position)
SELECT
    (substr(md5('perf-thread-' || n), 1, 8) || '-' ||
     substr(md5('perf-thread-' || n), 9, 4) || '-' ||
     substr(md5('perf-thread-' || n), 13, 4) || '-' ||
     substr(md5('perf-thread-' || n), 17, 4) || '-' ||
     substr(md5('perf-thread-' || n), 21, 12))::uuid,
    'tag' || (n % 10),
    0
FROM generate_series(1, 1000) AS n
ON CONFLICT (thread_id, tag) DO NOTHING;

INSERT INTO votes (post_id, user_id, value)
SELECT
    (substr(md5('perf-post-' || thread_n || '-1'), 1, 8) || '-' ||
     substr(md5('perf-post-' || thread_n || '-1'), 9, 4) || '-' ||
     substr(md5('perf-post-' || thread_n || '-1'), 13, 4) || '-' ||
     substr(md5('perf-post-' || thread_n || '-1'), 17, 4) || '-' ||
     substr(md5('perf-post-' || thread_n || '-1'), 21, 12))::uuid,
    (substr(md5('perf-voter-' || voter_n), 1, 8) || '-' ||
     substr(md5('perf-voter-' || voter_n), 9, 4) || '-' ||
     substr(md5('perf-voter-' || voter_n), 13, 4) || '-' ||
     substr(md5('perf-voter-' || voter_n), 17, 4) || '-' ||
     substr(md5('perf-voter-' || voter_n), 21, 12))::uuid,
    CASE WHEN voter_n % 5 = 0 THEN -1 ELSE 1 END
FROM generate_series(1, 1000) AS thread_n
CROSS JOIN generate_series(1, 5) AS voter_n
ON CONFLICT (post_id, user_id) DO NOTHING;

VACUUM (ANALYZE) threads;
VACUUM (ANALYZE) posts;
VACUUM (ANALYZE) votes;
VACUUM (ANALYZE) thread_tags;

SELECT
    (SELECT count(*) FROM threads WHERE title LIKE 'perf thread %') AS perf_threads,
    (SELECT count(*) FROM posts WHERE body LIKE 'perf body thread %') AS perf_posts,
    (SELECT count(*) FROM votes v JOIN posts p ON p.id = v.post_id WHERE p.body LIKE 'perf body thread %') AS perf_votes;
