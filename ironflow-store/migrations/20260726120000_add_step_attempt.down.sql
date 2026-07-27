ALTER TABLE ironflow.runs ALTER COLUMN max_retries SET DEFAULT 3;

DROP INDEX IF EXISTS ironflow.idx_steps_run_attempt;

ALTER TABLE ironflow.steps DROP COLUMN attempt;
