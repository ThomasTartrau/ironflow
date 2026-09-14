-- Add the `warning` state and the `completed_with_warnings` transition to
-- the run_lifecycle FSM so that runs with allow_failure steps that failed
-- can reach their terminal state.

DO $$
DECLARE
    v_machine_id UUID;
    v_running_id UUID;
    v_warning_id UUID;
BEGIN
    SELECT abstract_machine__id INTO STRICT v_machine_id
    FROM lib_fsm.abstract_state_machine
    WHERE name = 'run_lifecycle';

    SELECT abstract_state__id INTO STRICT v_running_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'running';

    v_warning_id := lib_fsm.abstract_state_create(
        v_machine_id,
        'warning',
        'Completed with allowed failures',
        FALSE
    );

    PERFORM lib_fsm.abstract_transition_create(
        v_running_id,
        'completed_with_warnings',
        v_warning_id,
        'All steps finished but at least one allow_failure step failed'
    );
END;
$$;
