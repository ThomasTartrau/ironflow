-- Agent roles: cost counter of a role over its period, shared by every worker.
CREATE TABLE ironflow.agent_roles (
    id               UUID PRIMARY KEY,
    name             TEXT NOT NULL UNIQUE CHECK (name ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    code_cap_usd     NUMERIC(12, 6) CHECK (code_cap_usd IS NULL OR code_cap_usd >= 0),
    operator_cap_usd NUMERIC(12, 6) CHECK (operator_cap_usd IS NULL OR operator_cap_usd >= 0),
    period           TEXT NOT NULL CHECK (period IN ('day', 'month')),
    period_start     TIMESTAMPTZ NOT NULL,
    spent_usd        NUMERIC(12, 6) NOT NULL DEFAULT 0 CHECK (spent_usd >= 0),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Room held by the agent steps of a role while they run.
CREATE TABLE ironflow.agent_role_reservations (
    id         UUID PRIMARY KEY,
    role_id    UUID NOT NULL REFERENCES ironflow.agent_roles(id) ON DELETE CASCADE,
    amount_usd NUMERIC(12, 6) NOT NULL CHECK (amount_usd >= 0),
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_agent_role_reservations_role_expires
    ON ironflow.agent_role_reservations (role_id, expires_at);
