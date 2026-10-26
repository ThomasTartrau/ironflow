use std::sync::Arc;
use std::time::Duration;

use gitlab::api::Pagination;
use gitlab::api::projects::Project;
use gitlab::api::projects::merge_requests::{CreateMergeRequest, MergeRequests};
use gitlab::{AsyncGitlab, GitlabBuilder};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_gitlab::GitLab;
use serde_json::json;
use wiremock::matchers::{method, path};
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

fn context() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn create_merge_request_conflict_carries_status_409() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v4/projects/42/merge_requests"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({
                "message": ["Another open merge request already exists for this source branch: !7"]
            })))
            .mount(&server)
            .await;

        let gitlab = GitLab::from(insecure_client(&server).await);
        let endpoint = CreateMergeRequest::builder()
            .project(42)
            .source_branch("feat")
            .target_branch("main")
            .title("t")
            .build()
            .unwrap();

        let err = gitlab.op(endpoint).execute(&context()).await.unwrap_err();
        assert!(
            matches!(
                err,
                OperationError::Http {
                    status: Some(409),
                    ..
                }
            ),
            "{err:?}"
        );
        assert!(err.to_string().contains("already exists"), "{err}");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn not_found_carries_status_404() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/42"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(json!({"message": "404 Project Not Found"})),
            )
            .mount(&server)
            .await;

        let gitlab = GitLab::from(insecure_client(&server).await);
        let endpoint = Project::builder().project(42).build().unwrap();

        let err = gitlab.op(endpoint).execute(&context()).await.unwrap_err();
        assert!(
            matches!(
                err,
                OperationError::Http {
                    status: Some(404),
                    ..
                }
            ),
            "{err:?}"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn bad_gateway_with_plain_text_body_carries_status_502() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/42"))
            .respond_with(ResponseTemplate::new(502).set_body_string("Bad Gateway"))
            .mount(&server)
            .await;

        let gitlab = GitLab::from(insecure_client(&server).await);
        let endpoint = Project::builder().project(42).build().unwrap();

        let err = gitlab.op(endpoint).execute(&context()).await.unwrap_err();
        assert!(
            matches!(
                err,
                OperationError::Http {
                    status: Some(502),
                    ..
                }
            ),
            "{err:?}"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn paged_op_error_carries_status_403() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/42/merge_requests"))
            .respond_with(
                ResponseTemplate::new(403).set_body_json(json!({"message": "403 Forbidden"})),
            )
            .mount(&server)
            .await;

        let gitlab = GitLab::from(insecure_client(&server).await);
        let endpoint = MergeRequests::builder().project(42).build().unwrap();

        let err = gitlab
            .paged_op(endpoint, Pagination::All)
            .execute(&context())
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                OperationError::Http {
                    status: Some(403),
                    ..
                }
            ),
            "{err:?}"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn transport_failure_has_no_status() {
    tokio::time::timeout(Duration::from_secs(10), async {
        // A non-pooled server is shut down on drop; a pooled one would keep
        // listening on the same port and answer 404.
        let server = MockServer::builder().start().await;
        let gitlab = GitLab::from(insecure_client(&server).await);
        drop(server);
        let endpoint = Project::builder().project(42).build().unwrap();

        let err = gitlab.op(endpoint).execute(&context()).await.unwrap_err();
        assert!(
            matches!(err, OperationError::Http { status: None, .. }),
            "{err:?}"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn success_with_unparsable_body_has_no_status() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/42"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let gitlab = GitLab::from(insecure_client(&server).await);
        let endpoint = Project::builder().project(42).build().unwrap();

        let err = gitlab.op(endpoint).execute(&context()).await.unwrap_err();
        assert!(
            matches!(err, OperationError::Http { status: None, .. }),
            "{err:?}"
        );
    })
    .await
    .expect("test timed out");
}
