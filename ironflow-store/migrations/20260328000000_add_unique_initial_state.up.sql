-- Enforce a single initial state per abstract state machine.
-- Without this, state_machine_create's SELECT ... WHERE is_initial = TRUE LIMIT 1
-- is non-deterministic if multiple initial states exist.
CREATE UNIQUE INDEX uq_abstract_state_initial
    ON lib_fsm.abstract_state (abstract_machine__id)
    WHERE is_initial = TRUE;
