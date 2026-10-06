-- Claude Code session an agent step ran in, so a step interrupted by a lost
-- worker lease can resume the session instead of restarting from scratch.
ALTER TABLE ironflow.steps ADD COLUMN session_id TEXT;
