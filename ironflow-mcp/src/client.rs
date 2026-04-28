//! HTTP client for the Ironflow public API.

use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tracing::debug;

use crate::error::McpError;

/// Ironflow API response envelope.
#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    data: T,
}

/// Ironflow API error envelope.
#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    error: ApiErrorDetail,
}

/// Detail inside an API error response.
#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    message: String,
}

/// HTTP client for the Ironflow public REST API.
///
/// Handles authentication via Bearer token and unwraps
/// the standard `{ "data": ..., "meta": ... }` envelope.
pub struct ApiClient {
    client: Client,
    base_url: String,
    token: String,
}

impl ApiClient {
    /// Create a new client with an API key (`irfl_...`).
    pub fn new(base_url: &str, token: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1{}", self.base_url, path)
    }

    /// Send a GET request and deserialize the `data` field from the response.
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, McpError> {
        debug!(path, "GET");
        let resp = self
            .client
            .get(self.url(path))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(McpError::Http)?;

        self.handle_response(resp).await
    }

    /// Send a POST request with a JSON body.
    pub async fn post<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T, McpError> {
        debug!(path, "POST");
        let resp = self
            .client
            .post(self.url(path))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(McpError::Http)?;

        self.handle_response(resp).await
    }

    /// Send a POST request without a body, returning raw JSON.
    pub async fn post_action(&self, path: &str) -> Result<Value, McpError> {
        debug!(path, "POST action");
        let resp = self
            .client
            .post(self.url(path))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(McpError::Http)?;

        self.handle_response(resp).await
    }

    /// Send a GET request with query params and return the full raw JSON
    /// response (the envelope is NOT unwrapped -- includes `data` and `meta`).
    pub async fn get_raw_with_query(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Value, McpError> {
        debug!(path, ?query, "GET raw with query");
        let resp = self
            .client
            .get(self.url(path))
            .bearer_auth(&self.token)
            .query(query)
            .send()
            .await
            .map_err(McpError::Http)?;

        self.handle_raw_response(resp).await
    }

    async fn handle_raw_response(&self, resp: reqwest::Response) -> Result<Value, McpError> {
        let status = resp.status();
        let body = resp.text().await.map_err(McpError::Http)?;

        if !status.is_success() {
            let message = serde_json::from_str::<ApiErrorResponse>(&body)
                .map(|e| e.error.message)
                .unwrap_or(body);
            return Err(McpError::Api {
                status: status.as_u16(),
                message,
            });
        }

        serde_json::from_str(&body).map_err(|e| McpError::Deserialize(format!("{e}: {body}")))
    }

    async fn handle_response<T: DeserializeOwned>(
        &self,
        resp: reqwest::Response,
    ) -> Result<T, McpError> {
        let raw = self.handle_raw_response(resp).await?;
        let api_resp: ApiResponse<T> =
            serde_json::from_value(raw).map_err(|e| McpError::Deserialize(e.to_string()))?;
        Ok(api_resp.data)
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::Arc;

    use axum::extract::Query;
    use axum::http::header::AUTHORIZATION;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde::Deserialize;
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    use super::*;

    async fn start_server(router: Router) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        addr
    }

    fn client_for(addr: SocketAddr) -> ApiClient {
        ApiClient::new(&format!("http://{addr}"), "test-key".to_string())
    }

    // ---------------------------------------------------------------
    // new() / url()
    // ---------------------------------------------------------------

    #[test]
    fn new_trims_trailing_slash() {
        let c = ApiClient::new("http://localhost:3000/", "tok".to_string());
        assert_eq!(c.base_url, "http://localhost:3000");
    }

    #[test]
    fn new_trims_multiple_trailing_slashes() {
        let c = ApiClient::new("http://localhost:3000///", "tok".to_string());
        assert_eq!(c.base_url, "http://localhost:3000");
    }

    #[test]
    fn new_no_trailing_slash_unchanged() {
        let c = ApiClient::new("http://localhost:3000", "tok".to_string());
        assert_eq!(c.base_url, "http://localhost:3000");
    }

    #[test]
    fn url_builds_versioned_path() {
        let c = ApiClient::new("http://host:8080", "tok".to_string());
        assert_eq!(c.url("/workflows"), "http://host:8080/api/v1/workflows");
    }

    // ---------------------------------------------------------------
    // get()
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn get_unwraps_data_envelope() {
        let app = Router::new().route(
            "/api/v1/items",
            get(|| async { Json(json!({ "data": { "id": 42 } })) }),
        );
        let addr = start_server(app).await;
        let client = client_for(addr);

        let result: Value = client.get("/items").await.unwrap();
        assert_eq!(result, json!({ "id": 42 }));
    }

    #[tokio::test]
    async fn get_sends_bearer_auth() {
        let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let captured_clone = captured.clone();

        let app = Router::new().route(
            "/api/v1/check",
            get(move |headers: axum::http::HeaderMap| {
                let captured = captured_clone.clone();
                async move {
                    let auth = headers
                        .get(AUTHORIZATION)
                        .map(|v| v.to_str().unwrap().to_string());
                    *captured.lock().await = auth;
                    Json(json!({ "data": null }))
                }
            }),
        );
        let addr = start_server(app).await;
        let client = ApiClient::new(&format!("http://{addr}"), "irfl_secret".to_string());

        let _: Value = client.get("/check").await.unwrap();
        let header = captured.lock().await.clone().unwrap();
        assert_eq!(header, "Bearer irfl_secret");
    }

    #[tokio::test]
    async fn get_returns_api_error_on_404() {
        let app = Router::new().route(
            "/api/v1/missing",
            get(|| async {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({ "error": { "code": "NOT_FOUND", "message": "introuvable" } })),
                )
                    .into_response()
            }),
        );
        let addr = start_server(app).await;
        let client = client_for(addr);

        let err = client.get::<Value>("/missing").await.unwrap_err();
        match err {
            McpError::Api { status, message } => {
                assert_eq!(status, 404);
                assert_eq!(message, "introuvable");
            }
            other => panic!("expected McpError::Api, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_returns_api_error_with_raw_body_on_non_json() {
        let app = Router::new().route(
            "/api/v1/bad",
            get(|| async { (StatusCode::BAD_GATEWAY, "upstream down").into_response() }),
        );
        let addr = start_server(app).await;
        let client = client_for(addr);

        let err = client.get::<Value>("/bad").await.unwrap_err();
        match err {
            McpError::Api { status, message } => {
                assert_eq!(status, 502);
                assert_eq!(message, "upstream down");
            }
            other => panic!("expected McpError::Api, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_returns_deserialize_error_on_missing_data_field() {
        let app = Router::new().route(
            "/api/v1/no-envelope",
            get(|| async { Json(json!({ "result": 1 })) }),
        );
        let addr = start_server(app).await;
        let client = client_for(addr);

        let err = client.get::<Value>("/no-envelope").await.unwrap_err();
        assert!(matches!(err, McpError::Deserialize(_)));
    }

    // ---------------------------------------------------------------
    // post()
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn post_sends_json_body_and_unwraps_data() {
        let received: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let received_clone = received.clone();

        let app = Router::new().route(
            "/api/v1/runs",
            post(move |Json(body): Json<Value>| {
                let received = received_clone.clone();
                async move {
                    *received.lock().await = Some(body);
                    (
                        StatusCode::CREATED,
                        Json(json!({ "data": { "id": "run-1" } })),
                    )
                }
            }),
        );
        let addr = start_server(app).await;
        let client = client_for(addr);

        let payload = json!({ "workflow": "deploy" });
        let result: Value = client.post("/runs", &payload).await.unwrap();

        assert_eq!(result, json!({ "id": "run-1" }));
        let body = received.lock().await.clone().unwrap();
        assert_eq!(body, json!({ "workflow": "deploy" }));
    }

    #[tokio::test]
    async fn post_returns_api_error_on_422() {
        let app = Router::new().route(
            "/api/v1/runs",
            post(|| async {
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({ "error": { "code": "VALIDATION", "message": "champ manquant" } })),
                )
                    .into_response()
            }),
        );
        let addr = start_server(app).await;
        let client = client_for(addr);

        let err = client.post::<Value>("/runs", &json!({})).await.unwrap_err();
        match err {
            McpError::Api { status, message } => {
                assert_eq!(status, 422);
                assert_eq!(message, "champ manquant");
            }
            other => panic!("expected McpError::Api, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // post_action()
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn post_action_sends_empty_body_and_unwraps_data() {
        let app = Router::new().route(
            "/api/v1/runs/1/cancel",
            post(|| async { Json(json!({ "data": { "status": "cancelled" } })) }),
        );
        let addr = start_server(app).await;
        let client = client_for(addr);

        let result = client.post_action("/runs/1/cancel").await.unwrap();
        assert_eq!(result, json!({ "status": "cancelled" }));
    }

    #[tokio::test]
    async fn post_action_returns_api_error_on_409() {
        let app = Router::new().route(
            "/api/v1/runs/1/cancel",
            post(|| async {
                (
                    StatusCode::CONFLICT,
                    Json(json!({ "error": { "code": "CONFLICT", "message": "deja termine" } })),
                )
                    .into_response()
            }),
        );
        let addr = start_server(app).await;
        let client = client_for(addr);

        let err = client.post_action("/runs/1/cancel").await.unwrap_err();
        match err {
            McpError::Api { status, message } => {
                assert_eq!(status, 409);
                assert_eq!(message, "deja termine");
            }
            other => panic!("expected McpError::Api, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // get_raw_with_query()
    // ---------------------------------------------------------------

    #[derive(Deserialize)]
    struct ListParams {
        page: Option<String>,
        per_page: Option<String>,
        status: Option<String>,
    }

    #[tokio::test]
    async fn get_raw_with_query_sends_params_and_returns_full_envelope() {
        let app = Router::new().route(
            "/api/v1/runs",
            get(|Query(params): Query<ListParams>| async move {
                Json(json!({
                    "data": [{ "id": "r1" }],
                    "meta": {
                        "page": params.page.unwrap_or_default(),
                        "per_page": params.per_page.unwrap_or_default(),
                        "status": params.status
                    }
                }))
            }),
        );
        let addr = start_server(app).await;
        let client = client_for(addr);

        let result = client
            .get_raw_with_query("/runs", &[("page", "2"), ("per_page", "10"), ("status", "running")])
            .await
            .unwrap();

        assert_eq!(result["data"][0]["id"], "r1");
        assert_eq!(result["meta"]["page"], "2");
        assert_eq!(result["meta"]["per_page"], "10");
        assert_eq!(result["meta"]["status"], "running");
    }

    #[tokio::test]
    async fn get_raw_with_query_returns_api_error_on_500() {
        let app = Router::new().route(
            "/api/v1/runs",
            get(|| async {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": { "code": "INTERNAL", "message": "erreur interne" } })),
                )
                    .into_response()
            }),
        );
        let addr = start_server(app).await;
        let client = client_for(addr);

        let err = client
            .get_raw_with_query("/runs", &[])
            .await
            .unwrap_err();
        match err {
            McpError::Api { status, message } => {
                assert_eq!(status, 500);
                assert_eq!(message, "erreur interne");
            }
            other => panic!("expected McpError::Api, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_raw_with_empty_query_works() {
        let app = Router::new().route(
            "/api/v1/stats",
            get(|| async { Json(json!({ "data": { "total": 5 } })) }),
        );
        let addr = start_server(app).await;
        let client = client_for(addr);

        let result = client.get_raw_with_query("/stats", &[]).await.unwrap();
        assert_eq!(result["data"]["total"], 5);
    }
}
