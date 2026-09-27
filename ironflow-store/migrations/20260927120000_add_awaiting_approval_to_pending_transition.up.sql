-- Add the awaiting_approval -> pending transition to the run_lifecycle FSM so
-- a run whose approval, human input or escalation just resolved can be
-- requeued for a worker instead of resumed in the API process
-- (ExecutionMode::Workers).

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

    PERFORM lib_fsm.abstract_transition_create(
        v_awaiting_id,
        'requeued_for_worker',
        v_pending_id,
        'Run requeued for a worker after approval, human input or escalation resolved the gate'
    );
END;
$$;
