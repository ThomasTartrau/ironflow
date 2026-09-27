-- Remove the `awaiting_approval` state and its transitions from the
-- run_lifecycle FSM.
-- Runs suspended on a gate are moved to `failed`: without the state the gate
-- can no longer be resolved, and requeuing them would replay the handler up to
-- the same impossible transition.

DO $$
DECLARE
    v_machine_id UUID;
    v_awaiting_id UUID;
    v_failed_id UUID;
BEGIN
    SELECT abstract_machine__id INTO STRICT v_machine_id
    FROM lib_fsm.abstract_state_machine
    WHERE name = 'run_lifecycle';

    SELECT abstract_state__id INTO STRICT v_awaiting_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'awaiting_approval';

    SELECT abstract_state__id INTO STRICT v_failed_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'failed';

    UPDATE ironflow.runs r
    SET error = COALESCE(r.error, 'Run was awaiting approval when the awaiting_approval state was rolled back'),
        completed_at = COALESCE(r.completed_at, NOW()),
        updated_at = NOW()
    FROM lib_fsm.state_machine sm
    WHERE sm.state_machine__id = r.state_machine__id
      AND sm.abstract_state__id = v_awaiting_id;

    -- This is a rollback administrative operation; the FSM transition
    -- mechanism cannot be used to remove an abstract state.
    UPDATE lib_fsm.state_machine
    SET abstract_state__id = v_failed_id, updated_at = NOW()
    WHERE abstract_state__id = v_awaiting_id;

    -- Remove event history entries that reference the `awaiting_approval` state
    DELETE FROM lib_fsm.state_machine_event
    WHERE abstract_state__id = v_awaiting_id;

    -- Remove transitions pointing to or from `awaiting_approval`
    DELETE FROM lib_fsm.abstract_transition
    WHERE from_abstract_state__id = v_awaiting_id
       OR to_abstract_state__id = v_awaiting_id;

    -- Remove the state itself
    DELETE FROM lib_fsm.abstract_state
    WHERE abstract_state__id = v_awaiting_id;
END;
$$;
