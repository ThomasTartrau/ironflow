-- Remove run_lifecycle FSM definition
DELETE FROM lib_fsm.abstract_state_machine WHERE name = 'run_lifecycle';
