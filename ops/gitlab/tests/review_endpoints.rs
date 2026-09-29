//! Endpoints a merge request review needs that the `gitlab` crate lacks:
//! diff versions, merge base, note deletion.

use std::sync::Arc;

use gitlab::api::AsyncQuery;
use gitlab::api::common::NameOrId;
use gitlab::{AsyncGitlab, GitlabBuilder};
use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_gitlab::GitLab;
use ironflow_ops_gitlab::endpoints::merge_requests::{
    DeleteMergeRequestNote, MergeRequestVersion, MergeRequestVersions,
};
use ironflow_ops_gitlab::endpoints::repository::MergeBase;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn insecure_client(server: &MockServer) -> AsyncGitlab {
    Mock::given(method("GET"))
        .and(path("/api/v4/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 1})))
        .mount(server)
        .await;

    GitlabBuilder::new(server.address().to_string(), "token")
        .insecure()
        .build_async()
        .await
        .unwrap()
}

#[tokio::test]
async fn versions_lists_diff_versions_of_the_merge_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/group%2Fproject/merge_requests/7/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 2, "head_commit_sha": "bbb", "base_commit_sha": "base", "start_commit_sha": "start"},
            {"id": 1, "head_commit_sha": "aaa", "base_commit_sha": "base", "start_commit_sha": "start"}
        ])))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = MergeRequestVersions {
        project: NameOrId::from("group/project"),
        merge_request: 7,
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result[0]["head_commit_sha"], "bbb");
    assert_eq!(result.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn version_reads_one_diff_version_with_its_diffs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/merge_requests/7/versions/2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 2,
            "head_commit_sha": "bbb",
            "diffs": [{"old_path": "a.rs", "new_path": "a.rs", "diff": "@@ -1 +1 @@\n-a\n+b\n"}]
        })))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = MergeRequestVersion {
        project: NameOrId::from(42),
        merge_request: 7,
        version: 2,
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result["diffs"][0]["new_path"], "a.rs");
}

#[tokio::test]
async fn versions_404_is_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/merge_requests/7/versions"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "404 Not found"})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = MergeRequestVersions {
        project: NameOrId::from(42),
        merge_request: 7,
    };

    let result: Result<Value, _> = endpoint.query_async(&client).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn merge_base_sends_every_ref_as_refs_array() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/repository/merge_base"))
        .and(query_param("refs[]", "aaa"))
        .and(query_param("refs[]", "bbb"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "aaa"})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = MergeBase {
        project: NameOrId::from(42),
        refs: vec!["aaa".to_string(), "bbb".to_string()],
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result["id"], "aaa");
}

#[tokio::test]
async fn merge_base_of_unrelated_refs_is_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/repository/merge_base"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"message": "Could not find merge base"})),
        )
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = MergeBase {
        project: NameOrId::from(42),
        refs: vec!["aaa".to_string(), "zzz".to_string()],
    };

    let result: Result<Value, _> = endpoint.query_async(&client).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn delete_note_through_an_operation_accepts_204_no_content() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v4/projects/42/merge_requests/7/notes/99"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let gitlab: GitLab = insecure_client(&server).await.into();
    let endpoint = DeleteMergeRequestNote {
        project: NameOrId::from(42),
        merge_request: 7,
        note: 99,
    };
    let ctx = OperationContext::new(Arc::new(NoopSecretResolver));

    let result = gitlab.op(endpoint).execute(&ctx).await.unwrap();
    assert_eq!(result, Value::Null);
}

#[tokio::test]
async fn delete_missing_note_is_error() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v4/projects/42/merge_requests/7/notes/99"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "404 Not found"})))
        .mount(&server)
        .await;

    let gitlab: GitLab = insecure_client(&server).await.into();
    let endpoint = DeleteMergeRequestNote {
        project: NameOrId::from(42),
        merge_request: 7,
        note: 99,
    };
    let ctx = OperationContext::new(Arc::new(NoopSecretResolver));

    assert!(gitlab.op(endpoint).execute(&ctx).await.is_err());
}
