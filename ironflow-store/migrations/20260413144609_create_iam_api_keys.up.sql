CREATE TABLE iam.api_keys (
    id              UUID            PRIMARY KEY,
    user_id         UUID            NOT NULL REFERENCES iam.users(id) ON DELETE CASCADE,
    name            VARCHAR(255)    NOT NULL,
    key_hash        TEXT            NOT NULL,
    key_prefix      VARCHAR(12)     NOT NULL,
    scopes          TEXT[]          NOT NULL DEFAULT '{}',
    is_active       BOOLEAN         NOT NULL DEFAULT TRUE,
    expires_at      TIMESTAMPTZ,
    last_used_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);
