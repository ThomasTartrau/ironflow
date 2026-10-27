-- Agent steps grouped by role: an agent step built from an `AgentRole` carries
-- the role name under `role` in its persisted input. Serves the steps listed by
-- role (newest first) and the per-role cost statistics over a period.
CREATE INDEX IF NOT EXISTS idx_steps_agent_role ON ironflow.steps ((input->>'role'), created_at) WHERE kind = 'agent';
