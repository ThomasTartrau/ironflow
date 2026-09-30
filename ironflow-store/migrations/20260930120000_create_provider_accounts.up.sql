CREATE TABLE ironflow.provider_accounts (
    id              UUID PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    display_name    TEXT NOT NULL,
    kind            TEXT NOT NULL,
    secret_key      TEXT NOT NULL,
    enabled         BOOLEAN NOT NULL DEFAULT TRUE,
    priority        INTEGER NOT NULL DEFAULT 100,
    tags            TEXT[] NOT NULL DEFAULT '{}',
    max_concurrency INTEGER CHECK (max_concurrency IS NULL OR max_concurrency > 0),
    alert_threshold DOUBLE PRECISION NOT NULL DEFAULT 0.8
        CHECK (alert_threshold > 0 AND alert_threshold <= 1),
    expires_at      TIMESTAMPTZ NOT NULL,
    plan            TEXT,
    auth_failed_at  TIMESTAMPTZ,
    created_by      UUID REFERENCES iam.users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_provider_accounts_kind ON ironflow.provider_accounts (kind) WHERE enabled;

-- Latest reading per window. `model_scope = ''` means the window applies to every model.
CREATE TABLE ironflow.provider_account_windows (
    account_id  UUID NOT NULL REFERENCES ironflow.provider_accounts(id) ON DELETE CASCADE,
    window_name TEXT NOT NULL,
    model_scope TEXT NOT NULL DEFAULT '',
    utilization DOUBLE PRECISION NOT NULL,
    resets_at   TIMESTAMPTZ,
    status      TEXT NOT NULL,
    observed_at TIMESTAMPTZ NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (account_id, window_name, model_scope)
);

-- Every observation, for the usage history.
CREATE TABLE ironflow.provider_account_usage (
    id          UUID PRIMARY KEY,
    account_id  UUID NOT NULL REFERENCES ironflow.provider_accounts(id) ON DELETE CASCADE,
    window_name TEXT NOT NULL,
    model_scope TEXT NOT NULL DEFAULT '',
    utilization DOUBLE PRECISION NOT NULL,
    resets_at   TIMESTAMPTZ,
    status      TEXT NOT NULL,
    observed_at TIMESTAMPTZ NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_provider_account_usage_account_observed
    ON ironflow.provider_account_usage (account_id, observed_at);
CREATE INDEX idx_provider_account_usage_observed
    ON ironflow.provider_account_usage (observed_at);

ALTER TABLE ironflow.steps
    ADD COLUMN account_id UUID REFERENCES ironflow.provider_accounts(id) ON DELETE SET NULL;

CREATE INDEX idx_steps_account_id ON ironflow.steps (account_id) WHERE account_id IS NOT NULL;
