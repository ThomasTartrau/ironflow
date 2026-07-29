-- Files produced by a step and consumed by later steps.
--
-- Risk: safe. New table only, no existing column touched, no backfill.
--
-- This table is the source of truth: a blob with no row here is never listed
-- and never served. `storage_key` is derived from UUIDs only, never from `name`.
CREATE TABLE IF NOT EXISTS ironflow.step_artifacts (
    id UUID PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES ironflow.runs (id) ON DELETE CASCADE,
    step_id UUID NOT NULL REFERENCES ironflow.steps (id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    storage_key TEXT NOT NULL,
    content_type VARCHAR(255) NOT NULL,
    size_bytes BIGINT NOT NULL,
    sha256 CHAR(64) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (step_id, name),
    CHECK (size_bytes >= 0)
);

CREATE INDEX idx_step_artifacts_run_id ON ironflow.step_artifacts (run_id);
CREATE INDEX idx_step_artifacts_step_id ON ironflow.step_artifacts (step_id);
