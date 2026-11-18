ALTER TABLE reports
    ADD COLUMN IF NOT EXISTS queue_state TEXT NOT NULL DEFAULT 'new',
    ADD COLUMN IF NOT EXISTS assigned_to UUID REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS assigned_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_reports_open_priority_state_created
    ON reports (resolved, priority DESC, queue_state, created_at DESC);
