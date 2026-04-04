-- Remove awaiting_approval + rejected states and their transitions from the step lifecycle FSM

DO $$
DECLARE
    v_machine_id UUID;
    v_awaiting_id UUID;
    v_rejected_id UUID;
BEGIN
    SELECT id INTO v_machine_id
    FROM lib_fsm.abstract_machines
    WHERE name = 'step_lifecycle';

    SELECT id INTO v_awaiting_id
    FROM lib_fsm.abstract_states
    WHERE abstract_machine__id = v_machine_id AND name = 'awaiting_approval';

    SELECT id INTO v_rejected_id
    FROM lib_fsm.abstract_states
    WHERE abstract_machine__id = v_machine_id AND name = 'rejected';

    -- Delete transitions involving the new states
    DELETE FROM lib_fsm.abstract_transitions
    WHERE from_abstract_state__id IN (v_awaiting_id, v_rejected_id)
       OR to_abstract_state__id IN (v_awaiting_id, v_rejected_id);

    -- Delete the states
    DELETE FROM lib_fsm.abstract_states
    WHERE id IN (v_awaiting_id, v_rejected_id);
END;
$$;
