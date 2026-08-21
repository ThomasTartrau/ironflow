//! Ergonomic HTTP client for the Ironflow API.
//!
//! Wraps [`reqwest::Client`] with Bearer auth and automatic
//! deserialization of the `{ data, meta }` response envelope.

use std::time::Duration;

use chrono::{DateTime, Utc};
pub use ironflow_types::{ApiMeta, ApiResponse};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::{Client, RequestBuilder};
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::error::{ApiErrorEnvelope, Error};
use crate::types;

/// Configuration for the Ironflow SDK client.
///
/// # Examples
///
/// ```
/// use ironflow_sdk::client::ClientConfig;
/// use std::time::Duration;
///
/// let config = ClientConfig {
///     base_url: "https://ironflow.example.com".to_string(),
///     api_key: "my-api-key".to_string(),
///     timeout: Duration::from_secs(60),
/// };
/// ```
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Base URL of the Ironflow API (e.g. `https://ironflow.example.com`).
    pub base_url: String,
    /// API key for Bearer authentication.
    pub api_key: String,
    /// HTTP request timeout.
    pub timeout: Duration,
}

/// Filtering options for [`IronflowClient::list_runs_filtered`].
///
/// # Examples
///
/// ```
/// use ironflow_sdk::client::ListRunsFilter;
///
/// let filter = ListRunsFilter {
///     workflow: Some("deploy"),
///     status: Some("completed"),
///     page: Some(1),
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ListRunsFilter<'a> {
    /// Filter by workflow name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow: Option<&'a str>,
    /// Filter by run status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<&'a str>,
    /// Page number (1-based).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    /// Items per page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_page: Option<u32>,
    /// Filter runs that have/don't have steps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_steps: Option<bool>,
    /// Filter by label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<&'a str>,
    /// Filter by author: the user ID that triggered the run.
    ///
    /// Also matches runs triggered by one of that user's API keys.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<Uuid>,
}

/// Filtering options for [`IronflowClient::get_run_logs`].
///
/// # Examples
///
/// ```
/// use ironflow_sdk::client::ListRunLogsFilter;
/// use uuid::Uuid;
///
/// let filter = ListRunLogsFilter {
///     step_id: Some(Uuid::nil()),
///     stream: Some("stderr"),
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ListRunLogsFilter<'a> {
    /// Filter by step ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_id: Option<Uuid>,
    /// Filter by output stream (`stdout`, `stderr`, `system`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<&'a str>,
    /// Cursor for pagination (last entry ID from previous page).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<Uuid>,
    /// Number of entries to return (default: 100, max: 1000).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Filtering options for [`IronflowClient::list_audit_logs_filtered`].
///
/// # Examples
///
/// ```
/// use ironflow_sdk::client::ListAuditLogsFilter;
///
/// let filter = ListAuditLogsFilter {
///     event_type: Some("run_created"),
///     per_page: Some(25),
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ListAuditLogsFilter<'a> {
    /// Filter by event type (e.g. `run_status_changed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<&'a str>,
    /// Filter by run ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<Uuid>,
    /// Keep entries recorded at or after this instant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<DateTime<Utc>>,
    /// Keep entries recorded at or before this instant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<DateTime<Utc>>,
    /// Page number (1-based).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    /// Items per page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_page: Option<u32>,
}

/// Type-safe client for the Ironflow REST API.
///
/// Handles Bearer authentication, the `{ data, meta }` response envelope,
/// and provides methods for all public API endpoints.
///
/// # Examples
///
/// ```no_run
/// use ironflow_sdk::IronflowClient;
///
/// # async fn example() -> Result<(), ironflow_sdk::Error> {
/// let client = IronflowClient::new("https://ironflow.example.com", "my-api-key");
///
/// // List runs
/// let runs = client.list_runs().await?;
/// for run in &runs.data {
///     # let _ = run;
/// }
///
/// // Get a specific workflow
/// let wf = client.get_workflow("deploy").await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct IronflowClient {
    pub(crate) http: Client,
    base_url: String,
}

impl IronflowClient {
    /// Create a new client with the given base URL and API key.
    ///
    /// Uses a default timeout of 30 seconds.
    ///
    /// # Panics
    ///
    /// Panics if `api_key` contains characters invalid for HTTP headers
    /// (e.g. newlines, NUL bytes, or non-visible ASCII).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_sdk::IronflowClient;
    ///
    /// let client = IronflowClient::new("https://ironflow.example.com", "my-api-key");
    /// ```
    pub fn new(base_url: &str, api_key: &str) -> Self {
        let config = ClientConfig {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            timeout: Duration::from_secs(30),
        };
        Self::from_config(config)
    }

    /// Create a new client from a [`ClientConfig`].
    ///
    /// # Panics
    ///
    /// Panics if `config.api_key` contains characters invalid for HTTP headers
    /// (e.g. newlines, NUL bytes, or non-visible ASCII).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_sdk::IronflowClient;
    /// use ironflow_sdk::client::ClientConfig;
    /// use std::time::Duration;
    ///
    /// let config = ClientConfig {
    ///     base_url: "https://ironflow.example.com".to_string(),
    ///     api_key: "my-api-key".to_string(),
    ///     timeout: Duration::from_secs(60),
    /// };
    /// let client = IronflowClient::from_config(config);
    /// ```
    pub fn from_config(config: ClientConfig) -> Self {
        let mut headers = HeaderMap::new();
        let auth_value = format!("Bearer {}", config.api_key);
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&auth_value).expect("invalid API key characters"),
        );

        let http = Client::builder()
            .timeout(config.timeout)
            .default_headers(headers)
            .build()
            .expect("failed to build HTTP client");

        Self {
            http,
            base_url: config.base_url,
        }
    }

    /// Return the base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    pub(crate) fn get(&self, path: &str) -> RequestBuilder {
        self.http.get(self.url(path))
    }

    fn post(&self, path: &str) -> RequestBuilder {
        self.http.post(self.url(path))
    }

    fn put(&self, path: &str) -> RequestBuilder {
        self.http.put(self.url(path))
    }

    fn delete(&self, path: &str) -> RequestBuilder {
        self.http.delete(self.url(path))
    }

    fn patch(&self, path: &str) -> RequestBuilder {
        self.http.patch(self.url(path))
    }

    /// Parse an error response into an [`Error`].
    pub(crate) async fn into_api_error(response: reqwest::Response) -> Error {
        let status = response.status();
        let status_code = status.as_u16();
        let text = response.text().await.unwrap_or_default();
        serde_json::from_str::<ApiErrorEnvelope>(&text)
            .map(|env| Error::api(status_code, env.error.code, env.error.message))
            .unwrap_or_else(|_| {
                Error::api(
                    status_code,
                    status.canonical_reason().unwrap_or("UNKNOWN"),
                    text,
                )
            })
    }

    /// Send a request and deserialize the response envelope.
    async fn send_envelope<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<ApiResponse<T>, Error> {
        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(Self::into_api_error(response).await);
        }

        let bytes = response.bytes().await?;
        serde_json::from_slice::<ApiResponse<T>>(&bytes)
            .map_err(|e| Error::Deserialize(format!("{e}: {}", String::from_utf8_lossy(&bytes))))
    }

    /// Send a request that returns 204 No Content.
    async fn send_no_content(&self, request: RequestBuilder) -> Result<(), Error> {
        let response = request.send().await?;

        if response.status().is_success() {
            return Ok(());
        }

        Err(Self::into_api_error(response).await)
    }

    // ── Health ──────────────────────────────────────────────────────

    /// `GET /api/v1/health-check`
    ///
    /// # Errors
    ///
    /// Returns [`Error::Http`] on network failure.
    pub async fn health_check(&self) -> Result<(), Error> {
        self.send_no_content(self.get("/api/v1/health-check")).await
    }

    // ── Runs ───────────────────────────────────────────────────────

    /// `GET /api/v1/runs` -- List runs with optional filtering and pagination.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 401 Unauthorized.
    pub async fn list_runs(&self) -> Result<ApiResponse<Vec<types::RunResponse>>, Error> {
        self.list_runs_filtered(&ListRunsFilter::default()).await
    }

    /// `GET /api/v1/runs` with query parameters.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 401 Unauthorized.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use ironflow_sdk::client::ListRunsFilter;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let filter = ListRunsFilter {
    ///     workflow: Some("deploy"),
    ///     status: Some("completed"),
    ///     ..Default::default()
    /// };
    /// let runs = client.list_runs_filtered(&filter).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_runs_filtered(
        &self,
        filter: &ListRunsFilter<'_>,
    ) -> Result<ApiResponse<Vec<types::RunResponse>>, Error> {
        self.send_envelope(self.get("/api/v1/runs").query(filter))
            .await
    }

    /// `POST /api/v1/runs` -- Trigger a workflow.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 400 (unknown workflow) or 401/403.
    pub async fn create_run(
        &self,
        request: &types::CreateRunRequest,
    ) -> Result<ApiResponse<types::RunResponse>, Error> {
        self.send_envelope(self.post("/api/v1/runs").json(request))
            .await
    }

    /// `POST /api/v1/runs` with an `Idempotency-Key` header -- trigger a workflow
    /// exactly once.
    ///
    /// Replaying the same key returns the run it already produced instead of
    /// creating a second one. The key stays bound for 24 hours. Use a value the
    /// caller can reproduce across retries, such as a webhook delivery id.
    ///
    /// The returned envelope is identical whether the run was created or replayed;
    /// compare `data.idempotency_key` or the run id if the distinction matters.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 400 (unknown workflow or malformed key), 401/403,
    /// or 409 (`IDEMPOTENCY_KEY_CONFLICT`) when the key is already bound to a
    /// different workflow or payload.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::{IronflowClient, types};
    ///
    /// # async fn example(request: &types::CreateRunRequest) -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    ///
    /// // Derive the key from something the caller can reproduce on retry,
    /// // such as the provider delivery id of the webhook that triggered this.
    /// let run = client
    ///     .create_run_idempotent(request, "github:8f4e2a10")
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_run_idempotent(
        &self,
        request: &types::CreateRunRequest,
        idempotency_key: &str,
    ) -> Result<ApiResponse<types::RunResponse>, Error> {
        self.send_envelope(
            self.post("/api/v1/runs")
                .header("Idempotency-Key", idempotency_key)
                .json(request),
        )
        .await
    }

    /// `GET /api/v1/runs/:id` -- Get run details.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 404 (run not found) or 401.
    pub async fn get_run(&self, id: Uuid) -> Result<ApiResponse<types::RunDetailResponse>, Error> {
        self.send_envelope(self.get(&format!("/api/v1/runs/{id}")))
            .await
    }

    /// Execute a lifecycle action on a run.
    async fn run_action(
        &self,
        id: Uuid,
        action: &str,
    ) -> Result<ApiResponse<types::RunResponse>, Error> {
        self.send_envelope(self.post(&format!("/api/v1/runs/{id}/{action}")))
            .await
    }

    /// `POST /api/v1/runs/:id/cancel` -- Cancel a run.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 404 or 400 (invalid state transition).
    pub async fn cancel_run(&self, id: Uuid) -> Result<ApiResponse<types::RunResponse>, Error> {
        self.run_action(id, "cancel").await
    }

    /// `POST /api/v1/runs/:id/approve` -- Approve a run waiting for approval.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 404 or 400.
    pub async fn approve_run(&self, id: Uuid) -> Result<ApiResponse<types::RunResponse>, Error> {
        self.run_action(id, "approve").await
    }

    /// `POST /api/v1/runs/:id/reject` -- Reject a run waiting for approval.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 404 or 400.
    pub async fn reject_run(&self, id: Uuid) -> Result<ApiResponse<types::RunResponse>, Error> {
        self.run_action(id, "reject").await
    }

    /// `POST /api/v1/runs/:id/retry` -- Retry a failed run (creates a new run).
    ///
    /// When `force` is `true`, the query parameter `?force=true` is appended
    /// to override a handler version mismatch.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 404, 400, or 409 (version mismatch).
    pub async fn retry_run(
        &self,
        id: Uuid,
        force: bool,
    ) -> Result<ApiResponse<types::RunResponse>, Error> {
        let action = if force { "retry?force=true" } else { "retry" };
        self.run_action(id, action).await
    }

    // ── Workflows ──────────────────────────────────────────────────

    /// `GET /api/v1/workflows` -- List registered workflows.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 401.
    pub async fn list_workflows(&self) -> Result<ApiResponse<Vec<types::WorkflowSummary>>, Error> {
        self.send_envelope(self.get("/api/v1/workflows")).await
    }

    /// `GET /api/v1/workflows/:name` -- Get workflow details.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 404 or 401.
    pub async fn get_workflow(
        &self,
        name: &str,
    ) -> Result<ApiResponse<types::WorkflowDetailResponse>, Error> {
        self.send_envelope(self.get(&format!("/api/v1/workflows/{name}")))
            .await
    }

    // ── Stats ──────────────────────────────────────────────────────

    /// `GET /api/v1/stats` -- Aggregate statistics.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 401.
    pub async fn get_stats(&self) -> Result<ApiResponse<types::StatsResponse>, Error> {
        self.send_envelope(self.get("/api/v1/stats")).await
    }

    // ── Auth ───────────────────────────────────────────────────────

    /// `POST /api/v1/auth/sign-in` -- Sign in with credentials.
    ///
    /// Returns cookies/headers for session management.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 401 (invalid credentials).
    pub async fn sign_in(&self, request: &types::SignInRequest) -> Result<(), Error> {
        self.send_no_content(self.post("/api/v1/auth/sign-in").json(request))
            .await
    }

    /// `POST /api/v1/auth/sign-out` -- Sign out.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 401.
    pub async fn sign_out(&self) -> Result<(), Error> {
        self.send_no_content(self.post("/api/v1/auth/sign-out"))
            .await
    }

    /// `GET /api/v1/auth/me` -- Get current user info.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 401.
    pub async fn me(&self) -> Result<ApiResponse<types::MeResponse>, Error> {
        self.send_envelope(self.get("/api/v1/auth/me")).await
    }

    // ── API Keys ───────────────────────────────────────────────────

    /// `GET /api/v1/api-keys` -- List API keys.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 401.
    pub async fn list_api_keys(&self) -> Result<ApiResponse<Vec<types::ApiKeyResponse>>, Error> {
        self.send_envelope(self.get("/api/v1/api-keys")).await
    }

    /// `POST /api/v1/api-keys` -- Create a new API key.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 400 or 401.
    pub async fn create_api_key(
        &self,
        request: &types::CreateApiKeyRequest,
    ) -> Result<ApiResponse<types::CreateApiKeyResponse>, Error> {
        self.send_envelope(self.post("/api/v1/api-keys").json(request))
            .await
    }

    /// `GET /api/v1/api-keys/scopes` -- List available API key scopes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 401.
    pub async fn available_scopes(&self) -> Result<ApiResponse<Vec<types::ScopeEntry>>, Error> {
        self.send_envelope(self.get("/api/v1/api-keys/scopes"))
            .await
    }

    /// `DELETE /api/v1/api-keys/:id` -- Delete an API key.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 404 or 401.
    pub async fn delete_api_key(&self, id: Uuid) -> Result<(), Error> {
        self.send_no_content(self.delete(&format!("/api/v1/api-keys/{id}")))
            .await
    }

    // ── Users (admin) ──────────────────────────────────────────────

    /// `GET /api/v1/users` -- List users (admin only).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 401 or 403.
    pub async fn list_users(&self) -> Result<ApiResponse<Vec<types::UserResponse>>, Error> {
        self.send_envelope(self.get("/api/v1/users")).await
    }

    /// `POST /api/v1/users` -- Create a user (admin only).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 400, 401, 403, or 409.
    pub async fn create_user(
        &self,
        request: &types::CreateUserRequest,
    ) -> Result<ApiResponse<types::UserResponse>, Error> {
        self.send_envelope(self.post("/api/v1/users").json(request))
            .await
    }

    /// `DELETE /api/v1/users/:id` -- Delete a user (admin only).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 404, 401, or 403.
    pub async fn delete_user(&self, id: Uuid) -> Result<(), Error> {
        self.send_no_content(self.delete(&format!("/api/v1/users/{id}")))
            .await
    }

    /// `PATCH /api/v1/users/:id/role` -- Update a user's role (admin only).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 404, 401, or 403.
    pub async fn update_role(
        &self,
        id: Uuid,
        request: &types::UpdateRoleRequest,
    ) -> Result<ApiResponse<types::UserResponse>, Error> {
        self.send_envelope(
            self.patch(&format!("/api/v1/users/{id}/role"))
                .json(request),
        )
        .await
    }

    // ── Secrets (admin) ────────────────────────────────────────────

    /// `GET /api/v1/secrets` -- List secrets (admin only).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 401 or 403.
    pub async fn list_secrets(&self) -> Result<ApiResponse<Vec<types::SecretResponse>>, Error> {
        self.send_envelope(self.get("/api/v1/secrets")).await
    }

    /// `POST /api/v1/secrets` -- Create a secret (admin only).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 400, 401, or 403.
    pub async fn create_secret(
        &self,
        request: &types::SetSecretRequest,
    ) -> Result<ApiResponse<types::SecretResponse>, Error> {
        self.send_envelope(self.post("/api/v1/secrets").json(request))
            .await
    }

    /// `PUT /api/v1/secrets/:key` -- Update a secret (admin only).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 404, 401, or 403.
    pub async fn update_secret(
        &self,
        key: &str,
        request: &types::UpdateSecretRequest,
    ) -> Result<ApiResponse<types::SecretResponse>, Error> {
        self.send_envelope(self.put(&format!("/api/v1/secrets/{key}")).json(request))
            .await
    }

    /// `DELETE /api/v1/secrets/:key` -- Delete a secret (admin only).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 404, 401, or 403.
    pub async fn delete_secret(&self, key: &str) -> Result<(), Error> {
        self.send_no_content(self.delete(&format!("/api/v1/secrets/{key}")))
            .await
    }

    /// `POST /api/v1/secrets/rotate` -- Re-encrypt one batch of secrets
    /// towards a key version (admin only).
    ///
    /// One call handles one batch. Loop, passing the returned `last_id` back
    /// as `after_id`, until `remaining` reaches zero or `last_id` is `None`.
    /// The operation is idempotent, so an interrupted rotation can simply be
    /// restarted.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 400 (unknown key version), 401, or 403.
    pub async fn rotate_secrets(
        &self,
        request: &types::RotateSecretsRequest,
    ) -> Result<ApiResponse<types::RotateSecretsResponse>, Error> {
        self.send_envelope(self.post("/api/v1/secrets/rotate").json(request))
            .await
    }

    /// `GET /api/v1/secrets/key-versions` -- Encryption key ring status
    /// (admin only).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 401 or 403.
    pub async fn secret_key_versions(
        &self,
    ) -> Result<ApiResponse<types::KeyVersionsResponse>, Error> {
        self.send_envelope(self.get("/api/v1/secrets/key-versions"))
            .await
    }

    // ── Run Logs ───────────────────────────────────────────────────

    /// `GET /api/v1/runs/:id/logs` -- Retrieve persisted log lines for a run.
    ///
    /// Returns log entries with cursor-based pagination. Pass the
    /// `next_cursor` from a previous response to fetch the next page.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 404 (run not found) or 401.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use ironflow_sdk::client::ListRunLogsFilter;
    /// use uuid::Uuid;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let run_id = Uuid::nil();
    /// let logs = client.get_run_logs(run_id, &ListRunLogsFilter::default()).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_run_logs(
        &self,
        run_id: Uuid,
        filter: &ListRunLogsFilter<'_>,
    ) -> Result<ApiResponse<Vec<types::LogEntry>>, Error> {
        self.send_envelope(
            self.get(&format!("/api/v1/runs/{run_id}/logs"))
                .query(filter),
        )
        .await
    }

    // ── Audit Logs (admin) ─────────────────────────────────────────

    /// `GET /api/v1/audit-logs` -- List audit log entries (admin only).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 401 or 403.
    pub async fn list_audit_logs(&self) -> Result<ApiResponse<Vec<types::AuditLogEntry>>, Error> {
        self.list_audit_logs_filtered(&ListAuditLogsFilter::default())
            .await
    }

    /// `GET /api/v1/audit-logs` with query parameters (admin only).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 401 or 403.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use ironflow_sdk::client::ListAuditLogsFilter;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let filter = ListAuditLogsFilter {
    ///     event_type: Some("run_created"),
    ///     ..Default::default()
    /// };
    /// let entries = client.list_audit_logs_filtered(&filter).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_audit_logs_filtered(
        &self,
        filter: &ListAuditLogsFilter<'_>,
    ) -> Result<ApiResponse<Vec<types::AuditLogEntry>>, Error> {
        self.send_envelope(self.get("/api/v1/audit-logs").query(filter))
            .await
    }
}
