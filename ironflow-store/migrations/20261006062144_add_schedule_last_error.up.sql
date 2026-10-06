-- Why Ironflow disabled a schedule on its own (e.g. a cron expression whose
-- next occurrence cannot be computed). Nullable with no default: no table rewrite.
ALTER TABLE ironflow.schedules ADD COLUMN last_error TEXT;
