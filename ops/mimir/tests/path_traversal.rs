use std::sync::Arc;

use ironflow_core::error::OperationError;
use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_mimir::MimirClient;
use ironflow_ops_mimir::compactor::{FinishBlockUpload, StartBlockUpload, UploadBlockFile};
use ironflow_ops_mimir::rules::{
    CreateRuleGroup, DeleteRuleGroup, GetRuleGroup, GetRulesByNamespace,
};
use ironflow_ops_mimir::series::GetLabelValues;
use ironflow_ops_mimir::store_gateway::GetTenantBlocks;
use reqwest::Client;

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

fn mimir() -> MimirClient {
    MimirClient::new("http://localhost:8080", Client::new())
}

fn assert_path_traversal_error(err: OperationError) {
    match err {
        OperationError::External { origin, message } => {
            assert_eq!(origin, "mimir");
            assert!(
                message.contains("must be non-empty"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected External error, got: {other}"),
    }
}

#[tokio::test]
async fn rules_namespace_rejects_traversal() {
    let err = GetRulesByNamespace::new(mimir(), "../admin")
        .execute(&ctx())
        .await
        .unwrap_err();
    assert_path_traversal_error(err);
}

#[tokio::test]
async fn rules_group_rejects_traversal() {
    let err = GetRuleGroup::new(mimir(), "ns", "../admin")
        .execute(&ctx())
        .await
        .unwrap_err();
    assert_path_traversal_error(err);
}

#[tokio::test]
async fn create_rule_group_rejects_traversal_namespace() {
    let err = CreateRuleGroup::new(mimir(), "../admin", "body")
        .execute(&ctx())
        .await
        .unwrap_err();
    assert_path_traversal_error(err);
}

#[tokio::test]
async fn delete_rule_group_rejects_traversal() {
    let err = DeleteRuleGroup::new(mimir(), "ns", "../admin")
        .execute(&ctx())
        .await
        .unwrap_err();
    assert_path_traversal_error(err);
}

#[tokio::test]
async fn tenant_blocks_rejects_traversal() {
    let err = GetTenantBlocks::new(mimir(), "../admin")
        .execute(&ctx())
        .await
        .unwrap_err();
    assert_path_traversal_error(err);
}

#[tokio::test]
async fn start_block_upload_rejects_traversal() {
    let err = StartBlockUpload::new(mimir(), "../admin")
        .execute(&ctx())
        .await
        .unwrap_err();
    assert_path_traversal_error(err);
}

#[tokio::test]
async fn upload_block_file_rejects_traversal() {
    let err = UploadBlockFile::new(mimir(), "../admin", "idx", vec![])
        .execute(&ctx())
        .await
        .unwrap_err();
    assert_path_traversal_error(err);
}

#[tokio::test]
async fn finish_block_upload_rejects_traversal() {
    let err = FinishBlockUpload::new(mimir(), "../admin")
        .execute(&ctx())
        .await
        .unwrap_err();
    assert_path_traversal_error(err);
}

#[tokio::test]
async fn label_values_rejects_traversal() {
    let err = GetLabelValues::new(mimir(), "../admin")
        .execute(&ctx())
        .await
        .unwrap_err();
    assert_path_traversal_error(err);
}

#[tokio::test]
async fn empty_namespace_rejected() {
    let err = GetRulesByNamespace::new(mimir(), "")
        .execute(&ctx())
        .await
        .unwrap_err();
    assert_path_traversal_error(err);
}
