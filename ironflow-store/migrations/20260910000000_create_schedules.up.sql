CREATE TABLE ironflow.schedules (
    id              UUID PRIMARY KEY,
    workflow_name   TEXT NOT NULL,
    cron_expression TEXT NOT NULL,
    inputs          JSONB NOT NULL DEFAULT '{}',
    disabled_at     TIMESTAMPTZ,
    last_triggered_at TIMESTAMPTZ,
    next_trigger_at   TIMESTAMPTZ,
    created_by_user_id UUID NOT NULL REFERENCES iam.users(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_schedules_active_next ON ironflow.schedules (next_trigger_at)
    WHERE disabled_at IS NULL;
