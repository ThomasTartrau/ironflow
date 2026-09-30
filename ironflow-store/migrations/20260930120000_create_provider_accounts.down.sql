DROP INDEX IF EXISTS ironflow.idx_steps_account_id;
ALTER TABLE ironflow.steps DROP COLUMN IF EXISTS account_id;
DROP TABLE IF EXISTS ironflow.provider_account_usage;
DROP TABLE IF EXISTS ironflow.provider_account_windows;
DROP TABLE IF EXISTS ironflow.provider_accounts;
