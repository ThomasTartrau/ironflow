-- Grants of ironflow-auth-proxy, shared by its replicas. The id is the SHA-256
-- of the opaque token: the token itself is never stored. The credential is
-- AES-256-GCM encrypted with the key ring version `key_version`.
CREATE TABLE IF NOT EXISTS ironflow.auth_proxy_grants (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    step TEXT NOT NULL,
    expires_at BIGINT NOT NULL,
    credential_kind TEXT NOT NULL CHECK (credential_kind IN ('oauth_token', 'api_key')),
    encrypted_credential BYTEA NOT NULL,
    nonce BYTEA NOT NULL,
    key_version INT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_auth_proxy_grants_run_id ON ironflow.auth_proxy_grants (run_id);
CREATE INDEX IF NOT EXISTS idx_auth_proxy_grants_expires_at ON ironflow.auth_proxy_grants (expires_at);
