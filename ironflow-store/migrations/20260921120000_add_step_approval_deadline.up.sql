-- SLA deadline + escalation bookkeeping for approval gates.
ALTER TABLE ironflow.steps
    ADD COLUMN approval_deadline_at TIMESTAMPTZ,
    ADD COLUMN approval_stage INT NOT NULL DEFAULT 0,
    ADD COLUMN approval_assignee TEXT;

-- The deadline is only ever set on a step in `awaiting_approval` and is cleared
-- as soon as the gate resolves, so a partial index on the expiry column is
-- enough for the escalator's claim query.
CREATE INDEX idx_steps_approval_deadline_at
    ON ironflow.steps (approval_deadline_at)
    WHERE approval_deadline_at IS NOT NULL;

-- An escalation can complete a gate outright (EscalationPolicy::AutoApprove),
-- and the approval replay marks a granted gate completed. Both go straight from
-- `awaiting_approval` to `completed`, a transition the step FSM never declared.
DO $$
DECLARE
    v_machine_id UUID;
    v_awaiting_id UUID;
    v_completed_id UUID;
BEGIN
    SELECT abstract_machine__id INTO v_machine_id
    FROM lib_fsm.abstract_state_machine
    WHERE name = 'step_lifecycle';

    SELECT abstract_state__id INTO v_awaiting_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'awaiting_approval';

    SELECT abstract_state__id INTO v_completed_id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = v_machine_id AND name = 'completed';

    PERFORM lib_fsm.abstract_transition_create(
        v_awaiting_id, 'approved', v_completed_id, 'Approval gate resolved without re-running the step'
    );
END;
$$;
