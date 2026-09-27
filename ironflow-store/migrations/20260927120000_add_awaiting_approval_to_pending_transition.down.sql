-- Remove the awaiting_approval -> pending transition from the run_lifecycle
-- FSM. No state was created by the up migration, so nothing is reassigned.

DO $$
DECLARE
    v_machine_id UUID;
    v_awaiting_id UUID;
    v_pending_id UUID;
BEGIN
    SELECT abstract_machine__id INTO STRICT v_machine_id
    FROM lib_fsm.abstract_state_machine
    WHERE name = 'run_lifecycle';

    SELECT abstract_state__id INTO STRICT v_awaiting_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'awaiting_approval';

    SELECT abstract_state__id INTO STRICT v_pending_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'pending';

    DELETE FROM lib_fsm.abstract_transition
    WHERE from_abstract_state__id = v_awaiting_id
      AND to_abstract_state__id = v_pending_id
      AND event = 'requeued_for_worker';
END;
$$;
