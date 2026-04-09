-- Add awaiting_approval + rejected states and transitions to the step lifecycle FSM

DO $$
DECLARE
    v_machine_id UUID;
    v_running_id UUID;
    v_awaiting_id UUID;
    v_rejected_id UUID;
    v_failed_id UUID;
BEGIN
    -- Look up existing machine and states
    SELECT abstract_machine__id INTO v_machine_id
    FROM lib_fsm.abstract_state_machine
    WHERE name = 'step_lifecycle';

    SELECT abstract_state__id INTO v_running_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'running';

    SELECT abstract_state__id INTO v_failed_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'failed';

    -- Create the new states
    v_awaiting_id := lib_fsm.abstract_state_create(v_machine_id, 'awaiting_approval', 'Waiting for human approval', FALSE);
    v_rejected_id := lib_fsm.abstract_state_create(v_machine_id, 'rejected', 'Human rejected the approval', FALSE);

    -- Running -> AwaitingApproval (suspended)
    PERFORM lib_fsm.abstract_transition_create(v_running_id, 'suspended', v_awaiting_id, 'Step suspended for approval');

    -- AwaitingApproval -> Running (resumed after approval)
    PERFORM lib_fsm.abstract_transition_create(v_awaiting_id, 'resumed', v_running_id, 'Step resumed after approval');

    -- AwaitingApproval -> Rejected (human rejected)
    PERFORM lib_fsm.abstract_transition_create(v_awaiting_id, 'rejected', v_rejected_id, 'Step rejected by human');

    -- AwaitingApproval -> Failed (technical failure)
    PERFORM lib_fsm.abstract_transition_create(v_awaiting_id, 'failed', v_failed_id, 'Step failed during approval');
END;
$$;
