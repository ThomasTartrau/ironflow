ALTER TABLE ironflow.schedules
    DROP COLUMN IF EXISTS catchup,
    DROP COLUMN IF EXISTS catchup_max,
    DROP COLUMN IF EXISTS catchup_window_secs,
    DROP COLUMN IF EXISTS overlap,
    DROP COLUMN IF EXISTS timezone;
