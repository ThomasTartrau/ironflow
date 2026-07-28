-- Remove the worker lease from runs.

DO $$
DECLARE
    v_machine_id UUID;
    v_running_id UUID;
BEGIN
    SELECT abstract_machine__id INTO v_machine_id
    FROM lib_fsm.abstract_state_machine
    WHERE name = 'run_lifecycle';

    SELECT abstract_state__id INTO v_running_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'running';

    DELETE FROM lib_fsm.abstract_transition
    WHERE from_abstract_state__id = v_running_id AND event = 'lease_expired';
END;
$$;

DROP INDEX IF EXISTS ironflow.idx_runs_lease_expires_at;

ALTER TABLE ironflow.runs
    DROP COLUMN IF EXISTS worker_id,
    DROP COLUMN IF EXISTS lease_expires_at;
