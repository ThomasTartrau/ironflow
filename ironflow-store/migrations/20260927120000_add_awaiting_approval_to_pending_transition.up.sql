-- Add the `awaiting_approval` state and its transitions to the run_lifecycle
-- FSM, so a run can suspend on an approval, human input or escalation gate and
-- leave it through every path `RunStatus::can_transition_to` allows: resumed
-- in place, requeued for a worker (ExecutionMode::Workers), rejected or
-- cancelled.

DO $$
DECLARE
    v_machine_id UUID;
    v_pending_id UUID;
    v_running_id UUID;
    v_failed_id UUID;
    v_cancelled_id UUID;
    v_awaiting_id UUID;
BEGIN
    SELECT abstract_machine__id INTO STRICT v_machine_id
    FROM lib_fsm.abstract_state_machine
    WHERE name = 'run_lifecycle';

    SELECT abstract_state__id INTO STRICT v_pending_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'pending';

    SELECT abstract_state__id INTO STRICT v_running_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'running';

    SELECT abstract_state__id INTO STRICT v_failed_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'failed';

    SELECT abstract_state__id INTO STRICT v_cancelled_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'cancelled';

    v_awaiting_id := lib_fsm.abstract_state_create(
        v_machine_id,
        'awaiting_approval',
        'Suspended on an approval, human input or escalation gate',
        FALSE
    );

    PERFORM lib_fsm.abstract_transition_create(
        v_running_id,
        'approval_requested',
        v_awaiting_id,
        'A step suspended the run on a gate'
    );

    PERFORM lib_fsm.abstract_transition_create(
        v_awaiting_id,
        'approved',
        v_running_id,
        'Gate resolved, run resumed in place'
    );

    PERFORM lib_fsm.abstract_transition_create(
        v_awaiting_id,
        'requeued_for_worker',
        v_pending_id,
        'Gate resolved, run requeued for a worker'
    );

    PERFORM lib_fsm.abstract_transition_create(
        v_awaiting_id,
        'rejected',
        v_failed_id,
        'Gate rejected'
    );

    PERFORM lib_fsm.abstract_transition_create(
        v_awaiting_id,
        'cancel_requested',
        v_cancelled_id,
        'User requested cancellation'
    );
END;
$$;
