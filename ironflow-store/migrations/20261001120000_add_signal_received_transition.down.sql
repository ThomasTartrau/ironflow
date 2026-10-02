-- Remove the `signal_received` transition from the run_lifecycle FSM.
-- History rows recorded through it are relabelled `delay_elapsed`, the other
-- `sleeping -> pending` transition, so the event log keeps naming a
-- transition that still exists.

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

    UPDATE lib_fsm.state_machine_event
    SET event = 'delay_elapsed'
    WHERE event = 'signal_received'
      AND abstract_state__id = v_pending_id;

    DELETE FROM lib_fsm.abstract_transition
    WHERE from_abstract_state__id = v_sleeping_id
      AND event = 'signal_received';
END;
$$;
