-- Add the `signal_received` transition to the run_lifecycle FSM, so a run
-- suspended by `ctx.wait_for_signal` goes back to the queue as soon as a
-- matching signal resolves its step, without waiting for `scheduled_at`.

DO $$
DECLARE
    v_machine_id UUID;
    v_pending_id UUID;
    v_sleeping_id UUID;
BEGIN
    SELECT abstract_machine__id INTO STRICT v_machine_id
    FROM lib_fsm.abstract_state_machine
    WHERE name = 'run_lifecycle';

    SELECT abstract_state__id INTO STRICT v_pending_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'pending';

    SELECT abstract_state__id INTO STRICT v_sleeping_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'sleeping';

    PERFORM lib_fsm.abstract_transition_create(
        v_sleeping_id,
        'signal_received',
        v_pending_id,
        'A signal resumed the run'
    );
END;
$$;
