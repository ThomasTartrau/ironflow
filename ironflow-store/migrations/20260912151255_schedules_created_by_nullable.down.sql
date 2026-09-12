-- Reverting requires every row to have an author. Rows with a NULL
-- created_by_user_id (handler schedules) must be removed or backfilled before
-- this runs, otherwise the SET NOT NULL fails. This is a destructive reversion.
ALTER TABLE ironflow.schedules
    ALTER COLUMN created_by_user_id SET NOT NULL;
