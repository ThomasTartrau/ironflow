-- Delegation of approval power from one user to another for a time window.

CREATE TABLE ironflow.approval_delegations (
    id              UUID        PRIMARY KEY,
    from_user_id    UUID        NOT NULL REFERENCES iam.users(id) ON DELETE CASCADE,
    to_user_id      UUID        NOT NULL REFERENCES iam.users(id) ON DELETE CASCADE,
    valid_from      TIMESTAMPTZ NOT NULL,
    valid_until     TIMESTAMPTZ NOT NULL,
    workflow_filter TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT approval_delegations_period CHECK (valid_until > valid_from),
    CONSTRAINT approval_delegations_distinct_users CHECK (from_user_id <> to_user_id)
);

CREATE INDEX idx_approval_delegations_to_user
    ON ironflow.approval_delegations (to_user_id, valid_until);
CREATE INDEX idx_approval_delegations_from_user
    ON ironflow.approval_delegations (from_user_id, valid_until);
