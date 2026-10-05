-- Persistent environment (PVC name) an agent step ran in, so a later step can
-- resume in the same workspace.
ALTER TABLE ironflow.steps ADD COLUMN environment_id TEXT;
