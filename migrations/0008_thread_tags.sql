CREATE TABLE IF NOT EXISTS thread_tags (
    thread_id UUID NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (thread_id, tag)
);

CREATE INDEX IF NOT EXISTS idx_thread_tags_tag_thread
    ON thread_tags (tag, thread_id);
