CREATE TABLE ironflow.run_logs (
    id         UUID        PRIMARY KEY,
    run_id     UUID        NOT NULL REFERENCES ironflow.runs(id) ON DELETE CASCADE,
    step_id    UUID        NOT NULL,
    step_name  TEXT        NOT NULL,
    stream     TEXT        NOT NULL,
    line       TEXT        NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_run_logs_run_id_id ON ironflow.run_logs (run_id, id);
CREATE INDEX idx_run_logs_step_id ON ironflow.run_logs (step_id);
