CREATE TABLE IF NOT EXISTS ironflow.secrets (
    id UUID PRIMARY KEY,
    key TEXT NOT NULL UNIQUE,
    encrypted_value BYTEA NOT NULL,
    nonce BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
