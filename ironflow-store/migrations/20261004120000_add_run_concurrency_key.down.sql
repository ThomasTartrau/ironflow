DROP INDEX IF EXISTS ironflow.idx_runs_concurrency_key;
ALTER TABLE ironflow.runs DROP COLUMN IF EXISTS concurrency_key;
