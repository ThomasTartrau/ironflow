//! Single note read and release listing endpoints.

use gitlab::api::AsyncQuery;
use gitlab::api::common::NameOrId;
use gitlab::{AsyncGitlab, GitlabBuilder};
use ironflow_ops_gitlab::endpoints::notes::{Note, Noteable};
use ironflow_ops_gitlab::endpoints::releases::{ReleaseOrderBy, ReleaseSort, Releases};
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
async fn issue_note_is_read_by_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/issues/7/notes/1001"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 1001, "body": "hi"})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = Note {
        project: NameOrId::from(42),
        noteable: Noteable::Issue(7),
        note: 1001,
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result["body"], "hi");
}

#[tokio::test]
async fn merge_request_note_is_read_by_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v4/projects/group%2Fproject/merge_requests/3/notes/55",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 55})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = Note {
        project: NameOrId::from("group/project"),
        noteable: Noteable::MergeRequest(3),
        note: 55,
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result["id"], 55);
}

#[tokio::test]
async fn missing_note_surfaces_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/issues/7/notes/9"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "404 Not found"})))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = Note {
        project: NameOrId::from(42),
        noteable: Noteable::Issue(7),
        note: 9,
    };

    let result: Result<Value, _> = endpoint.query_async(&client).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn releases_are_listed_with_sort_and_page_size() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/releases"))
        .and(query_param("order_by", "released_at"))
        .and(query_param("sort", "desc"))
        .and(query_param("per_page", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"tag_name": "v2"}])))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = Releases {
        project: NameOrId::from(42),
        order_by: Some(ReleaseOrderBy::ReleasedAt),
        sort: Some(ReleaseSort::Desc),
        per_page: Some(5),
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert_eq!(result[0]["tag_name"], "v2");
}

#[tokio::test]
async fn releases_can_be_ordered_by_creation_ascending() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/releases"))
        .and(query_param("order_by", "created_at"))
        .and(query_param("sort", "asc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = Releases {
        project: NameOrId::from(42),
        order_by: Some(ReleaseOrderBy::CreatedAt),
        sort: Some(ReleaseSort::Asc),
        per_page: None,
    };

    let result: Value = endpoint.query_async(&client).await.unwrap();
    assert!(result.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn releases_without_options_send_no_query_string() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/group%2Fproject/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    let client = insecure_client(&server).await;
    let endpoint = Releases {
        project: NameOrId::from("group/project"),
        order_by: None,
        sort: None,
        per_page: None,
    };

    let _: Value = endpoint.query_async(&client).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let request = requests
        .iter()
        .find(|r| r.url.path().ends_with("/releases"))
        .unwrap();
    assert_eq!(request.url.query_pairs().count(), 0);
}
