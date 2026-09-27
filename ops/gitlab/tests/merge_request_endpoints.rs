use gitlab::api::AsyncQuery;
use gitlab::api::common::NameOrId;
use gitlab::{AsyncGitlab, GitlabBuilder};
use ironflow_ops_gitlab::endpoints::merge_requests::{
    CreateMergeRequestDiscussionNote, ResolveMergeRequestDiscussion,
};
use serde_json::{Value, json};
use wiremock::matchers::{body_string, method, path};
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
async fn create_discussion_note_numeric_project_sends_post_and_form_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/api/v4/projects/42/merge_requests/7/discussions/abcd1234/notes",
        ))
        .and(body_string("body=looks+good"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 1})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = CreateMergeRequestDiscussionNote {
        project: NameOrId::from(42),
        merge_request: 7,
        discussion_id: "abcd1234".to_string(),
        body: "looks good".to_string(),
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result["id"], 1);
}

#[tokio::test]
async fn create_discussion_note_escapes_group_slash_project_path() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/api/v4/projects/group%2Fproject/merge_requests/7/discussions/abcd1234/notes",
        ))
        .and(body_string("body=looks+good"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 1})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = CreateMergeRequestDiscussionNote {
        project: NameOrId::from("group/project"),
        merge_request: 7,
        discussion_id: "abcd1234".to_string(),
        body: "looks good".to_string(),
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result["id"], 1);
}

#[tokio::test]
async fn create_discussion_note_404_is_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/api/v4/projects/42/merge_requests/7/discussions/abcd1234/notes",
        ))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "not found"})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = CreateMergeRequestDiscussionNote {
        project: NameOrId::from(42),
        merge_request: 7,
        discussion_id: "abcd1234".to_string(),
        body: "looks good".to_string(),
    };

    let result: Result<Value, _> = endpoint.query_async(&client).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn resolve_discussion_sends_put_and_resolved_true() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(
            "/api/v4/projects/42/merge_requests/7/discussions/abcd1234",
        ))
        .and(body_string("resolved=true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "abcd1234"})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = ResolveMergeRequestDiscussion {
        project: NameOrId::from(42),
        merge_request: 7,
        discussion_id: "abcd1234".to_string(),
        resolved: true,
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result["id"], "abcd1234");
}

#[tokio::test]
async fn resolve_discussion_sends_resolved_false() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(
            "/api/v4/projects/42/merge_requests/7/discussions/abcd1234",
        ))
        .and(body_string("resolved=false"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "abcd1234"})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = ResolveMergeRequestDiscussion {
        project: NameOrId::from(42),
        merge_request: 7,
        discussion_id: "abcd1234".to_string(),
        resolved: false,
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result["id"], "abcd1234");
}

#[tokio::test]
async fn resolve_discussion_404_is_error() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(
            "/api/v4/projects/42/merge_requests/7/discussions/abcd1234",
        ))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "not found"})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = ResolveMergeRequestDiscussion {
        project: NameOrId::from(42),
        merge_request: 7,
        discussion_id: "abcd1234".to_string(),
        resolved: true,
    };

    let result: Result<Value, _> = endpoint.query_async(&client).await;
    assert!(result.is_err());
}
