use std::sync::Arc;

use ironflow_core::error::OperationError;
use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_loki::LokiClient;
use ironflow_ops_loki::labels::GetLabelValues;
use ironflow_ops_loki::patterns::GetDetectedFieldValues;
use ironflow_ops_loki::rules::{
    CreateRuleGroup, DeleteRuleGroup, GetRuleGroup, GetRulesByNamespace,
};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

fn loki() -> LokiClient {
    LokiClient::new("http://localhost:3100", reqwest::Client::new())
}

fn assert_external_error(err: OperationError) {
    match err {
        OperationError::External { origin, message } => {
            assert_eq!(origin, "loki");
            assert!(
                message.contains("must be non-empty"),
                "unexpected error message: {message}"
            );
        }
        other => panic!("expected External error, got: {other}"),
    }
}

#[tokio::test]
async fn get_label_values_rejects_path_traversal() {
    let op = GetLabelValues::new(loki(), "../admin");
    let err = op.execute(&ctx()).await.unwrap_err();
    assert_external_error(err);
}

#[tokio::test]
async fn get_label_values_rejects_slash() {
    let op = GetLabelValues::new(loki(), "a/b");
    let err = op.execute(&ctx()).await.unwrap_err();
    assert_external_error(err);
}

#[tokio::test]
async fn get_rules_by_namespace_rejects_path_traversal() {
    let op = GetRulesByNamespace::new(loki(), "../etc");
    let err = op.execute(&ctx()).await.unwrap_err();
    assert_external_error(err);
}

#[tokio::test]
async fn get_rule_group_rejects_traversal_in_namespace() {
    let op = GetRuleGroup::new(loki(), "../etc", "group");
    let err = op.execute(&ctx()).await.unwrap_err();
    assert_external_error(err);
}

#[tokio::test]
async fn get_rule_group_rejects_traversal_in_group() {
    let op = GetRuleGroup::new(loki(), "production", "../secret");
    let err = op.execute(&ctx()).await.unwrap_err();
    assert_external_error(err);
}

#[tokio::test]
async fn create_rule_group_rejects_path_traversal() {
    let op = CreateRuleGroup::new(loki(), "../admin", "body");
    let err = op.execute(&ctx()).await.unwrap_err();
    assert_external_error(err);
}

#[tokio::test]
async fn delete_rule_group_rejects_empty_namespace() {
    let op = DeleteRuleGroup::new(loki(), "", "group");
    let err = op.execute(&ctx()).await.unwrap_err();
    assert_external_error(err);
}

#[tokio::test]
async fn get_detected_field_values_rejects_path_traversal() {
    let op = GetDetectedFieldValues::new(loki(), "../secret", r#"{job="app"}"#);
    let err = op.execute(&ctx()).await.unwrap_err();
    assert_external_error(err);
}
