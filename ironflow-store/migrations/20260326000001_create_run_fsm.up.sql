-- Define the run lifecycle FSM

DO $$
DECLARE
    v_machine_id UUID;
    v_pending_id UUID;
    v_running_id UUID;
    v_completed_id UUID;
    v_failed_id UUID;
    v_retrying_id UUID;
    v_cancelled_id UUID;
BEGIN
    -- Create the abstract machine
    v_machine_id := lib_fsm.abstract_machine_create('run_lifecycle', 'Workflow run lifecycle');

    -- Create states
    v_pending_id := lib_fsm.abstract_state_create(v_machine_id, 'pending', 'Waiting for execution', TRUE);
    v_running_id := lib_fsm.abstract_state_create(v_machine_id, 'running', 'Executing steps', FALSE);
    v_completed_id := lib_fsm.abstract_state_create(v_machine_id, 'completed', 'All steps succeeded', FALSE);
    v_failed_id := lib_fsm.abstract_state_create(v_machine_id, 'failed', 'Execution failed', FALSE);
    v_retrying_id := lib_fsm.abstract_state_create(v_machine_id, 'retrying', 'Waiting to retry', FALSE);
    v_cancelled_id := lib_fsm.abstract_state_create(v_machine_id, 'cancelled', 'Cancelled by user', FALSE);

    -- Pending transitions
    PERFORM lib_fsm.abstract_transition_create(v_pending_id, 'picked_up', v_running_id, 'Worker picked up the run');
    PERFORM lib_fsm.abstract_transition_create(v_pending_id, 'cancel_requested', v_cancelled_id, 'User requested cancellation');

    -- Running transitions
    PERFORM lib_fsm.abstract_transition_create(v_running_id, 'all_steps_completed', v_completed_id, 'All steps executed successfully');
    PERFORM lib_fsm.abstract_transition_create(v_running_id, 'step_failed', v_failed_id, 'Step failed and no retries available');
    PERFORM lib_fsm.abstract_transition_create(v_running_id, 'step_failed_retryable', v_retrying_id, 'Step failed but retries available');
    PERFORM lib_fsm.abstract_transition_create(v_running_id, 'cancel_requested', v_cancelled_id, 'User requested cancellation');

    -- Retrying transitions
    PERFORM lib_fsm.abstract_transition_create(v_retrying_id, 'retry_started', v_running_id, 'Retry started');
    PERFORM lib_fsm.abstract_transition_create(v_retrying_id, 'max_retries_exceeded', v_failed_id, 'Max retries exceeded');
    PERFORM lib_fsm.abstract_transition_create(v_retrying_id, 'cancel_requested', v_cancelled_id, 'User requested cancellation');
END;
$$;
