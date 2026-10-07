-- Catch-up, overlap and timezone policy of each schedule. NOT NULL with a
-- constant DEFAULT: no table rewrite on PostgreSQL 11+.
ALTER TABLE ironflow.schedules
    ADD COLUMN catchup TEXT NOT NULL DEFAULT 'latest'
        CHECK (catchup IN ('latest', 'all', 'skip')),
    ADD COLUMN catchup_max INTEGER NOT NULL DEFAULT 10
        CHECK (catchup_max BETWEEN 1 AND 1000),
    ADD COLUMN catchup_window_secs INTEGER NOT NULL DEFAULT 86400
        CHECK (catchup_window_secs BETWEEN 60 AND 2592000),
    ADD COLUMN overlap TEXT NOT NULL DEFAULT 'allow'
        CHECK (overlap IN ('allow', 'skip')),
    ADD COLUMN timezone TEXT NOT NULL DEFAULT 'UTC';
