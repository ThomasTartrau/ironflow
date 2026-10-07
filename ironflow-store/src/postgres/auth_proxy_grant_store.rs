//! [`GrantBackend`] implementation for PostgreSQL: the shared registry of ironflow-auth-proxy.
//!
//! Rows are keyed by the token id (the SHA-256 of the opaque token), so the
//! token itself is never stored. The credential (a Claude credential or the
//! value of a proxied secret) is AES-256-GCM encrypted with the active version
//! of the store's key ring. The name, injection mode and host allowlist of a
//! secret are not secret: they are stored in clear in `secret_spec`. A revoked
//! grant leaves a tombstone in `auth_proxy_revocations` until its expiry.

use std::fmt::Display;

use ironflow_core::auth_proxy::{
    AuthProxyError, CredentialKind, Grant, GrantBackend, GrantCredential, GrantFuture, HostPattern,
    ProxyCredential, SecretCredential, SecretInjection,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, from_value, to_value};
use sqlx::postgres::PgRow;
use sqlx::{Error as SqlxError, Row, query, query_scalar};

use crate::crypto::{decrypt, encrypt};

use super::PostgresStore;

/// `credential_kind` of a proxied secret grant.
const SECRET_KIND: &str = "secret";

/// A stored grant, credential still encrypted.
struct GrantRow {
    id: String,
    run_id: String,
    step: String,
    expires_at: i64,
    credential_kind: String,
    secret_spec: Option<Value>,
    encrypted_credential: Vec<u8>,
    nonce: Vec<u8>,
    key_version: i32,
}

/// Everything of a [`SecretCredential`] but its value: the `secret_spec`
/// column.
#[derive(Serialize, Deserialize)]
struct SecretSpec {
    name: String,
    injection: SecretInjection,
    hosts: Vec<HostPattern>,
}

/// Wrap a storage, crypto or conversion error. Callers never pass a value
/// holding the credential or the grant.
fn backend_error(e: impl Display) -> AuthProxyError {
    AuthProxyError::Backend(e.to_string())
}

fn kind_to_str(kind: CredentialKind) -> &'static str {
    match kind {
        CredentialKind::OauthToken => "oauth_token",
        CredentialKind::ApiKey => "api_key",
    }
}

fn kind_from_str(kind: &str) -> Result<CredentialKind, AuthProxyError> {
    match kind {
        "oauth_token" => Ok(CredentialKind::OauthToken),
        "api_key" => Ok(CredentialKind::ApiKey),
        other => Err(AuthProxyError::Backend(format!(
            "unknown credential kind {other:?}"
        ))),
    }
}

/// The `credential_kind` and `secret_spec` columns of `credential`.
fn credential_columns(
    credential: &GrantCredential,
) -> Result<(&'static str, Option<Value>), AuthProxyError> {
    match credential {
        GrantCredential::Claude(claude) => Ok((kind_to_str(claude.kind()), None)),
        GrantCredential::Secret(secret) => {
            let spec = SecretSpec {
                name: secret.name().to_string(),
                injection: secret.injection().clone(),
                hosts: secret.hosts().to_vec(),
            };
            Ok((SECRET_KIND, Some(to_value(spec).map_err(backend_error)?)))
        }
    }
}

/// Rebuild a credential from its decrypted `value` and its columns.
fn credential_from_columns(
    kind: &str,
    spec: Option<Value>,
    value: String,
) -> Result<GrantCredential, AuthProxyError> {
    if kind != SECRET_KIND {
        return Ok(ProxyCredential::new(kind_from_str(kind)?, value).into());
    }
    let spec = spec.ok_or_else(|| backend_error("secret grant without secret_spec"))?;
    let spec: SecretSpec = from_value(spec).map_err(backend_error)?;
    Ok(SecretCredential::new(spec.name, value, spec.injection, spec.hosts).into())
}

fn to_unix_i64(secs: u64) -> Result<i64, AuthProxyError> {
    i64::try_from(secs).map_err(|e| backend_error(format!("expires_at out of range: {e}")))
}

impl GrantRow {
    fn from_pg(row: &PgRow) -> Result<Self, SqlxError> {
        Ok(Self {
            id: row.try_get("id")?,
            run_id: row.try_get("run_id")?,
            step: row.try_get("step")?,
            expires_at: row.try_get("expires_at")?,
            credential_kind: row.try_get("credential_kind")?,
            secret_spec: row.try_get("secret_spec")?,
            encrypted_credential: row.try_get("encrypted_credential")?,
            nonce: row.try_get("nonce")?,
            key_version: row.try_get("key_version")?,
        })
    }
}

fn to_count(rows: u64) -> Result<usize, AuthProxyError> {
    usize::try_from(rows).map_err(backend_error)
}

impl PostgresStore {
    /// Decrypt a stored row back into a [`Grant`].
    fn grant_from_row(&self, row: GrantRow) -> Result<Grant, AuthProxyError> {
        let ring = self.require_key_ring().map_err(backend_error)?;
        let key = ring.key_for(row.key_version).ok_or_else(|| {
            AuthProxyError::Backend(format!(
                "auth proxy grant uses key version {} which is not configured",
                row.key_version
            ))
        })?;
        let plaintext =
            decrypt(key, &row.encrypted_credential, &row.nonce).map_err(backend_error)?;
        let value = String::from_utf8(plaintext)
            .map_err(|e| backend_error(format!("invalid UTF-8: {}", e.utf8_error())))?;
        let credential = credential_from_columns(&row.credential_kind, row.secret_spec, value)?;
        let expires_at = u64::try_from(row.expires_at)
            .map_err(|e| backend_error(format!("expires_at out of range: {e}")))?;
        Ok(Grant {
            run_id: row.run_id,
            step: row.step,
            expires_at,
            credential,
            id: row.id,
        })
    }
}

impl GrantBackend for PostgresStore {
    fn insert(&self, grant: Grant) -> GrantFuture<'_, ()> {
        Box::pin(async move {
            let ring = self.require_key_ring().map_err(backend_error)?;
            let credential = grant.credential.expose().as_bytes();
            let (encrypted_credential, nonce) =
                encrypt(ring.active_key(), credential).map_err(backend_error)?;
            let expires_at = to_unix_i64(grant.expires_at)?;
            let (kind, secret_spec) = credential_columns(&grant.credential)?;
            query(
                "INSERT INTO ironflow.auth_proxy_grants (id, run_id, step, expires_at, credential_kind, secret_spec, encrypted_credential, nonce, key_version) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(&grant.id)
            .bind(&grant.run_id)
            .bind(&grant.step)
            .bind(expires_at)
            .bind(kind)
            .bind(secret_spec)
            .bind(&encrypted_credential)
            .bind(&nonce)
            .bind(ring.active_version())
            .execute(&self.pool)
            .await
            .map_err(backend_error)?;
            Ok(())
        })
    }

    fn get(&self, id: &str) -> GrantFuture<'_, Option<Grant>> {
        let id = id.to_string();
        Box::pin(async move {
            let row = query(
                "SELECT id, run_id, step, expires_at, credential_kind, secret_spec, encrypted_credential, nonce, key_version FROM ironflow.auth_proxy_grants WHERE id = $1",
            )
            .bind(&id)
            .fetch_optional(&self.pool)
            .await
            .map_err(backend_error)?
            .map(|row| GrantRow::from_pg(&row))
            .transpose()
            .map_err(backend_error)?;
            row.map(|row| self.grant_from_row(row)).transpose()
        })
    }

    fn remove(&self, id: &str) -> GrantFuture<'_, bool> {
        let id = id.to_string();
        Box::pin(async move {
            let result = query("DELETE FROM ironflow.auth_proxy_grants WHERE id = $1")
                .bind(&id)
                .execute(&self.pool)
                .await
                .map_err(backend_error)?;
            Ok(result.rows_affected() > 0)
        })
    }

    fn revoke(&self, id: &str) -> GrantFuture<'_, bool> {
        let id = id.to_string();
        Box::pin(async move {
            let mut tx = self.pool.begin().await.map_err(backend_error)?;
            let removed = query(
                "DELETE FROM ironflow.auth_proxy_grants WHERE id = $1 RETURNING run_id, expires_at",
            )
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(backend_error)?;
            let Some(row) = removed else {
                tx.commit().await.map_err(backend_error)?;
                return Ok(false);
            };
            let run_id: String = row.try_get("run_id").map_err(backend_error)?;
            let expires_at: i64 = row.try_get("expires_at").map_err(backend_error)?;
            query(
                "INSERT INTO ironflow.auth_proxy_revocations (id, run_id, expires_at) VALUES ($1, $2, $3) ON CONFLICT (id) DO NOTHING",
            )
            .bind(&id)
            .bind(&run_id)
            .bind(expires_at)
            .execute(&mut *tx)
            .await
            .map_err(backend_error)?;
            tx.commit().await.map_err(backend_error)?;
            Ok(true)
        })
    }

    fn revoke_run(&self, run_id: &str) -> GrantFuture<'_, usize> {
        let run_id = run_id.to_string();
        Box::pin(async move {
            let mut tx = self.pool.begin().await.map_err(backend_error)?;
            let rows = query(
                "DELETE FROM ironflow.auth_proxy_grants WHERE run_id = $1 RETURNING id, expires_at",
            )
            .bind(&run_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(backend_error)?;
            let ids = rows
                .iter()
                .map(|row| row.try_get::<String, _>("id"))
                .collect::<Result<Vec<_>, _>>()
                .map_err(backend_error)?;
            let expiries = rows
                .iter()
                .map(|row| row.try_get::<i64, _>("expires_at"))
                .collect::<Result<Vec<_>, _>>()
                .map_err(backend_error)?;
            query(
                "INSERT INTO ironflow.auth_proxy_revocations (id, run_id, expires_at) SELECT id, $2, expires_at FROM UNNEST($1::TEXT[], $3::BIGINT[]) AS revoked (id, expires_at) ON CONFLICT (id) DO NOTHING",
            )
            .bind(&ids)
            .bind(&run_id)
            .bind(&expiries)
            .execute(&mut *tx)
            .await
            .map_err(backend_error)?;
            tx.commit().await.map_err(backend_error)?;
            Ok(ids.len())
        })
    }

    fn is_revoked(&self, id: &str) -> GrantFuture<'_, bool> {
        let id = id.to_string();
        Box::pin(async move {
            query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM ironflow.auth_proxy_revocations WHERE id = $1)",
            )
            .bind(&id)
            .fetch_one(&self.pool)
            .await
            .map_err(backend_error)
        })
    }

    fn purge_expired(&self, now: u64) -> GrantFuture<'_, usize> {
        Box::pin(async move {
            let now = to_unix_i64(now)?;
            let mut tx = self.pool.begin().await.map_err(backend_error)?;
            let result = query("DELETE FROM ironflow.auth_proxy_grants WHERE expires_at <= $1")
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(backend_error)?;
            query("DELETE FROM ironflow.auth_proxy_revocations WHERE expires_at <= $1")
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(backend_error)?;
            tx.commit().await.map_err(backend_error)?;
            to_count(result.rows_affected())
        })
    }

    fn len(&self) -> GrantFuture<'_, usize> {
        Box::pin(async move {
            let count = query_scalar::<_, i64>("SELECT COUNT(*) FROM ironflow.auth_proxy_grants")
                .fetch_one(&self.pool)
                .await
                .map_err(backend_error)?;
            usize::try_from(count).map_err(backend_error)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_kind_round_trips() {
        for kind in [CredentialKind::OauthToken, CredentialKind::ApiKey] {
            assert_eq!(kind_from_str(kind_to_str(kind)).unwrap(), kind);
        }
    }

    #[test]
    fn unknown_credential_kind_is_an_error() {
        match kind_from_str("bearer") {
            Err(AuthProxyError::Backend(message)) => assert!(message.contains("bearer")),
            other => panic!("expected Backend, got {other:?}"),
        }
    }

    #[test]
    fn expires_at_beyond_i64_is_an_error() {
        assert_eq!(to_unix_i64(1_700_000_000).unwrap(), 1_700_000_000);
        assert!(matches!(
            to_unix_i64(u64::MAX),
            Err(AuthProxyError::Backend(_))
        ));
    }
    #[test]
    fn secret_credential_round_trips_through_columns() {
        let secret = SecretCredential::new(
            "GITLAB_TOKEN".to_string(),
            "glpat-x".to_string(),
            SecretInjection::Basic {
                username: "oauth2".to_string(),
            },
            vec![HostPattern::parse("*.gitlab.com").unwrap()],
        );
        let (kind, spec) = credential_columns(&secret.into()).unwrap();
        assert_eq!(kind, SECRET_KIND);
        let spec = spec.unwrap();
        assert!(!spec.to_string().contains("glpat-x"), "{spec}");
        match credential_from_columns(kind, Some(spec), "glpat-x".to_string()).unwrap() {
            GrantCredential::Secret(secret) => {
                assert_eq!(secret.name(), "GITLAB_TOKEN");
                assert_eq!(secret.expose(), "glpat-x");
                assert_eq!(secret.hosts()[0].as_str(), "*.gitlab.com");
                assert_eq!(
                    secret.injection(),
                    &SecretInjection::Basic {
                        username: "oauth2".to_string()
                    }
                );
            }
            other => panic!("expected a secret, got {other:?}"),
        }
    }

    #[test]
    fn claude_credential_has_no_secret_spec() {
        let claude = ProxyCredential::new(CredentialKind::ApiKey, "sk-ant-api03-x".to_string());
        let (kind, spec) = credential_columns(&claude.into()).unwrap();
        assert_eq!(kind, "api_key");
        assert!(spec.is_none());
        match credential_from_columns(kind, None, "sk-ant-api03-x".to_string()).unwrap() {
            GrantCredential::Claude(claude) => assert_eq!(claude.kind(), CredentialKind::ApiKey),
            other => panic!("expected a Claude credential, got {other:?}"),
        }
    }

    #[test]
    fn secret_row_without_spec_is_an_error() {
        match credential_from_columns(SECRET_KIND, None, "x".to_string()) {
            Err(AuthProxyError::Backend(message)) => assert!(message.contains("secret_spec")),
            other => panic!("expected Backend, got {other:?}"),
        }
    }
}
