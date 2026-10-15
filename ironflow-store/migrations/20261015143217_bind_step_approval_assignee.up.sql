-- The user an approval gate's `user:` assignee resolved to when the gate opened.
-- Authorization compares the caller to this id, never to the username, so an
-- account created later under the assignee's name cannot resolve the gate.
--
-- No FOREIGN KEY: user ids are UUIDv7 and never reused, so a deleted user's id
-- matches nobody (same choice as ironflow.runs.created_by_user_id).
ALTER TABLE ironflow.steps
    ADD COLUMN IF NOT EXISTS approval_assignee_user_id UUID;

-- Gates already open keep working: bind them to the account holding the name now.
UPDATE ironflow.steps s
SET approval_assignee_user_id = u.id
FROM iam.users u
WHERE s.approval_assignee = 'user:' || u.username;
