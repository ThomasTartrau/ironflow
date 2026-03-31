CREATE SCHEMA IF NOT EXISTS ironflow;

CREATE TABLE IF NOT EXISTS ironflow.runs (
    id UUID PRIMARY KEY,
    workflow_name VARCHAR(255) NOT NULL,
    state_machine__id UUID NOT NULL REFERENCES lib_fsm.state_machine (state_machine__id) ON DELETE RESTRICT,
    trigger JSONB NOT NULL DEFAULT '{}',
    payload JSONB NOT NULL DEFAULT '{}',
    error TEXT,
    retry_count INT NOT NULL DEFAULT 0,
    max_retries INT NOT NULL DEFAULT 3,
    cost_usd NUMERIC(12, 6) NOT NULL DEFAULT 0,
    duration_ms BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ
);

CREATE INDEX idx_runs_state_machine ON ironflow.runs (state_machine__id);
CREATE INDEX idx_runs_workflow_name ON ironflow.runs (workflow_name);
CREATE INDEX idx_runs_created_at ON ironflow.runs (created_at DESC);
