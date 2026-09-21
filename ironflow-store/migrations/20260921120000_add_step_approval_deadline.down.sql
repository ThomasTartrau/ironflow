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

    DELETE FROM lib_fsm.abstract_transition
    WHERE from_abstract_state__id = v_awaiting_id
      AND to_abstract_state__id = v_completed_id;
END;
$$;

DROP INDEX ironflow.idx_steps_approval_deadline_at;

ALTER TABLE ironflow.steps
    DROP COLUMN approval_assignee,
    DROP COLUMN approval_stage,
    DROP COLUMN approval_deadline_at;
