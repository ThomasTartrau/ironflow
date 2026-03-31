//! HTTP-based RunStore that talks to the ironflow API internal routes.
//!
//! This is internal to the worker crate — not exposed publicly.

use std::future::Future;
use std::pin::Pin;

use std::time::Duration;

use reqwest::Client;
use uuid::Uuid;

use ironflow_store::entities::{
    NewRun, NewStep, Page, Run, RunFilter, RunStats, RunStatus, RunUpdate, Step, StepUpdate,
};
use ironflow_store::error::StoreError;
use ironflow_store::store::RunStore;

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
    fn create_run(&self, req: NewRun) -> StoreFuture<'_, Run> {
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
            Ok(api_resp.data)
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

    fn pick_next_pending(&self) -> StoreFuture<'_, Option<Run>> {
        Box::pin(async move {
            let resp = self
                .client
                .get(self.internal("/runs/next"))
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(Self::err)?;

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(Self::status_err(&body));
            }

            let api_resp: ApiResponse<Option<Run>> = resp.json().await.map_err(Self::err)?;
            Ok(api_resp.data)
        })
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

    fn get_stats(&self) -> StoreFuture<'_, RunStats> {
        Box::pin(async move {
            Err(StoreError::Database(
                "get_stats not supported via worker API".to_string(),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironflow_store::entities::TriggerKind;

    use serde_json::json;

    #[tokio::test]
    async fn create_run_returns_error_on_unreachable_server() {
        let store = ApiRunStore::new("http://127.0.0.1:1", "token");
        let req = NewRun {
            workflow_name: "test".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
        };
        let result = store.create_run(req).await;
        assert!(result.is_err());
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
        let result = store.get_stats().await;
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
