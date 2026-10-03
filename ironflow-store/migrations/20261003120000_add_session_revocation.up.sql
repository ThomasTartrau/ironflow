-- Server-side session state: a per-user token version checked on every request,
-- and single-use refresh tokens stored as SHA-256 hashes (never the token itself).
ALTER TABLE iam.users ADD COLUMN IF NOT EXISTS token_version BIGINT NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS iam.refresh_tokens (
    token_hash  TEXT        PRIMARY KEY,
    user_id     UUID        NOT NULL REFERENCES iam.users (id) ON DELETE CASCADE,
    expires_at  TIMESTAMPTZ NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_refresh_tokens_user_id ON iam.refresh_tokens (user_id);
