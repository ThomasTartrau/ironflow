use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

use super::*;

struct GitLabIssueOp {
    project_id: u64,
    title: String,
}

#[async_trait]
impl Operation for GitLabIssueOp {
    fn kind(&self) -> &str {
        "gitlab"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        Ok(json!({
            "issue_id": 42,
            "url": "https://gitlab.com/issues/42",
            "project_id": self.project_id,
            "title": self.title
        }))
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "project_id": self.project_id,
            "title": self.title
        }))
    }
}

struct NoInputOp;

#[async_trait]
impl Operation for NoInputOp {
    fn kind(&self) -> &str {
        "noop"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        Ok(json!({"status": "ok"}))
    }
}

struct ErrorOp;

#[async_trait]
impl Operation for ErrorOp {
    fn kind(&self) -> &str {
        "error_test"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        Err(OperationError::Http {
            status: Some(500),
            message: "test error".to_string(),
        })
    }
}

fn test_ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[test]
fn operation_kind_identifies_operation_type() {
    let op = GitLabIssueOp {
        project_id: 123,
        title: "Bug".to_string(),
    };
    assert_eq!(op.kind(), "gitlab");
}

#[test]
fn operation_with_input_provides_structured_logging() {
    let op = GitLabIssueOp {
        project_id: 456,
        title: "Feature request".to_string(),
    };
    let input = op.input();
    assert!(input.is_some());

    let input_value = input.unwrap();
    assert_eq!(input_value["project_id"], 456);
    assert_eq!(input_value["title"], "Feature request");
}

#[test]
fn operation_without_input_returns_none() {
    let op = NoInputOp;
    assert_eq!(op.input(), None);
}

#[tokio::test]
async fn operation_execute_returns_json_output() {
    let ctx = test_ctx();
    let op = GitLabIssueOp {
        project_id: 789,
        title: "Test".to_string(),
    };
    let result = op.execute(&ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    assert_eq!(output["issue_id"], 42);
    assert_eq!(output["project_id"], 789);
    assert_eq!(output["title"], "Test");
}

#[tokio::test]
async fn operation_execute_can_return_error() {
    let ctx = test_ctx();
    let op = ErrorOp;
    let result = op.execute(&ctx).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn noop_secret_resolver_returns_none() {
    let resolver = NoopSecretResolver;
    let result = resolver.get("any_key").await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[test]
fn operation_context_provides_http_client() {
    let ctx = test_ctx();
    let _client = ctx.http_client();
}

#[test]
fn operation_context_with_custom_client() {
    let client = Client::new();
    let ctx = OperationContext::with_http_client(client, Arc::new(NoopSecretResolver));
    let _client = ctx.http_client();
}

#[tokio::test]
async fn operation_context_secrets_delegates_to_resolver() {
    let ctx = test_ctx();
    let result = ctx.secrets().get("missing").await;
    assert!(result.unwrap().is_none());
}

#[test]
fn secret_value_holds_plaintext() {
    let secret = SecretValue {
        value: "sk-ant-12345".to_string(),
    };
    assert_eq!(secret.value, "sk-ant-12345");
}

#[test]
fn secret_value_debug_redacts_value() {
    let secret = SecretValue {
        value: "super-secret-token".to_string(),
    };
    let debug = format!("{secret:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("super-secret-token"));
}

#[test]
fn operation_error_secret_display() {
    let err = OperationError::Secret {
        message: "decryption failed".to_string(),
    };
    assert!(err.to_string().contains("secret error"));
    assert!(err.to_string().contains("decryption failed"));
}
