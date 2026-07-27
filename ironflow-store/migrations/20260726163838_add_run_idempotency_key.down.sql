DROP INDEX IF EXISTS ironflow.idx_runs_idempotency_key;

ALTER TABLE ironflow.runs DROP COLUMN IF EXISTS idempotency_key;
