CREATE TABLE IF NOT EXISTS ironflow.step_dependencies (
    step_id    UUID NOT NULL REFERENCES ironflow.steps (id) ON DELETE CASCADE,
    depends_on UUID NOT NULL REFERENCES ironflow.steps (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (step_id, depends_on),
    CHECK (step_id != depends_on)
);

CREATE INDEX idx_step_deps_step_id ON ironflow.step_dependencies (step_id);
CREATE INDEX idx_step_deps_depends_on ON ironflow.step_dependencies (depends_on);
