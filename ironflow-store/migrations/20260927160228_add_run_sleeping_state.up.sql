-- Add the `sleeping` state and its transitions to the run_lifecycle FSM, so a
-- run paused by a delay step can wait for `scheduled_at` and leave the pause
-- through every path `RunStatus::can_transition_to` allows: back to the queue
-- once the timer elapsed, or cancelled.

DO $$
DECLARE
    v_machine_id UUID;
    v_pending_id UUID;
    v_running_id UUID;
    v_cancelled_id UUID;
    v_sleeping_id UUID;
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

    SELECT abstract_state__id INTO STRICT v_cancelled_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'cancelled';

    v_sleeping_id := lib_fsm.abstract_state_create(
        v_machine_id,
        'sleeping',
        'Paused by a delay step until scheduled_at',
        FALSE
    );

    PERFORM lib_fsm.abstract_transition_create(
        v_running_id,
        'delay_started',
        v_sleeping_id,
        'A delay step paused the run'
    );

    PERFORM lib_fsm.abstract_transition_create(
        v_sleeping_id,
        'delay_elapsed',
        v_pending_id,
        'Wake-up timer elapsed, run requeued'
    );

    PERFORM lib_fsm.abstract_transition_create(
        v_sleeping_id,
        'cancel_requested',
        v_cancelled_id,
        'User requested cancellation'
    );
END;
$$;
