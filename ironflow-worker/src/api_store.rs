//! HTTP-based [`RunStore`] that talks to the ironflow API internal routes.

use std::future::Future;
use std::pin::Pin;

use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use uuid::Uuid;

use ironflow_store::api_key_store::ApiKeyStore;
use ironflow_store::artifact_store::ArtifactStore;
use ironflow_store::audit_log_store::AuditLogStore;
use ironflow_store::entities::{
    ApiKey, ApiKeyUpdate, Artifact, ArtifactLookup, AuditLogEntry, AuditLogFilter,
    KeyVersionStatus, LeaseRequest, LogEntry, LogFilter, NewApiKey, NewArtifact, NewAuditLogEntry,
    NewLogEntries, NewRun, NewStep, NewStepDependency, NewUser, Page, PurgePolicy, PurgeableRun,
    ReapedRun, RotationBatch, RotationRequest, Run, RunCreation, RunFilter, RunStats, RunStatus,
    RunUpdate, Secret, SecretMetadata, Step, StepDependency, StepUpdate, User,
};
use ironflow_store::error::StoreError;
use ironflow_store::log_store::LogStore;
use ironflow_store::secret_store::SecretStore;
use ironflow_store::store::RunStore;
use ironflow_store::user_store::UserStore;

type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'a>>;

/// API response envelope.
#[derive(serde::Deserialize, serde::Serialize)]
struct ApiResponse<T> {
    data: T,
}

/// RunStore implementation that communicates with the API server via HTTP.
#[derive(Debug, Clone)]
pub struct ApiRunStore {
    client: Client,
    base_url: String,
    token: String,
}

impl ApiRunStore {
    pub fn new(base_url: &str, token: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    fn internal(&self, path: &str) -> String {
        format!("{}/api/v1/internal{}", self.base_url, path)
    }

    fn err(e: reqwest::Error) -> StoreError {
        StoreError::Database(format!("worker HTTP error: {e}"))
    }

    fn status_err(body: &str) -> StoreError {
        StoreError::Database(format!("worker API error: {body}"))
    }
}

impl RunStore for ApiRunStore {
    fn create_run(&self, req: NewRun) -> StoreFuture<'_, RunCreation> {
        Box::pin(async move {
            let resp = self
                .client
                .post(self.internal("/runs"))
                .bearer_auth(&self.token)
                .json(&req)
                .send()
                .await
                .map_err(Self::err)?;

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(Self::status_err(&body));
            }

            let api_resp: ApiResponse<Run> = resp.json().await.map_err(Self::err)?;
            // The internal endpoint is not idempotent: a success is always a creation.
            Ok(RunCreation::Created(api_resp.data))
        })
    }

    fn find_run_by_idempotency_key(&self, _key: &str) -> StoreFuture<'_, Option<Run>> {
        Box::pin(async move {
            Err(StoreError::Database(
                "find_run_by_idempotency_key not supported via worker API".to_string(),
            ))
        })
    }

    fn get_run(&self, id: Uuid) -> StoreFuture<'_, Option<Run>> {
        Box::pin(async move {
            let resp = self
                .client
                .get(self.internal(&format!("/runs/{id}")))
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(Self::err)?;

            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(None);
            }
            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(Self::status_err(&body));
            }

            #[derive(serde::Deserialize)]
            struct RunDetail {
                run: Run,
            }

            let api_resp: ApiResponse<RunDetail> = resp.json().await.map_err(Self::err)?;
            Ok(Some(api_resp.data.run))
        })
    }

    fn list_runs(
        &self,
        _filter: RunFilter,
        _page: u32,
        _per_page: u32,
    ) -> StoreFuture<'_, Page<Run>> {
        Box::pin(async move {
            Err(StoreError::Database(
                "list_runs not supported via worker API".to_string(),
            ))
        })
    }

    fn update_run_status(&self, id: Uuid, new_status: RunStatus) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let resp = self
                .client
                .put(self.internal(&format!("/runs/{id}/status")))
                .bearer_auth(&self.token)
                .json(&serde_json::json!({ "status": new_status }))
                .send()
                .await
                .map_err(Self::err)?;

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(Self::status_err(&body));
            }
            Ok(())
        })
    }

    fn update_run(&self, id: Uuid, update: RunUpdate) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let resp = self
                .client
                .put(self.internal(&format!("/runs/{id}")))
                .bearer_auth(&self.token)
                .json(&update)
                .send()
                .await
                .map_err(Self::err)?;

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(Self::status_err(&body));
            }
            Ok(())
        })
    }

    fn pick_next_pending(&self, lease: Option<LeaseRequest>) -> StoreFuture<'_, Option<Run>> {
        Box::pin(async move {
            let mut request = self
                .client
                .get(self.internal("/runs/next"))
                .bearer_auth(&self.token);

            if let Some(lease) = lease {
                request = request.query(&[
                    ("worker_id", lease.worker_id),
                    ("lease_ttl_secs", lease.ttl.as_secs().to_string()),
                ]);
            }

            let resp = request.send().await.map_err(Self::err)?;

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(Self::status_err(&body));
            }

            let api_resp: ApiResponse<Option<Run>> = resp.json().await.map_err(Self::err)?;
            Ok(api_resp.data)
        })
    }

    fn renew_lease(&self, id: Uuid, lease: LeaseRequest) -> StoreFuture<'_, DateTime<Utc>> {
        Box::pin(async move {
            #[derive(serde::Serialize)]
            struct RenewLeaseBody {
                worker_id: String,
                lease_ttl_secs: u64,
            }

            #[derive(serde::Deserialize)]
            struct RenewLeaseData {
                lease_expires_at: DateTime<Utc>,
            }

            let resp = self
                .client
                .post(self.internal(&format!("/runs/{id}/lease")))
                .bearer_auth(&self.token)
                .json(&RenewLeaseBody {
                    worker_id: lease.worker_id.clone(),
                    lease_ttl_secs: lease.ttl.as_secs(),
                })
                .send()
                .await
                .map_err(Self::err)?;

            // 409 means another worker owns the run now, or the run left the
            // Running state: the caller must abandon it, not retry.
            if resp.status() == StatusCode::CONFLICT {
                return Err(StoreError::LeaseLost {
                    run_id: id,
                    held_by: None,
                });
            }
            if resp.status() == StatusCode::NOT_FOUND {
                return Err(StoreError::RunNotFound(id));
            }
            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(Self::status_err(&body));
            }

            let api_resp: ApiResponse<RenewLeaseData> = resp.json().await.map_err(Self::err)?;
            Ok(api_resp.data.lease_expires_at)
        })
    }

    fn reap_expired_leases(&self, _limit: u32) -> StoreFuture<'_, Vec<ReapedRun>> {
        // Recovery is an API-server responsibility: the worker has no route for
        // it and must never requeue runs it does not own.
        Box::pin(async move { Ok(Vec::new()) })
    }

    fn list_purgeable_runs(
        &self,
        _policy: &PurgePolicy,
        _batch_size: u32,
    ) -> StoreFuture<'_, Vec<PurgeableRun>> {
        // Purging is an API-server responsibility.
        Box::pin(async move { Ok(Vec::new()) })
    }

    fn delete_run(&self, _id: Uuid) -> StoreFuture<'_, Vec<String>> {
        // Purging is an API-server responsibility.
        Box::pin(async move { Ok(Vec::new()) })
    }

    fn create_step(&self, req: NewStep) -> StoreFuture<'_, Step> {
        Box::pin(async move {
            let resp = self
                .client
                .post(self.internal("/steps"))
                .bearer_auth(&self.token)
                .json(&req)
                .send()
                .await
                .map_err(Self::err)?;

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(Self::status_err(&body));
            }

            let api_resp: ApiResponse<Step> = resp.json().await.map_err(Self::err)?;
            Ok(api_resp.data)
        })
    }

    fn update_step(&self, id: Uuid, update: StepUpdate) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let resp = self
                .client
                .put(self.internal(&format!("/steps/{id}")))
                .bearer_auth(&self.token)
                .json(&update)
                .send()
                .await
                .map_err(Self::err)?;

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(Self::status_err(&body));
            }
            Ok(())
        })
    }

    fn get_step(&self, _id: Uuid) -> StoreFuture<'_, Option<Step>> {
        // The worker never reads a step back through its store — step lookup
        // lives on the API side. Return None so this trait method stays total
        // without adding a dedicated HTTP route the worker doesn't use.
        Box::pin(async move { Ok(None) })
    }

    fn list_steps(&self, run_id: Uuid) -> StoreFuture<'_, Vec<Step>> {
        Box::pin(async move {
            let resp = self
                .client
                .get(self.internal(&format!("/runs/{run_id}")))
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(Self::err)?;

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(Self::status_err(&body));
            }

            #[derive(serde::Deserialize)]
            struct RunDetail {
                steps: Vec<Step>,
            }

            let api_resp: ApiResponse<RunDetail> = resp.json().await.map_err(Self::err)?;
            Ok(api_resp.data.steps)
        })
    }

    fn get_stats(&self, _filter: RunFilter) -> StoreFuture<'_, RunStats> {
        Box::pin(async move {
            Err(StoreError::Database(
                "get_stats not supported via worker API".to_string(),
            ))
        })
    }

    fn create_step_dependencies(&self, deps: Vec<NewStepDependency>) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            if deps.is_empty() {
                return Ok(());
            }

            let resp = self
                .client
                .post(self.internal("/step-dependencies"))
                .bearer_auth(&self.token)
                .json(&deps)
                .send()
                .await
                .map_err(Self::err)?;

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(Self::status_err(&body));
            }

            Ok(())
        })
    }

    fn list_step_dependencies(&self, _run_id: Uuid) -> StoreFuture<'_, Vec<StepDependency>> {
        Box::pin(async move {
            Err(StoreError::Database(
                "list_step_dependencies not supported via worker API".to_string(),
            ))
        })
    }
}

impl UserStore for ApiRunStore {
    fn create_user(&self, _req: NewUser) -> StoreFuture<'_, User> {
        Box::pin(async move {
            Err(StoreError::Database(
                "UserStore not available in worker".to_string(),
            ))
        })
    }

    fn find_user_by_email(&self, _email: &str) -> StoreFuture<'_, Option<User>> {
        Box::pin(async move { Ok(None) })
    }

    fn find_user_by_id(&self, _id: Uuid) -> StoreFuture<'_, Option<User>> {
        Box::pin(async move { Ok(None) })
    }

    fn count_users(&self) -> StoreFuture<'_, u64> {
        Box::pin(async move {
            Err(StoreError::Database(
                "UserStore not available in worker".to_string(),
            ))
        })
    }

    fn list_users(&self, _page: u32, _per_page: u32) -> StoreFuture<'_, Page<User>> {
        Box::pin(async move {
            Err(StoreError::Database(
                "UserStore not available in worker".to_string(),
            ))
        })
    }

    fn delete_user(&self, _id: Uuid) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            Err(StoreError::Database(
                "UserStore not available in worker".to_string(),
            ))
        })
    }

    fn update_user_role(&self, _id: Uuid, _is_admin: bool) -> StoreFuture<'_, User> {
        Box::pin(async move {
            Err(StoreError::Database(
                "UserStore not available in worker".to_string(),
            ))
        })
    }
}

impl ApiKeyStore for ApiRunStore {
    fn create_api_key(&self, _req: NewApiKey) -> StoreFuture<'_, ApiKey> {
        Box::pin(async move {
            Err(StoreError::Database(
                "ApiKeyStore not available in worker".to_string(),
            ))
        })
    }

    fn find_api_key_by_prefix(&self, _prefix: &str) -> StoreFuture<'_, Option<ApiKey>> {
        Box::pin(async move { Ok(None) })
    }

    fn find_api_key_by_id(&self, _id: Uuid) -> StoreFuture<'_, Option<ApiKey>> {
        Box::pin(async move { Ok(None) })
    }

    fn list_api_keys_by_user(&self, _user_id: Uuid) -> StoreFuture<'_, Vec<ApiKey>> {
        Box::pin(async move {
            Err(StoreError::Database(
                "ApiKeyStore not available in worker".to_string(),
            ))
        })
    }

    fn update_api_key(&self, _id: Uuid, _update: ApiKeyUpdate) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            Err(StoreError::Database(
                "ApiKeyStore not available in worker".to_string(),
            ))
        })
    }

    fn touch_api_key(&self, _id: Uuid) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            Err(StoreError::Database(
                "ApiKeyStore not available in worker".to_string(),
            ))
        })
    }

    fn delete_api_key(&self, _id: Uuid) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            Err(StoreError::Database(
                "ApiKeyStore not available in worker".to_string(),
            ))
        })
    }
}

impl AuditLogStore for ApiRunStore {
    fn append_audit_log(&self, _entry: NewAuditLogEntry) -> StoreFuture<'_, AuditLogEntry> {
        Box::pin(async move {
            Err(StoreError::Database(
                "AuditLogStore not available in worker".to_string(),
            ))
        })
    }

    fn list_audit_logs(
        &self,
        _filter: AuditLogFilter,
        _page: u32,
        _per_page: u32,
    ) -> StoreFuture<'_, Page<AuditLogEntry>> {
        Box::pin(async move {
            Err(StoreError::Database(
                "AuditLogStore not available in worker".to_string(),
            ))
        })
    }
}

impl SecretStore for ApiRunStore {
    fn get_secret(&self, key: &str) -> StoreFuture<'_, Option<Secret>> {
        let key = key.to_string();
        Box::pin(async move {
            let resp = self
                .client
                .get(self.internal(&format!("/secrets/{key}")))
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(Self::err)?;

            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(None);
            }

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(Self::status_err(&body));
            }

            let api_resp: ApiResponse<Secret> = resp.json().await.map_err(Self::err)?;
            Ok(Some(api_resp.data))
        })
    }

    fn set_secret(&self, _key: &str, _value: &str) -> StoreFuture<'_, Secret> {
        Box::pin(async move {
            Err(StoreError::Database(
                "SecretStore not available in worker".to_string(),
            ))
        })
    }

    fn delete_secret(&self, _key: &str) -> StoreFuture<'_, bool> {
        Box::pin(async move {
            Err(StoreError::Database(
                "SecretStore not available in worker".to_string(),
            ))
        })
    }

    fn list_secret_keys(&self, _prefix: &str) -> StoreFuture<'_, Vec<String>> {
        Box::pin(async move {
            Err(StoreError::Database(
                "SecretStore not available in worker".to_string(),
            ))
        })
    }

    fn list_secrets(
        &self,
        _prefix: &str,
        _page: u32,
        _per_page: u32,
    ) -> StoreFuture<'_, Page<SecretMetadata>> {
        Box::pin(async move {
            Err(StoreError::Database(
                "SecretStore not available in worker".to_string(),
            ))
        })
    }

    fn secret_key_status(&self) -> StoreFuture<'_, KeyVersionStatus> {
        Box::pin(async move {
            Err(StoreError::Database(
                "SecretStore not available in worker".to_string(),
            ))
        })
    }

    fn rotate_secrets(&self, _request: RotationRequest) -> StoreFuture<'_, RotationBatch> {
        Box::pin(async move {
            Err(StoreError::Database(
                "SecretStore not available in worker".to_string(),
            ))
        })
    }
}

impl LogStore for ApiRunStore {
    fn append_logs(&self, _entries: NewLogEntries) -> StoreFuture<'_, ()> {
        // Log persistence is handled by the API server via push_logs.
        Box::pin(async move { Ok(()) })
    }

    fn get_logs(
        &self,
        _run_id: Uuid,
        _filter: LogFilter,
        _cursor: Option<Uuid>,
        _limit: u32,
    ) -> StoreFuture<'_, Vec<LogEntry>> {
        Box::pin(async move {
            Err(StoreError::Database(
                "LogStore not available in worker".to_string(),
            ))
        })
    }
}

impl ArtifactStore for ApiRunStore {
    fn create_artifact(&self, artifact: NewArtifact) -> StoreFuture<'_, Artifact> {
        Box::pin(async move {
            // The worker records an artifact by uploading its bytes, which the
            // API writes and registers in one call. There is no metadata-only
            // route, so this must never be reached from the worker.
            let _ = artifact;
            Err(StoreError::Database(
                "create_artifact not supported via worker API — upload the bytes instead"
                    .to_string(),
            ))
        })
    }

    fn get_artifact(&self, _step_id: Uuid, _name: &str) -> StoreFuture<'_, Option<Artifact>> {
        // The worker resolves artifacts through find_artifact_for_input, which
        // needs the run to scope the search. Keep this total rather than add a
        // route the worker never calls.
        Box::pin(async move { Ok(None) })
    }

    fn list_artifacts_for_run(&self, run_id: Uuid) -> StoreFuture<'_, Vec<Artifact>> {
        Box::pin(async move {
            let resp = self
                .client
                .get(self.internal(&format!("/runs/{run_id}/artifacts")))
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(Self::err)?;

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(Self::status_err(&body));
            }

            let api_resp: ApiResponse<Vec<Artifact>> = resp.json().await.map_err(Self::err)?;
            Ok(api_resp.data)
        })
    }

    fn find_artifact_for_input(&self, lookup: ArtifactLookup) -> StoreFuture<'_, Option<Artifact>> {
        Box::pin(async move {
            // Resolved worker-side from the run's steps and artifacts, so the
            // matching rule stays in one place instead of being duplicated in a
            // dedicated route.
            let steps = self.list_steps(lookup.run_id).await?;

            let Some(producer) = steps
                .iter()
                .filter(|step| {
                    step.attempt == lookup.attempt
                        && step.name == lookup.step_name
                        && step.position < lookup.before_position
                })
                .max_by_key(|step| step.position)
            else {
                return Ok(None);
            };

            let artifacts = self.list_artifacts_for_run(lookup.run_id).await?;
            Ok(artifacts
                .into_iter()
                .find(|artifact| artifact.step_id == producer.id && artifact.name == lookup.name))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    use ironflow_store::entities::TriggerKind;

    use serde_json::json;

    #[tokio::test]
    async fn create_run_returns_error_on_unreachable_server() {
        let store = ApiRunStore::new("http://127.0.0.1:1", "token");
        let req = NewRun {
            created_by: None,
            workflow_name: "test".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            idempotency_key: None,
            max_cost_usd: None,
        };
        let result = store.create_run(req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn find_run_by_idempotency_key_not_supported() {
        let store = ApiRunStore::new("http://localhost:3000", "token");
        let result = store.find_run_by_idempotency_key("github:abc").await;
        match result {
            Err(StoreError::Database(msg)) => assert!(msg.contains("not supported")),
            other => panic!("expected an unsupported-operation error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_runs_not_supported() {
        let store = ApiRunStore::new("http://localhost:3000", "token");
        let result = store
            .list_runs(ironflow_store::entities::RunFilter::default(), 0, 10)
            .await;
        assert!(result.is_err());
        match result {
            Err(StoreError::Database(msg)) => {
                assert!(msg.contains("not supported"));
            }
            _ => panic!("expected Database error"),
        }
    }

    #[tokio::test]
    async fn get_stats_not_supported() {
        let store = ApiRunStore::new("http://localhost:3000", "token");
        let result = store.get_stats(RunFilter::default()).await;
        assert!(result.is_err());
        match result {
            Err(StoreError::Database(msg)) => {
                assert!(msg.contains("not supported"));
            }
            _ => panic!("expected Database error"),
        }
    }

    #[test]
    fn api_run_store_clone() {
        let store = ApiRunStore::new("http://localhost:3000", "token");
        let store2 = store.clone();
        assert_eq!(store.base_url, store2.base_url);
        assert_eq!(store.token, store2.token);
    }

    #[test]
    fn api_run_store_with_trailing_slash() {
        let store = ApiRunStore::new("http://localhost:3000/", "token");
        assert_eq!(store.base_url, "http://localhost:3000");
    }

    #[test]
    fn api_run_store_without_trailing_slash() {
        let store = ApiRunStore::new("http://localhost:3000", "token");
        assert_eq!(store.base_url, "http://localhost:3000");
    }

    #[test]
    fn api_run_store_builds_internal_url() {
        let store = ApiRunStore::new("http://localhost:3000", "token");
        let url = store.internal("/runs/123");
        assert_eq!(url, "http://localhost:3000/api/v1/internal/runs/123");
    }
}
