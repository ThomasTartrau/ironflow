//! [`GrantBackend`] implementation for PostgreSQL: the shared registry of ironflow-auth-proxy.
//!
//! Rows are keyed by the token id (the SHA-256 of the opaque token), so the
//! token itself is never stored. The credential is AES-256-GCM encrypted with
//! the active version of the store's key ring.

use std::fmt::Display;

use ironflow_core::auth_proxy::{
    AuthProxyError, CredentialKind, Grant, GrantBackend, GrantFuture, ProxyCredential,
};
use sqlx::postgres::PgRow;
use sqlx::{Error as SqlxError, Row, query, query_scalar};

use crate::crypto::{decrypt, encrypt};

use super::PostgresStore;

/// A stored grant, credential still encrypted.
struct GrantRow {
    id: String,
    run_id: String,
    step: String,
    expires_at: i64,
    credential_kind: String,
    encrypted_credential: Vec<u8>,
    nonce: Vec<u8>,
    key_version: i32,
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
        let kind = kind_from_str(&row.credential_kind)?;
        let expires_at = u64::try_from(row.expires_at)
            .map_err(|e| backend_error(format!("expires_at out of range: {e}")))?;
        Ok(Grant {
            run_id: row.run_id,
            step: row.step,
            expires_at,
            credential: ProxyCredential::new(kind, value),
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
            query(
                "INSERT INTO ironflow.auth_proxy_grants (id, run_id, step, expires_at, credential_kind, encrypted_credential, nonce, key_version) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(&grant.id)
            .bind(&grant.run_id)
            .bind(&grant.step)
            .bind(expires_at)
            .bind(kind_to_str(grant.credential.kind()))
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
                "SELECT id, run_id, step, expires_at, credential_kind, encrypted_credential, nonce, key_version FROM ironflow.auth_proxy_grants WHERE id = $1",
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

    fn remove_run(&self, run_id: &str) -> GrantFuture<'_, usize> {
        let run_id = run_id.to_string();
        Box::pin(async move {
            let result = query("DELETE FROM ironflow.auth_proxy_grants WHERE run_id = $1")
                .bind(&run_id)
                .execute(&self.pool)
                .await
                .map_err(backend_error)?;
            to_count(result.rows_affected())
        })
    }

    fn purge_expired(&self, now: u64) -> GrantFuture<'_, usize> {
        Box::pin(async move {
            let now = to_unix_i64(now)?;
            let result = query("DELETE FROM ironflow.auth_proxy_grants WHERE expires_at <= $1")
                .bind(now)
                .execute(&self.pool)
                .await
                .map_err(backend_error)?;
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
}
