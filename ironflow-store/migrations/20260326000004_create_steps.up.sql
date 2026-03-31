CREATE TABLE IF NOT EXISTS ironflow.steps (
    id UUID PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES ironflow.runs (id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    kind VARCHAR(50) NOT NULL,
    position INT NOT NULL,
    state_machine__id UUID NOT NULL REFERENCES lib_fsm.state_machine (state_machine__id) ON DELETE RESTRICT,
    input JSONB,
    output JSONB,
    error TEXT,
    duration_ms BIGINT NOT NULL DEFAULT 0,
    cost_usd NUMERIC(12, 6) NOT NULL DEFAULT 0,
    input_tokens BIGINT,
    output_tokens BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ
);

CREATE INDEX idx_steps_run_id ON ironflow.steps (run_id);
CREATE INDEX idx_steps_state_machine ON ironflow.steps (state_machine__id);
CREATE INDEX idx_steps_position ON ironflow.steps (run_id, position);
