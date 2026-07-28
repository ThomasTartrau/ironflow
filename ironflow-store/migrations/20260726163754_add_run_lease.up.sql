-- Attach a worker lease to runs so orphaned runs can be recovered.

ALTER TABLE ironflow.runs
    ADD COLUMN worker_id TEXT,
    ADD COLUMN lease_expires_at TIMESTAMPTZ;

-- The lease is only ever set while a run is `running` and is cleared as soon as
-- it leaves that state, so a partial index on the expiry column is equivalent to
-- indexing (status, lease_expires_at) -- runs has no status column, the state
-- lives in lib_fsm.state_machine.
CREATE INDEX idx_runs_lease_expires_at
    ON ironflow.runs (lease_expires_at)
    WHERE lease_expires_at IS NOT NULL;

-- Allow the reaper to requeue a run whose lease expired.
DO $$
DECLARE
    v_machine_id UUID;
    v_running_id UUID;
    v_pending_id UUID;
BEGIN
    SELECT abstract_machine__id INTO v_machine_id
    FROM lib_fsm.abstract_state_machine
    WHERE name = 'run_lifecycle';

    SELECT abstract_state__id INTO v_running_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'running';

    SELECT abstract_state__id INTO v_pending_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'pending';

    PERFORM lib_fsm.abstract_transition_create(v_running_id, 'lease_expired', v_pending_id, 'Worker lease expired, run requeued');
END;
$$;
