DROP TABLE IF EXISTS ironflow.auth_proxy_revocations;
-- Secret grants cannot be represented without secret_spec: drop them. They
-- live minutes and the worker issues new ones for the next pod.
DELETE FROM ironflow.auth_proxy_grants WHERE credential_kind = 'secret';
ALTER TABLE ironflow.auth_proxy_grants DROP CONSTRAINT IF EXISTS auth_proxy_grants_secret_spec_check;
ALTER TABLE ironflow.auth_proxy_grants DROP COLUMN IF EXISTS secret_spec;
ALTER TABLE ironflow.auth_proxy_grants DROP CONSTRAINT IF EXISTS auth_proxy_grants_credential_kind_check;
ALTER TABLE ironflow.auth_proxy_grants ADD CONSTRAINT auth_proxy_grants_credential_kind_check CHECK (credential_kind IN ('oauth_token', 'api_key'));
