use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::teams::{
    TeamAddMember, TeamCreate, TeamDelete, TeamGet, TeamGetMembers, TeamList, TeamRemoveMember,
    TeamUpdate,
};
use serde_json::json;
use wiremock::matchers::{bearer_token, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn setup() -> (MockServer, GrafanaClient, OperationContext) {
    let server = MockServer::start().await;
    let client = GrafanaClient::new("test-token", &server.uri()).unwrap();
    let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    (server, client, ctx)
}

#[tokio::test]
async fn team_list() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/teams/search"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "teams": [{"id": 1, "name": "Backend", "memberCount": 5}],
            "totalCount": 1
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = TeamList::new(&client);
    assert_eq!(op.kind(), "grafana");
    let result = op.execute(&ctx).await.unwrap();
    assert!(result["teams"].is_array());
}

#[tokio::test]
async fn team_list_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/teams/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "teams": [{"id": 2, "name": "Frontend", "email": "fe@ex.com"}],
            "totalCount": 1
        })))
        .mount(&server)
        .await;

    let op = TeamList::new(&client);
    let output = op.run().await.unwrap();
    assert_eq!(output.total_count, Some(1));
    let teams = output.teams.unwrap();
    assert_eq!(teams[0].name.as_deref(), Some("Frontend"));
}

#[tokio::test]
async fn team_get() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/teams/10"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 10, "name": "Ops", "memberCount": 3
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = TeamGet::new(&client, 10);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["id"], 10);
}

#[tokio::test]
async fn team_create() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/teams"))
        .and(bearer_token("test-token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"teamId": 5, "message": "created"})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let op = TeamCreate::new(&client, json!({"name": "SRE", "email": "sre@ex.com"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["teamId"], 5);
}

#[tokio::test]
async fn team_update() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/teams/5"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "updated"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = TeamUpdate::new(&client, 5, json!({"name": "SRE v2"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "updated");
}

#[tokio::test]
async fn team_delete() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/teams/5"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "deleted"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = TeamDelete::new(&client, 5);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "deleted");
}

#[tokio::test]
async fn team_get_members() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/teams/10/members"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"userId": 1, "login": "admin", "email": "admin@ex.com"}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = TeamGetMembers::new(&client, 10);
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
    assert_eq!(result[0]["login"], "admin");
}

#[tokio::test]
async fn team_get_members_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/teams/10/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"userId": 2, "login": "dev", "email": "dev@ex.com"}
        ])))
        .mount(&server)
        .await;

    let op = TeamGetMembers::new(&client, 10);
    let members = op.run().await.unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].login.as_deref(), Some("dev"));
}

#[tokio::test]
async fn team_add_member() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/teams/10/members"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "added"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = TeamAddMember::new(&client, 10, json!({"userId": 5}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "added");
}

#[tokio::test]
async fn team_remove_member() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/teams/10/members/5"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "removed"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = TeamRemoveMember::new(&client, 10, 5);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "removed");
}
