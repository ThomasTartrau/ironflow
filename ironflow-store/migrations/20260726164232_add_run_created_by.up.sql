-- Track the principal that created a run.
--
-- Both columns are NULL for cron, webhook and programmatic triggers.
-- A run created through an API key fills both: the key and its owner, so
-- filtering by user also covers runs triggered by that user's keys.
--
-- No FOREIGN KEY on purpose: deleting a user or a key must not erase the
-- authorship of past runs (same choice as ironflow.audit_logs.user_id).
ALTER TABLE ironflow.runs
    ADD COLUMN created_by_user_id UUID,
    ADD COLUMN created_by_api_key_id UUID;

-- Composite index: serves both the author filter and the created_at ordering
-- used by list_runs, and the author filter alone through its leading column.
CREATE INDEX idx_runs_created_by_user
    ON ironflow.runs (created_by_user_id, created_at DESC);
