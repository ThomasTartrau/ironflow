-- Remove step_lifecycle FSM definition
DELETE FROM lib_fsm.abstract_state_machine WHERE name = 'step_lifecycle';
