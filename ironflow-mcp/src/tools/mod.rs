//! MCP tool definitions for Ironflow.
//!
//! Each tool lives in its own file, grouped by domain:
//! - `workflows/` - list and inspect workflows
//! - `runs/` - create, list, and inspect runs
//! - `actions/` - cancel, approve, reject, retry runs
//! - `stats.rs` - aggregated statistics

pub mod actions;
pub mod runs;
pub mod stats;
pub mod workflows;

pub use actions::{ApproveRunTool, CancelRunTool, RejectRunTool, RetryRunTool};
pub use runs::{CreateRunTool, GetRunTool, ListRunsTool};
pub use stats::GetStatsTool;
pub use workflows::{GetWorkflowTool, ListWorkflowsTool};

rust_mcp_sdk::tool_box!(
    IronflowTools,
    [
        ListWorkflowsTool,
        GetWorkflowTool,
        CreateRunTool,
        ListRunsTool,
        GetRunTool,
        CancelRunTool,
        ApproveRunTool,
        RejectRunTool,
        RetryRunTool,
        GetStatsTool
    ]
);

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::net::SocketAddr;

    use axum::extract::{Path, Query};
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use rust_mcp_sdk::schema::CallToolResult;
    use serde_json::{Value, json};
    use tokio::net::TcpListener;

    use crate::client::ApiClient;

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

    fn extract_text(result: &CallToolResult) -> String {
        result.content[0].as_text_content().unwrap().text.clone()
    }

    fn extract_json(result: &CallToolResult) -> Value {
        serde_json::from_str(&extract_text(result)).unwrap()
    }

    fn api_router() -> Router {
        Router::new()
            .route(
                "/api/v1/workflows",
                get(|| async {
                    Json(json!({
                        "data": [
                            { "name": "deploy", "category": "infra/prod" },
                            { "name": "backup", "category": null }
                        ]
                    }))
                }),
            )
            .route(
                "/api/v1/workflows/{name}",
                get(|Path(name): Path<String>| async move {
                    if name == "unknown" {
                        return (
                            StatusCode::NOT_FOUND,
                            Json(json!({ "error": { "code": "NOT_FOUND", "message": "workflow introuvable" } })),
                        )
                            .into_response();
                    }
                    Json(json!({ "data": { "name": name, "steps": 3 } })).into_response()
                }),
            )
            .route(
                "/api/v1/runs",
                get(|Query(params): Query<HashMap<String, String>>| async move {
                    Json(json!({
                        "data": [{ "id": "r1", "status": "completed" }],
                        "meta": {
                            "page": params.get("page").cloned().unwrap_or("1".to_string()),
                            "per_page": params.get("per_page").cloned().unwrap_or("20".to_string()),
                            "workflow": params.get("workflow").cloned(),
                            "status": params.get("status").cloned(),
                            "created_by": params.get("created_by").cloned()
                        }
                    }))
                })
                .post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                    let key = headers
                        .get("idempotency-key")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string);
                    let mut data = json!({
                        "id": "new-run",
                        "workflow": body["workflow"],
                        "payload": body["payload"],
                        "max_retries": body["max_retries"],
                        "status": "pending",
                        "idempotency_key": key
                    });
                    // The real API echoes the cap back and omits it when absent.
                    if let Some(cap) = body.get("max_cost_usd") {
                        data["max_cost_usd"] = cap.clone();
                    }
                    (StatusCode::CREATED, Json(json!({ "data": data })))
                }),
            )
            .route(
                "/api/v1/runs/{id}",
                get(|Path(id): Path<String>| async move {
                    if id == "missing" {
                        return (
                            StatusCode::NOT_FOUND,
                            Json(json!({ "error": { "code": "NOT_FOUND", "message": "run introuvable" } })),
                        )
                            .into_response();
                    }
                    Json(json!({ "data": { "id": id, "status": "running", "steps": [] } }))
                        .into_response()
                }),
            )
            .route(
                "/api/v1/runs/{id}/cancel",
                post(|Path(id): Path<String>| async move {
                    Json(json!({ "data": { "id": id, "status": "cancelled" } }))
                }),
            )
            .route(
                "/api/v1/runs/{id}/approve",
                post(|Path(id): Path<String>| async move {
                    Json(json!({ "data": { "id": id, "status": "running" } }))
                }),
            )
            .route(
                "/api/v1/runs/{id}/reject",
                post(|Path(id): Path<String>| async move {
                    Json(json!({ "data": { "id": id, "status": "failed" } }))
                }),
            )
            .route(
                "/api/v1/runs/{id}/retry",
                post(|Path(id): Path<String>| async move {
                    Json(json!({ "data": { "id": id, "status": "pending" } }))
                }),
            )
            .route(
                "/api/v1/stats",
                get(|| async {
                    Json(json!({
                        "data": {
                            "total": 42,
                            "completed": 30,
                            "failed": 5,
                            "active": 7
                        }
                    }))
                }),
            )
    }

    // ---------------------------------------------------------------
    // ListWorkflowsTool
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn list_workflows_returns_formatted_json() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = ListWorkflowsTool {};

        let result = tool.run(&client).await.unwrap();
        let text = extract_text(&result);
        let parsed: Vec<Value> = serde_json::from_str(&text).unwrap();

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["name"], "deploy");
        assert_eq!(parsed[0]["category"], "infra/prod");
        assert_eq!(parsed[1]["name"], "backup");
        assert!(parsed[1]["category"].is_null());
    }

    // ---------------------------------------------------------------
    // GetWorkflowTool
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn get_workflow_returns_detail() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = GetWorkflowTool {
            name: "deploy".to_string(),
        };

        let result = tool.run(&client).await.unwrap();
        let parsed = extract_json(&result);

        assert_eq!(parsed["name"], "deploy");
        assert_eq!(parsed["steps"], 3);
    }

    #[tokio::test]
    async fn get_workflow_propagates_404() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = GetWorkflowTool {
            name: "unknown".to_string(),
        };

        let err = tool.run(&client).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("introuvable"), "got: {msg}");
    }

    // ---------------------------------------------------------------
    // CreateRunTool
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn create_run_with_payload() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = CreateRunTool {
            workflow: "deploy".to_string(),
            payload: Some(r#"{"env":"prod"}"#.to_string()),
            max_retries: Some(2),
            idempotency_key: None,
            max_cost_usd: None,
        };

        let result = tool.run(&client).await.unwrap();
        let parsed = extract_json(&result);

        assert_eq!(parsed["workflow"], "deploy");
        assert_eq!(parsed["payload"]["env"], "prod");
        assert_eq!(parsed["max_retries"], 2);
        assert_eq!(parsed["status"], "pending");
    }

    #[tokio::test]
    async fn create_run_without_payload_sends_empty_object() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = CreateRunTool {
            workflow: "backup".to_string(),
            payload: None,
            max_retries: None,
            idempotency_key: None,
            max_cost_usd: None,
        };

        let result = tool.run(&client).await.unwrap();
        let parsed = extract_json(&result);

        assert_eq!(parsed["workflow"], "backup");
        assert_eq!(parsed["payload"], json!({}));
        assert_eq!(parsed["max_retries"], 0, "no automatic retry by default");
    }

    #[tokio::test]
    async fn create_run_with_invalid_json_payload_defaults_to_empty() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = CreateRunTool {
            workflow: "deploy".to_string(),
            payload: Some("not-json".to_string()),
            max_retries: None,
            idempotency_key: None,
            max_cost_usd: None,
        };

        let result = tool.run(&client).await.unwrap();
        let parsed = extract_json(&result);

        assert_eq!(parsed["payload"], json!({}));
    }

    #[tokio::test]
    async fn create_run_forwards_the_idempotency_key() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = CreateRunTool {
            workflow: "deploy".to_string(),
            payload: Some(r#"{"env":"prod"}"#.to_string()),
            max_retries: None,
            idempotency_key: Some("github:abc-123".to_string()),
            max_cost_usd: None,
        };

        let result = tool.run(&client).await.unwrap();
        let parsed = extract_json(&result);

        assert_eq!(parsed["idempotency_key"], "github:abc-123");
    }

    #[tokio::test]
    async fn create_run_without_a_key_sends_no_header() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = CreateRunTool {
            workflow: "deploy".to_string(),
            payload: None,
            max_retries: None,
            idempotency_key: None,
            max_cost_usd: None,
        };

        let result = tool.run(&client).await.unwrap();
        let parsed = extract_json(&result);

        assert!(parsed["idempotency_key"].is_null());
    }

    #[tokio::test]
    async fn create_run_forwards_max_cost_usd() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = CreateRunTool {
            workflow: "deploy".to_string(),
            payload: None,
            max_retries: None,
            idempotency_key: None,
            max_cost_usd: Some(2.5),
        };

        let result = tool.run(&client).await.unwrap();
        let parsed = extract_json(&result);

        assert_eq!(parsed["max_cost_usd"], 2.5);
    }

    #[tokio::test]
    async fn create_run_omits_max_cost_usd_when_absent() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = CreateRunTool {
            workflow: "deploy".to_string(),
            payload: None,
            max_retries: None,
            idempotency_key: None,
            max_cost_usd: None,
        };

        let result = tool.run(&client).await.unwrap();
        let parsed = extract_json(&result);

        assert!(parsed.get("max_cost_usd").is_none());
    }

    // ---------------------------------------------------------------
    // ListRunsTool
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn list_runs_with_all_filters() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = ListRunsTool {
            workflow: Some("deploy".to_string()),
            status: Some("running".to_string()),
            created_by: Some("019a3f2b-0000-7000-8000-000000000000".to_string()),
            page: Some(2),
            per_page: Some(10),
        };

        let result = tool.run(&client).await.unwrap();
        let parsed = extract_json(&result);

        assert_eq!(parsed["meta"]["page"], "2");
        assert_eq!(parsed["meta"]["per_page"], "10");
        assert_eq!(parsed["meta"]["workflow"], "deploy");
        assert_eq!(parsed["meta"]["status"], "running");
        assert_eq!(
            parsed["meta"]["created_by"],
            "019a3f2b-0000-7000-8000-000000000000"
        );
        assert_eq!(parsed["data"][0]["id"], "r1");
    }

    #[tokio::test]
    async fn list_runs_without_filters() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = ListRunsTool {
            workflow: None,
            status: None,
            created_by: None,
            page: None,
            per_page: None,
        };

        let result = tool.run(&client).await.unwrap();
        let parsed = extract_json(&result);

        assert_eq!(parsed["meta"]["page"], "1");
        assert_eq!(parsed["meta"]["per_page"], "20");
        assert!(parsed["meta"]["workflow"].is_null());
        assert!(parsed["meta"]["status"].is_null());
        assert!(parsed["meta"]["created_by"].is_null());
    }

    // ---------------------------------------------------------------
    // GetRunTool
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn get_run_returns_detail() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = GetRunTool {
            run_id: "abc-123".to_string(),
        };

        let result = tool.run(&client).await.unwrap();
        let parsed = extract_json(&result);

        assert_eq!(parsed["id"], "abc-123");
        assert_eq!(parsed["status"], "running");
    }

    #[tokio::test]
    async fn get_run_propagates_404() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = GetRunTool {
            run_id: "missing".to_string(),
        };

        let err = tool.run(&client).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("introuvable"), "got: {msg}");
    }

    // ---------------------------------------------------------------
    // CancelRunTool
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn cancel_run_returns_cancelled_status() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = CancelRunTool {
            run_id: "r1".to_string(),
        };

        let result = tool.run(&client).await.unwrap();
        let parsed = extract_json(&result);

        assert_eq!(parsed["id"], "r1");
        assert_eq!(parsed["status"], "cancelled");
    }

    // ---------------------------------------------------------------
    // ApproveRunTool
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn approve_run_returns_running_status() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = ApproveRunTool {
            run_id: "r2".to_string(),
        };

        let result = tool.run(&client).await.unwrap();
        let parsed = extract_json(&result);

        assert_eq!(parsed["id"], "r2");
        assert_eq!(parsed["status"], "running");
    }

    // ---------------------------------------------------------------
    // RejectRunTool
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn reject_run_returns_failed_status() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = RejectRunTool {
            run_id: "r3".to_string(),
        };

        let result = tool.run(&client).await.unwrap();
        let parsed = extract_json(&result);

        assert_eq!(parsed["id"], "r3");
        assert_eq!(parsed["status"], "failed");
    }

    // ---------------------------------------------------------------
    // RetryRunTool
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn retry_run_returns_pending_status() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = RetryRunTool {
            run_id: "r4".to_string(),
        };

        let result = tool.run(&client).await.unwrap();
        let parsed = extract_json(&result);

        assert_eq!(parsed["id"], "r4");
        assert_eq!(parsed["status"], "pending");
    }

    // ---------------------------------------------------------------
    // GetStatsTool
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn get_stats_returns_aggregated_data() {
        let addr = start_server(api_router()).await;
        let client = client_for(addr);
        let tool = GetStatsTool {};

        let result = tool.run(&client).await.unwrap();
        let parsed = extract_json(&result);

        assert_eq!(parsed["total"], 42);
        assert_eq!(parsed["completed"], 30);
        assert_eq!(parsed["failed"], 5);
        assert_eq!(parsed["active"], 7);
    }

    // ---------------------------------------------------------------
    // Error propagation (shared api error path)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn cancel_run_propagates_api_error() {
        let app = Router::new().route(
            "/api/v1/runs/{id}/cancel",
            post(|| async {
                (
                    StatusCode::CONFLICT,
                    Json(json!({ "error": { "code": "CONFLICT", "message": "run deja termine" } })),
                )
                    .into_response()
            }),
        );
        let addr = start_server(app).await;
        let client = client_for(addr);
        let tool = CancelRunTool {
            run_id: "done".to_string(),
        };

        let err = tool.run(&client).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("termine"), "got: {msg}");
    }

    #[tokio::test]
    async fn approve_run_propagates_api_error() {
        let app = Router::new().route(
            "/api/v1/runs/{id}/approve",
            post(|| async {
                (
                    StatusCode::CONFLICT,
                    Json(json!({ "error": { "code": "CONFLICT", "message": "pas en attente" } })),
                )
                    .into_response()
            }),
        );
        let addr = start_server(app).await;
        let client = client_for(addr);
        let tool = ApproveRunTool {
            run_id: "r1".to_string(),
        };

        let err = tool.run(&client).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("attente"), "got: {msg}");
    }
}
