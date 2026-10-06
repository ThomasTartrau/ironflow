DROP INDEX IF EXISTS ironflow.idx_runs_capacity_wait_kind;
ALTER TABLE ironflow.runs DROP COLUMN IF EXISTS capacity_wait_kind;
