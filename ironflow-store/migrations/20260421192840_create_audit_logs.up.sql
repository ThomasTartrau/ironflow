CREATE TABLE ironflow.audit_logs (
    id          UUID        PRIMARY KEY,
    event_type  TEXT        NOT NULL,
    payload     JSONB       NOT NULL,
    run_id      UUID,
    step_id     UUID,
    user_id     UUID,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
