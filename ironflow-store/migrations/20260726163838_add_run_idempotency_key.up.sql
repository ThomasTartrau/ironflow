ALTER TABLE ironflow.runs ADD COLUMN idempotency_key TEXT;

CREATE UNIQUE INDEX idx_runs_idempotency_key
    ON ironflow.runs (idempotency_key)
    WHERE idempotency_key IS NOT NULL;
