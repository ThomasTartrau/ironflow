-- Prompt-cache token accounting for agent steps. `input_tokens` holds uncached input only.
ALTER TABLE ironflow.steps
    ADD COLUMN cache_read_input_tokens BIGINT,
    ADD COLUMN cache_creation_input_tokens BIGINT;
