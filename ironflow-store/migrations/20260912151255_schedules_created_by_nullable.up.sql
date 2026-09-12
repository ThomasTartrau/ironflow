-- Handler-declared schedules have no human author, so created_by_user_id must
-- accept NULL. The FK to iam.users(id) stays; NULL satisfies a foreign key.
ALTER TABLE ironflow.schedules
    ALTER COLUMN created_by_user_id DROP NOT NULL;
