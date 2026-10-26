//! Issue and commit discussion endpoints.

use std::sync::Arc;

use gitlab::api::common::NameOrId;
use gitlab::api::{AsyncQuery, Pagination};
use gitlab::{AsyncGitlab, GitlabBuilder};
use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_gitlab::GitLab;
use ironflow_ops_gitlab::endpoints::discussions::{
    CreateDiscussion, Discussion, DiscussionObject, Discussions, ReplyToDiscussion,
};
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, method, path, query_param};
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
async fn issue_discussions_are_listed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/issues/7/discussions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id": "d1"}, {"id": "d2"}])))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = Discussions {
        project: NameOrId::from(42),
        object: DiscussionObject::Issue(7),
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn commit_discussions_use_the_commit_route_with_group_project() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v4/projects/group%2Fproject/repository/commits/0a1b2c3/discussions",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id": "d1"}])))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = Discussions {
        project: NameOrId::from("group/project"),
        object: DiscussionObject::Commit("0a1b2c3".to_string()),
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result[0]["id"], "d1");
}

#[tokio::test]
async fn commit_ref_with_slash_is_percent_encoded() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v4/projects/42/repository/commits/feat%2Fx/discussions",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = Discussions {
        project: NameOrId::from(42),
        object: DiscussionObject::Commit("feat/x".to_string()),
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert!(result.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn one_issue_discussion_is_read() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/issues/7/discussions/abcd1234"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "abcd1234"})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = Discussion {
        project: NameOrId::from(42),
        object: DiscussionObject::Issue(7),
        discussion_id: "abcd1234".to_string(),
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result["id"], "abcd1234");
}

#[tokio::test]
async fn one_commit_discussion_is_read() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v4/projects/42/repository/commits/0a1b2c3/discussions/abcd1234",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "abcd1234"})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = Discussion {
        project: NameOrId::from(42),
        object: DiscussionObject::Commit("0a1b2c3".to_string()),
        discussion_id: "abcd1234".to_string(),
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result["id"], "abcd1234");
}

#[tokio::test]
async fn discussion_id_with_special_chars_is_percent_encoded() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/issues/7/discussions/a%2Fb%3Fc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x"})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = Discussion {
        project: NameOrId::from(42),
        object: DiscussionObject::Issue(7),
        discussion_id: "a/b?c".to_string(),
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result["id"], "x");
}

#[tokio::test]
async fn missing_discussion_surfaces_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/issues/7/discussions/nope"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "404 Not found"})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = Discussion {
        project: NameOrId::from(42),
        object: DiscussionObject::Issue(7),
        discussion_id: "nope".to_string(),
    };

    let result: Result<Value, _> = endpoint.query_async(&client).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn discussion_is_created_on_an_issue() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v4/projects/42/issues/7/discussions"))
        .and(body_string_contains("body=Can+you+reproduce"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": "new"})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = CreateDiscussion {
        project: NameOrId::from(42),
        object: DiscussionObject::Issue(7),
        body: "Can you reproduce".to_string(),
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result["id"], "new");
}

#[tokio::test]
async fn discussion_is_created_on_a_commit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/api/v4/projects/42/repository/commits/0a1b2c3/discussions",
        ))
        .and(body_string_contains("body=hello"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": "new"})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = CreateDiscussion {
        project: NameOrId::from(42),
        object: DiscussionObject::Commit("0a1b2c3".to_string()),
        body: "hello".to_string(),
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result["id"], "new");
}

#[tokio::test]
async fn reply_is_posted_to_an_issue_discussion() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/api/v4/projects/42/issues/7/discussions/abcd1234/notes",
        ))
        .and(body_string_contains("body=Fixed"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 9})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = ReplyToDiscussion {
        project: NameOrId::from(42),
        object: DiscussionObject::Issue(7),
        discussion_id: "abcd1234".to_string(),
        body: "Fixed".to_string(),
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result["id"], 9);
}

#[tokio::test]
async fn reply_is_posted_to_a_commit_discussion() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/api/v4/projects/42/repository/commits/0a1b2c3/discussions/abcd1234/notes",
        ))
        .and(body_string_contains("body=Thanks"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 10})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = ReplyToDiscussion {
        project: NameOrId::from(42),
        object: DiscussionObject::Commit("0a1b2c3".to_string()),
        discussion_id: "abcd1234".to_string(),
        body: "Thanks".to_string(),
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result["id"], 10);
}

#[tokio::test]
async fn issue_discussions_are_paged_through_a_paged_operation() {
    let server = MockServer::start().await;
    let page1: Vec<Value> = (0..100).map(|i| json!({"id": format!("d{i}")})).collect();
    let page2: Vec<Value> = vec![json!({"id": "d100"}), json!({"id": "d101"})];
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/issues/7/discussions"))
        .and(query_param("page", "1"))
        .and(query_param("per_page", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&page1))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/issues/7/discussions"))
        .and(query_param("page", "2"))
        .and(query_param("per_page", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&page2))
        .mount(&server)
        .await;

    let gitlab: GitLab = insecure_client(&server).await.into();
    let endpoint = Discussions {
        project: NameOrId::from(42),
        object: DiscussionObject::Issue(7),
    };
    let ctx = OperationContext::new(Arc::new(NoopSecretResolver));

    let result = gitlab
        .paged_op(endpoint, Pagination::All)
        .execute(&ctx)
        .await
        .unwrap();
    let items = result.as_array().unwrap();
    assert_eq!(items.len(), 102);
    assert_eq!(items[101]["id"], "d101");
}
