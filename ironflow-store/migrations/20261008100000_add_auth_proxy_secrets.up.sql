-- Proxied secrets of ironflow-auth-proxy: a grant may now hold a generic
-- secret (credential_kind 'secret') next to a Claude credential. The secret
-- value stays in encrypted_credential; its name, injection mode and host
-- allowlist, none of them secret, live in secret_spec.
ALTER TABLE ironflow.auth_proxy_grants DROP CONSTRAINT IF EXISTS auth_proxy_grants_credential_kind_check;
ALTER TABLE ironflow.auth_proxy_grants ADD CONSTRAINT auth_proxy_grants_credential_kind_check CHECK (credential_kind IN ('oauth_token', 'api_key', 'secret'));
ALTER TABLE ironflow.auth_proxy_grants ADD COLUMN IF NOT EXISTS secret_spec JSONB NULL;
ALTER TABLE ironflow.auth_proxy_grants ADD CONSTRAINT auth_proxy_grants_secret_spec_check CHECK ((credential_kind = 'secret') = (secret_spec IS NOT NULL));
-- Tombstones of revoked grants, kept until the grant would have expired, so a
-- revoked token is told apart from an unknown one. No credential is kept.
CREATE TABLE IF NOT EXISTS ironflow.auth_proxy_revocations (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    expires_at BIGINT NOT NULL,
    revoked_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_auth_proxy_revocations_expires_at ON ironflow.auth_proxy_revocations (expires_at);
