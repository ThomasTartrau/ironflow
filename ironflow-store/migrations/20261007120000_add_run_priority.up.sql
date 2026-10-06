-- Queue priority of a run: pick_next_pending serves due runs by priority DESC,
-- then created_at ASC. Constant default: no table rewrite; the CHECK is the
-- last defense behind the API and engine validation.
ALTER TABLE ironflow.runs ADD COLUMN priority SMALLINT NOT NULL DEFAULT 0 CHECK (priority BETWEEN -100 AND 100);
-- Not partial: run status lives in lib_fsm, not in a column of ironflow.runs,
-- so no predicate can restrict the index to pending and retrying runs.
CREATE INDEX idx_runs_priority_created_at ON ironflow.runs (priority DESC, created_at);
-- Priority given to every run a schedule creates.
ALTER TABLE ironflow.schedules ADD COLUMN priority SMALLINT NOT NULL DEFAULT 0 CHECK (priority BETWEEN -100 AND 100);
