ALTER TABLE reports
    ADD COLUMN IF NOT EXISTS priority INT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS reviewed_by UUID REFERENCES users(id),
    ADD COLUMN IF NOT EXISTS reviewed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS resolution_note TEXT;

CREATE INDEX IF NOT EXISTS idx_reports_resolved_priority_created
    ON reports (resolved, priority DESC, created_at DESC);

ALTER TABLE moderation_events
    ADD COLUMN IF NOT EXISTS report_id UUID REFERENCES reports(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS target_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS action TEXT NOT NULL DEFAULT 'legacy',
    ADD COLUMN IF NOT EXISTS note TEXT;

CREATE INDEX IF NOT EXISTS idx_moderation_events_report_created
    ON moderation_events (report_id, created_at DESC);
