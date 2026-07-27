DROP INDEX IF EXISTS ironflow.idx_runs_created_by_user;

ALTER TABLE ironflow.runs
    DROP COLUMN IF EXISTS created_by_api_key_id,
    DROP COLUMN IF EXISTS created_by_user_id;
