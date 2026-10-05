-- Concurrency groups of a run, as a JSON array of {"group": text, "limit": int}.
-- Gating happens in pick_next_pending under per-group advisory locks; the GIN
-- index serves the containment lookups used to count running runs per group.
ALTER TABLE ironflow.runs ADD COLUMN concurrency_limits JSONB NOT NULL DEFAULT '[]'::jsonb;
CREATE INDEX idx_runs_concurrency_limits ON ironflow.runs USING GIN (concurrency_limits jsonb_path_ops) WHERE concurrency_limits <> '[]'::jsonb;
