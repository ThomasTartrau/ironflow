-- Worker tags a worker must carry to pick the run up. Empty means any worker.
ALTER TABLE ironflow.runs ADD COLUMN worker_tags TEXT[] NOT NULL DEFAULT '{}';
