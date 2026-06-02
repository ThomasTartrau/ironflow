//! Deserialization tests for progenitor-generated types against JSON fixtures.

use ironflow_sdk::types::{
    CreateRunRequest, MeResponse, RunResponse, RunStatus, ScopeEntry, SecretResponse,
    StatsResponse, WorkflowSummary,
};

#[test]
fn deserialize_run_response() {
    let json = r#"{
        "id": "01936f5a-0000-7000-8000-000000000001",
        "workflow_name": "deploy",
        "status": "completed",
        "trigger": {"kind": "manual"},
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:01:00Z",
        "cost_usd": 0.05,
        "duration_ms": 12345,
        "handler_version": "1.0.0",
        "labels": {"env": "prod"},
        "error": null,
        "max_retries": 0,
        "retry_count": 0,
        "payload": {},
        "scheduled_at": null,
        "completed_at": null,
        "started_at": null
    }"#;
    let run: RunResponse = serde_json::from_str(json).unwrap();
    assert_eq!(run.workflow_name, "deploy");
    assert_eq!(run.duration_ms, 12345);
}

#[test]
fn deserialize_run_response_with_nullable_fields() {
    let json = r#"{
        "id": "01936f5a-0000-7000-8000-000000000001",
        "workflow_name": "test",
        "status": "failed",
        "trigger": {"kind": "webhook", "path": "/hooks/deploy"},
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:01:00Z",
        "cost_usd": 0.0,
        "duration_ms": 0,
        "handler_version": null,
        "labels": {},
        "error": "step 2 failed",
        "max_retries": 0,
        "retry_count": 0,
        "payload": {"key": "value"},
        "scheduled_at": null,
        "completed_at": null,
        "started_at": null
    }"#;
    let run: RunResponse = serde_json::from_str(json).unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert!(run.handler_version.is_none());
    assert_eq!(run.error.as_deref(), Some("step 2 failed"));
}

#[test]
fn deserialize_stats_response() {
    let json = r#"{
        "total_runs": 100,
        "completed_runs": 80,
        "failed_runs": 10,
        "cancelled_runs": 3,
        "active_runs": 7,
        "total_cost_usd": 12.50,
        "total_duration_ms": 500000,
        "success_rate_percent": 88.89
    }"#;
    let stats: StatsResponse = serde_json::from_str(json).unwrap();
    assert_eq!(stats.total_runs, 100);
    assert_eq!(stats.completed_runs, 80);
}

#[test]
fn deserialize_workflow_summary() {
    let json = r#"{
        "name": "deploy",
        "category": null,
        "version": null
    }"#;
    let wf: WorkflowSummary = serde_json::from_str(json).unwrap();
    assert_eq!(wf.name, "deploy");
}

#[test]
fn deserialize_me_response() {
    let json = r#"{
        "user_id": "01936f5a-0000-7000-8000-000000000001",
        "username": "alice",
        "email": "alice@example.com",
        "is_admin": true
    }"#;
    let me: MeResponse = serde_json::from_str(json).unwrap();
    assert_eq!(me.username, "alice");
    assert!(me.is_admin);
}

#[test]
fn deserialize_create_run_request() {
    let json = r#"{
        "workflow": "deploy",
        "payload": {"env": "staging"},
        "labels": {"team": "backend"}
    }"#;
    let req: CreateRunRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.workflow, "deploy");
}

#[test]
fn deserialize_secret_response() {
    let json = r#"{
        "id": "01936f5a-0000-7000-8000-000000000001",
        "key": "API_KEY",
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z"
    }"#;
    let secret: SecretResponse = serde_json::from_str(json).unwrap();
    assert_eq!(secret.key, "API_KEY");
}

#[test]
fn deserialize_scope_entry() {
    let json = r#"{
        "value": "runs_read",
        "label": "Runs Read",
        "description": "Read run data"
    }"#;
    let scope: ScopeEntry = serde_json::from_str(json).unwrap();
    assert_eq!(scope.value, "runs_read");
    assert_eq!(scope.label, "Runs Read");
}

#[test]
fn deserialize_run_status_enum() {
    let variants = [
        ("\"pending\"", RunStatus::Pending),
        ("\"running\"", RunStatus::Running),
        ("\"completed\"", RunStatus::Completed),
        ("\"failed\"", RunStatus::Failed),
        ("\"cancelled\"", RunStatus::Cancelled),
    ];
    for (json, expected) in variants {
        let status: RunStatus = serde_json::from_str(json).unwrap();
        assert_eq!(status, expected);
    }
}
