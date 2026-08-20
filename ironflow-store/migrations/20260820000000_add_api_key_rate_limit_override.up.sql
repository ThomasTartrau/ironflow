ALTER TABLE iam.api_keys ADD COLUMN rate_limit_override INTEGER CHECK (rate_limit_override IS NULL OR rate_limit_override >= 0);
