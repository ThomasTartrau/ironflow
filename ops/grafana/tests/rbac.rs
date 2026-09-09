use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::rbac::{
    RbacCreateRole, RbacDeleteRole, RbacGetRole, RbacGetRoleAssignments, RbacGetRoles,
    RbacUpdateRole,
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
async fn rbac_get_roles() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/access-control/roles"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"uid": "r1", "name": "Custom Viewer", "version": 1, "global": false}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = RbacGetRoles::new(&client);
    assert_eq!(op.kind(), "grafana");
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
    assert_eq!(result[0]["name"], "Custom Viewer");
}

#[tokio::test]
async fn rbac_get_roles_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/access-control/roles"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"uid": "r2", "name": "DS Editor", "description": "Edit datasources", "version": 2, "global": true}
        ])))
        .mount(&server)
        .await;

    let op = RbacGetRoles::new(&client);
    let roles = op.run().await.unwrap();
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].global, Some(true));
}

#[tokio::test]
async fn rbac_get_role() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/access-control/roles/r1"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "uid": "r1", "name": "Custom Viewer", "version": 1
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = RbacGetRole::new(&client, "r1");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["uid"], "r1");
}

#[tokio::test]
async fn rbac_get_role_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/access-control/roles/typed-role"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "uid": "typed-role", "name": "Typed Role", "description": "test", "version": 3, "global": false
        })))
        .mount(&server)
        .await;

    let op = RbacGetRole::new(&client, "typed-role");
    let output = op.run().await.unwrap();
    assert_eq!(output.name.as_deref(), Some("Typed Role"));
    assert_eq!(output.version, Some(3));
    assert_eq!(output.global, Some(false));
}

#[tokio::test]
async fn rbac_create_role() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/access-control/roles"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "uid": "new-role", "name": "Dashboard Creator", "version": 1
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = RbacCreateRole::new(&client, json!({"name": "Dashboard Creator"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["uid"], "new-role");
}

#[tokio::test]
async fn rbac_create_role_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/access-control/roles"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "uid": "cr-role", "name": "New Custom", "version": 1, "global": true
        })))
        .mount(&server)
        .await;

    let op = RbacCreateRole::new(&client, json!({"name": "New Custom"}));
    let output = op.run().await.unwrap();
    assert_eq!(output.uid.as_deref(), Some("cr-role"));
    assert_eq!(output.global, Some(true));
}

#[tokio::test]
async fn rbac_update_role() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/access-control/roles/r1"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "uid": "r1", "name": "Updated", "version": 2
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = RbacUpdateRole::new(&client, "r1", json!({"name": "Updated"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["version"], 2);
}

#[tokio::test]
async fn rbac_delete_role() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/access-control/roles/r1"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "deleted"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = RbacDeleteRole::new(&client, "r1");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "deleted");
}

#[tokio::test]
async fn rbac_get_role_assignments() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/access-control/roles/r1/assignments"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"roleUid": "r1", "scope": "dashboards:*"}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = RbacGetRoleAssignments::new(&client, "r1");
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
    assert_eq!(result[0]["scope"], "dashboards:*");
}

#[tokio::test]
async fn rbac_get_role_assignments_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/access-control/roles/r2/assignments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"roleUid": "r2", "scope": "datasources:uid:ds1"}
        ])))
        .mount(&server)
        .await;

    let op = RbacGetRoleAssignments::new(&client, "r2");
    let assignments = op.run().await.unwrap();
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0].scope.as_deref(), Some("datasources:uid:ds1"));
}
