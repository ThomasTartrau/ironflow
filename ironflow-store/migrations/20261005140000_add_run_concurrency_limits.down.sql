DROP INDEX IF EXISTS ironflow.idx_runs_concurrency_limits;
ALTER TABLE ironflow.runs DROP COLUMN IF EXISTS concurrency_limits;
