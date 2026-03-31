-- Define the step lifecycle FSM

DO $$
DECLARE
    v_machine_id UUID;
    v_pending_id UUID;
    v_running_id UUID;
    v_completed_id UUID;
    v_failed_id UUID;
    v_skipped_id UUID;
BEGIN
    -- Create the abstract machine
    v_machine_id := lib_fsm.abstract_machine_create('step_lifecycle', 'Workflow step lifecycle');

    -- Create states
    v_pending_id := lib_fsm.abstract_state_create(v_machine_id, 'pending', 'Waiting to execute', TRUE);
    v_running_id := lib_fsm.abstract_state_create(v_machine_id, 'running', 'Currently executing', FALSE);
    v_completed_id := lib_fsm.abstract_state_create(v_machine_id, 'completed', 'Executed successfully', FALSE);
    v_failed_id := lib_fsm.abstract_state_create(v_machine_id, 'failed', 'Execution failed', FALSE);
    v_skipped_id := lib_fsm.abstract_state_create(v_machine_id, 'skipped', 'Skipped (e.g. prior step failed)', FALSE);

    -- Pending transitions
    PERFORM lib_fsm.abstract_transition_create(v_pending_id, 'started', v_running_id, 'Step execution started');
    PERFORM lib_fsm.abstract_transition_create(v_pending_id, 'skipped', v_skipped_id, 'Step skipped due to conditions');

    -- Running transitions
    PERFORM lib_fsm.abstract_transition_create(v_running_id, 'succeeded', v_completed_id, 'Step execution succeeded');
    PERFORM lib_fsm.abstract_transition_create(v_running_id, 'failed', v_failed_id, 'Step execution failed');

    -- No further transitions from terminal states
END;
$$;
