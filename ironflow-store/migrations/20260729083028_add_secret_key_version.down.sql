-- Dropping key_version would make secrets encrypted with any other version
-- permanently unreadable: nothing would record which key produced them.
-- Refuse the rollback instead of losing the data silently.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM ironflow.secrets WHERE key_version <> 1) THEN
        RAISE EXCEPTION
            'cannot drop key_version: % secret(s) are encrypted with a non-default key version; rotate back to version 1 first',
            (SELECT count(*) FROM ironflow.secrets WHERE key_version <> 1);
    END IF;
END $$;

DROP INDEX IF EXISTS ironflow.idx_secrets_key_version;

ALTER TABLE ironflow.secrets DROP COLUMN key_version;
