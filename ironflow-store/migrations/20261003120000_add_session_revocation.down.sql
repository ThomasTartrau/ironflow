DROP TABLE IF EXISTS iam.refresh_tokens;
ALTER TABLE iam.users DROP COLUMN IF EXISTS token_version;
