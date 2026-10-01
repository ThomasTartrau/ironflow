-- Signals: external messages, named and keyed, that resume the runs waiting
-- for them through `ctx.wait_for_signal`.

CREATE TABLE ironflow.signals (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    key TEXT NOT NULL,
    payload JSONB NOT NULL,
    idempotency_id TEXT UNIQUE,
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_signals_name_key_received_at ON ironflow.signals (name, key, received_at);
CREATE INDEX idx_signals_received_at ON ironflow.signals (received_at);

-- Waiting signal steps, looked up by (name, key) on every delivery.
CREATE INDEX idx_steps_signal_waiters ON ironflow.steps ((input->>'name'), (input->>'key')) WHERE kind = 'signal';
