ALTER TABLE ironflow.schedules DROP COLUMN IF EXISTS priority;
DROP INDEX IF EXISTS ironflow.idx_runs_priority_created_at;
ALTER TABLE ironflow.runs DROP COLUMN IF EXISTS priority;
