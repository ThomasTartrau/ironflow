-- Typed output set by the workflow handler with set_output, written when an
-- execution ends. Nullable with no default: no table rewrite.
ALTER TABLE ironflow.runs ADD COLUMN output JSONB;
