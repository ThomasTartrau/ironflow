-- Optional exclusivity key: at most one non-terminal run may hold a given key.
-- Run status lives in lib_fsm, so exclusivity is enforced by a transactional
-- advisory lock in create_run, not by a unique index.
ALTER TABLE ironflow.runs ADD COLUMN concurrency_key TEXT;
CREATE INDEX idx_runs_concurrency_key ON ironflow.runs (concurrency_key) WHERE concurrency_key IS NOT NULL;
