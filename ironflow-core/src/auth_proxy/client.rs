//! Admin client the worker uses to issue and revoke opaque tokens.

use std::fmt;
use std::time::Duration;

use reqwest::{Client, Error as ReqwestError, Response, StatusCode};
use serde::Deserialize;
use url::Url;

use super::AuthProxyError;
use super::registry::{IssuedToken, TokenRequest};

/// Timeout of one admin call.
const ADMIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Longest response body kept in [`AuthProxyError::Admin`].
const MAX_ERROR_CHARS: usize = 200;

#[derive(Deserialize)]
struct RevokedCount {
    revoked: usize,
}

/// Client of the `ironflow-auth-proxy` admin API. Its [`Debug`] output never
/// shows the admin key.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::auth_proxy::{
///     AuthProxyClient, CredentialKind, ProxyCredential, TokenRequest,
/// };
///
/// # async fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
/// let client = AuthProxyClient::new("http://ironflow-auth-proxy.ironflow-system", "admin-key");
/// let issued = client
///     .issue(&TokenRequest {
///         run_id: "run-1".to_string(),
///         step: "review".to_string(),
///         expires_at: 1_700_000_600,
///         credential: ProxyCredential::new(CredentialKind::OauthToken, "sk-ant-oat01-x".to_string()).into(),
///     })
///     .await?;
/// client.revoke(&issued.id).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct AuthProxyClient {
    base: String,
    admin_key: String,
    http: Client,
}

impl fmt::Debug for AuthProxyClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthProxyClient")
            .field("base", &self.base)
            .field("admin_key", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl AuthProxyClient {
    /// Build a client for the proxy at `base_url`, authenticating with
    /// `admin_key` (surrounding whitespace, such as the newline of a key read
    /// from a file, is dropped).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::auth_proxy::AuthProxyClient;
    ///
    /// let client = AuthProxyClient::new("http://proxy/", "admin-key");
    /// assert!(!format!("{client:?}").contains("admin-key"));
    /// ```
    pub fn new(base_url: &str, admin_key: &str) -> Self {
        Self {
            base: base_url.trim_end_matches('/').to_string(),
            admin_key: admin_key.trim().to_string(),
            http: Client::new(),
        }
    }

    /// Admin URL made of the base and the given path segments, each
    /// percent-encoded.
    fn url(&self, segments: &[&str]) -> Result<Url, AuthProxyError> {
        let mut url = Url::parse(&self.base)
            .map_err(|e| AuthProxyError::InvalidRequest(format!("invalid auth proxy URL: {e}")))?;
        url.path_segments_mut()
            .map_err(|()| {
                AuthProxyError::InvalidRequest("auth proxy URL cannot carry a path".to_string())
            })?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    /// Issue an opaque token for `req` (`POST /admin/v1/tokens`).
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Admin`] when the proxy answers anything but
    /// `201 Created` (the message is the response body, truncated, never the
    /// request), [`AuthProxyError::Transport`] when it cannot be reached.
    ///
    /// # Examples
    ///
    /// See [`AuthProxyClient`].
    pub async fn issue(&self, req: &TokenRequest) -> Result<IssuedToken, AuthProxyError> {
        let url = self.url(&["admin", "v1", "tokens"])?;
        let resp = self
            .http
            .post(url)
            .bearer_auth(&self.admin_key)
            .timeout(ADMIN_TIMEOUT)
            .json(req)
            .send()
            .await
            .map_err(transport)?;
        if resp.status() != StatusCode::CREATED {
            return Err(admin_error(resp).await);
        }
        resp.json::<IssuedToken>().await.map_err(transport)
    }

    /// Revoke a token by id (`DELETE /admin/v1/tokens/{id}`). An unknown id
    /// (already revoked or expired) is not an error.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Admin`] on a status other than 204 or 404,
    /// [`AuthProxyError::Transport`] when the proxy cannot be reached.
    ///
    /// # Examples
    ///
    /// See [`AuthProxyClient`].
    pub async fn revoke(&self, id: &str) -> Result<(), AuthProxyError> {
        let url = self.url(&["admin", "v1", "tokens", id])?;
        let resp = self
            .http
            .delete(url)
            .bearer_auth(&self.admin_key)
            .timeout(ADMIN_TIMEOUT)
            .send()
            .await
            .map_err(transport)?;
        match resp.status() {
            StatusCode::NO_CONTENT | StatusCode::NOT_FOUND => Ok(()),
            _ => Err(admin_error(resp).await),
        }
    }

    /// Revoke every token of a run (`DELETE /admin/v1/runs/{run_id}/tokens`).
    /// Returns how many were revoked.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Admin`] on a status other than 200,
    /// [`AuthProxyError::Transport`] when the proxy cannot be reached or the
    /// answer cannot be read.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::auth_proxy::AuthProxyClient;
    ///
    /// # async fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
    /// let client = AuthProxyClient::new("http://ironflow-auth-proxy.ironflow-system", "admin-key");
    /// let revoked = client.revoke_run("run-1").await?;
    /// println!("{revoked} tokens revoked");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn revoke_run(&self, run_id: &str) -> Result<usize, AuthProxyError> {
        let url = self.url(&["admin", "v1", "runs", run_id, "tokens"])?;
        let resp = self
            .http
            .delete(url)
            .bearer_auth(&self.admin_key)
            .timeout(ADMIN_TIMEOUT)
            .send()
            .await
            .map_err(transport)?;
        if resp.status() != StatusCode::OK {
            return Err(admin_error(resp).await);
        }
        let count = resp.json::<RevokedCount>().await.map_err(transport)?;
        Ok(count.revoked)
    }
}

/// A transport error, stripped of the URL.
fn transport(e: ReqwestError) -> AuthProxyError {
    AuthProxyError::Transport(e.without_url().to_string())
}

/// An [`AuthProxyError::Admin`] from an unexpected answer.
async fn admin_error(resp: Response) -> AuthProxyError {
    let status = resp.status().as_u16();
    let message = match resp.text().await {
        Ok(body) => body.chars().take(MAX_ERROR_CHARS).collect(),
        Err(e) => format!("unreadable response body: {}", e.without_url()),
    };
    AuthProxyError::Admin { status, message }
}
