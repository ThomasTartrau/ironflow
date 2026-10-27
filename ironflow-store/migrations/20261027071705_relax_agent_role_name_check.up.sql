-- An agent role is named like the `agent_role` of its steps: its type name by
-- default (`Reviewer`), any name that is not blank, has no surrounding
-- whitespace and fits in 128 bytes. Rust trims Unicode whitespace too; the
-- column only guards against spaces.
ALTER TABLE ironflow.agent_roles DROP CONSTRAINT IF EXISTS agent_roles_name_check;
ALTER TABLE ironflow.agent_roles ADD CONSTRAINT agent_roles_name_check
    CHECK (octet_length(name) BETWEEN 1 AND 128 AND btrim(name) = name);
