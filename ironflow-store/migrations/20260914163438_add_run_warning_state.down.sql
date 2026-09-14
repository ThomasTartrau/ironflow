-- Remove the `warning` state and `completed_with_warnings` transition
-- from the run_lifecycle FSM.
-- Instances currently in the `warning` state are moved to `completed`.

DO $$
DECLARE
    v_machine_id UUID;
    v_warning_id UUID;
    v_completed_id UUID;
BEGIN
    SELECT abstract_machine__id INTO STRICT v_machine_id
    FROM lib_fsm.abstract_state_machine
    WHERE name = 'run_lifecycle';

    SELECT abstract_state__id INTO STRICT v_warning_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'warning';

    SELECT abstract_state__id INTO STRICT v_completed_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'completed';

    -- Move any instance sitting on `warning` to `completed`.
    -- This is a rollback administrative operation; the FSM transition
    -- mechanism cannot be used to remove an abstract state.
    UPDATE lib_fsm.state_machine
    SET abstract_state__id = v_completed_id, updated_at = NOW()
    WHERE abstract_state__id = v_warning_id;

    -- Remove event history entries that reference the `warning` state
    DELETE FROM lib_fsm.state_machine_event
    WHERE abstract_state__id = v_warning_id;

    -- Remove transitions pointing to or from `warning`
    DELETE FROM lib_fsm.abstract_transition
    WHERE from_abstract_state__id = v_warning_id
       OR to_abstract_state__id = v_warning_id;

    -- Remove the state itself
    DELETE FROM lib_fsm.abstract_state
    WHERE abstract_state__id = v_warning_id;
END;
$$;
