-- Track which run attempt produced each step, so retries keep prior attempts readable.
ALTER TABLE ironflow.steps ADD COLUMN attempt INTEGER NOT NULL DEFAULT 1;

CREATE INDEX idx_steps_run_attempt ON ironflow.steps (run_id, attempt, position);

-- Align the column default with the application default (NewRun::max_retries = 0),
-- so a run never retries unless it was explicitly asked to.
ALTER TABLE ironflow.runs ALTER COLUMN max_retries SET DEFAULT 0;
