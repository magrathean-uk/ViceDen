CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE INDEX IF NOT EXISTS idx_thread_tags_tag_trgm
    ON thread_tags USING GIN (tag gin_trgm_ops);

CREATE INDEX IF NOT EXISTS idx_threads_title_pattern_created
    ON threads (title text_pattern_ops, created_at DESC);
