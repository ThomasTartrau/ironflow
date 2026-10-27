-- Counters of roles whose name the old check rejects are dropped (with their
-- reservations, by cascade): the next step of such a role recreates it under
-- a name it accepts, or fails.
DELETE FROM ironflow.agent_roles WHERE name !~ '^[a-z0-9][a-z0-9._-]{0,63}$';
ALTER TABLE ironflow.agent_roles DROP CONSTRAINT IF EXISTS agent_roles_name_check;
ALTER TABLE ironflow.agent_roles ADD CONSTRAINT agent_roles_name_check
    CHECK (name ~ '^[a-z0-9][a-z0-9._-]{0,63}$');
