-- Provider kind a Sleeping run waits for when every targeted Provider Account
-- is rate limited. NULL in every other state. Plain TEXT, not a PG enum: kinds are
-- an open registry (typed as ProviderKind in Rust). Nullable with no default: no
-- table rewrite. The partial index serves the wake-up on an account change.
ALTER TABLE ironflow.runs ADD COLUMN capacity_wait_kind TEXT;
CREATE INDEX idx_runs_capacity_wait_kind ON ironflow.runs (capacity_wait_kind) WHERE capacity_wait_kind IS NOT NULL;
