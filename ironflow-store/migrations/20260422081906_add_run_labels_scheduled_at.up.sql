ALTER TABLE ironflow.runs
    ADD COLUMN labels JSONB NOT NULL DEFAULT '{}',
    ADD COLUMN scheduled_at TIMESTAMPTZ;
