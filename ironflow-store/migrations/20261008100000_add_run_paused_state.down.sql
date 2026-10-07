-- Remove the operator pause (#172): the workflow_pauses table, the
-- resume_status column and the `paused` state of the run_lifecycle FSM.
-- Paused runs are moved to `failed`, like the sleeping rollback: without the
-- state they can neither hold nor go back where they were paused.

DROP TABLE IF EXISTS ironflow.workflow_pauses;

DO $$
DECLARE
    v_machine_id UUID;
    v_paused_id UUID;
    v_failed_id UUID;
BEGIN
    SELECT abstract_machine__id INTO STRICT v_machine_id
    FROM lib_fsm.abstract_state_machine
    WHERE name = 'run_lifecycle';

    SELECT abstract_state__id INTO STRICT v_paused_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'paused';

    SELECT abstract_state__id INTO STRICT v_failed_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'failed';

    UPDATE ironflow.runs r
    SET error = COALESCE(r.error, 'Run was paused when the paused state was rolled back'),
        completed_at = COALESCE(r.completed_at, NOW()),
        updated_at = NOW()
    FROM lib_fsm.state_machine sm
    WHERE sm.state_machine__id = r.state_machine__id
      AND sm.abstract_state__id = v_paused_id;

    -- This is a rollback administrative operation; the FSM transition
    -- mechanism cannot be used to remove an abstract state.
    UPDATE lib_fsm.state_machine
    SET abstract_state__id = v_failed_id, updated_at = NOW()
    WHERE abstract_state__id = v_paused_id;

    -- Remove event history entries that reference the `paused` state
    DELETE FROM lib_fsm.state_machine_event
    WHERE abstract_state__id = v_paused_id;

    -- Remove transitions pointing to or from `paused`
    DELETE FROM lib_fsm.abstract_transition
    WHERE from_abstract_state__id = v_paused_id
       OR to_abstract_state__id = v_paused_id;

    -- Remove the state itself
    DELETE FROM lib_fsm.abstract_state
    WHERE abstract_state__id = v_paused_id;
END;
$$;

ALTER TABLE ironflow.runs DROP COLUMN IF EXISTS resume_status;
