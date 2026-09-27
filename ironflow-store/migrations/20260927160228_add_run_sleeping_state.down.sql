-- Remove the `sleeping` state and its transitions from the run_lifecycle FSM.
-- Sleeping runs are moved to `failed`: without the state the pause can no
-- longer be taken, and requeuing them would replay the handler up to the same
-- impossible transition.

DO $$
DECLARE
    v_machine_id UUID;
    v_sleeping_id UUID;
    v_failed_id UUID;
BEGIN
    SELECT abstract_machine__id INTO STRICT v_machine_id
    FROM lib_fsm.abstract_state_machine
    WHERE name = 'run_lifecycle';

    SELECT abstract_state__id INTO STRICT v_sleeping_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'sleeping';

    SELECT abstract_state__id INTO STRICT v_failed_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'failed';

    UPDATE ironflow.runs r
    SET error = COALESCE(r.error, 'Run was sleeping when the sleeping state was rolled back'),
        completed_at = COALESCE(r.completed_at, NOW()),
        updated_at = NOW()
    FROM lib_fsm.state_machine sm
    WHERE sm.state_machine__id = r.state_machine__id
      AND sm.abstract_state__id = v_sleeping_id;

    -- This is a rollback administrative operation; the FSM transition
    -- mechanism cannot be used to remove an abstract state.
    UPDATE lib_fsm.state_machine
    SET abstract_state__id = v_failed_id, updated_at = NOW()
    WHERE abstract_state__id = v_sleeping_id;

    -- Remove event history entries that reference the `sleeping` state
    DELETE FROM lib_fsm.state_machine_event
    WHERE abstract_state__id = v_sleeping_id;

    -- Remove transitions pointing to or from `sleeping`
    DELETE FROM lib_fsm.abstract_transition
    WHERE from_abstract_state__id = v_sleeping_id
       OR to_abstract_state__id = v_sleeping_id;

    -- Remove the state itself
    DELETE FROM lib_fsm.abstract_state
    WHERE abstract_state__id = v_sleeping_id;
END;
$$;
