-- Operator pause (#172).
--
-- 1. The `paused` state of the run_lifecycle FSM, reachable from every
--    non-terminal state, and left back to the state the run was paused in
--    (kept in `runs.resume_status`), to `failed` or to `cancelled`.
-- 2. `ironflow.workflow_pauses`: a workflow listed there keeps its runs in the
--    queue, pick_next_pending skips them until the row is deleted.

DO $$
DECLARE
    v_machine_id UUID;
    v_pending_id UUID;
    v_running_id UUID;
    v_retrying_id UUID;
    v_sleeping_id UUID;
    v_awaiting_id UUID;
    v_failed_id UUID;
    v_cancelled_id UUID;
    v_paused_id UUID;
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

    SELECT abstract_state__id INTO STRICT v_retrying_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'retrying';

    SELECT abstract_state__id INTO STRICT v_sleeping_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'sleeping';

    SELECT abstract_state__id INTO STRICT v_awaiting_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'awaiting_approval';

    SELECT abstract_state__id INTO STRICT v_failed_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'failed';

    SELECT abstract_state__id INTO STRICT v_cancelled_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'cancelled';

    v_paused_id := lib_fsm.abstract_state_create(
        v_machine_id,
        'paused',
        'Paused by an operator until resumed or cancelled',
        FALSE
    );

    PERFORM lib_fsm.abstract_transition_create(
        v_pending_id, 'pause_requested', v_paused_id, 'Operator paused the run'
    );
    PERFORM lib_fsm.abstract_transition_create(
        v_retrying_id, 'pause_requested', v_paused_id, 'Operator paused the run'
    );
    PERFORM lib_fsm.abstract_transition_create(
        v_sleeping_id, 'pause_requested', v_paused_id, 'Operator paused the run'
    );
    PERFORM lib_fsm.abstract_transition_create(
        v_awaiting_id, 'pause_requested', v_paused_id, 'Operator paused the run'
    );
    PERFORM lib_fsm.abstract_transition_create(
        v_running_id, 'pause_requested', v_paused_id, 'Operator paused the run'
    );

    PERFORM lib_fsm.abstract_transition_create(
        v_paused_id, 'resume_to_pending', v_pending_id, 'Operator resumed the run'
    );
    PERFORM lib_fsm.abstract_transition_create(
        v_paused_id, 'resume_to_running', v_running_id, 'Operator resumed the run'
    );
    PERFORM lib_fsm.abstract_transition_create(
        v_paused_id, 'resume_to_retrying', v_retrying_id, 'Operator resumed the run'
    );
    PERFORM lib_fsm.abstract_transition_create(
        v_paused_id, 'resume_to_sleeping', v_sleeping_id, 'Operator resumed the run'
    );
    PERFORM lib_fsm.abstract_transition_create(
        v_paused_id, 'resume_to_awaiting_approval', v_awaiting_id, 'Operator resumed the run'
    );
    PERFORM lib_fsm.abstract_transition_create(
        v_paused_id, 'pause_rejected', v_failed_id, 'A gate decided while paused was a rejection'
    );
    PERFORM lib_fsm.abstract_transition_create(
        v_paused_id, 'cancel_requested', v_cancelled_id, 'User requested cancellation'
    );
END;
$$;

-- State to return to on resume; set only while the run is paused.
ALTER TABLE ironflow.runs ADD COLUMN resume_status TEXT NULL;

CREATE TABLE ironflow.workflow_pauses (
    workflow_name TEXT PRIMARY KEY,
    paused_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    paused_by UUID NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
