-- Version of the encryption key each secret is encrypted with.
-- Existing rows predate key versioning: they were encrypted with the single
-- key configured via IRONFLOW_SECRET_KEY, which is interpreted as version 1.
ALTER TABLE ironflow.secrets ADD COLUMN key_version INTEGER NOT NULL DEFAULT 1;

CREATE INDEX IF NOT EXISTS idx_secrets_key_version
    ON ironflow.secrets (key_version);
