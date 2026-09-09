use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::alerting::{
    ContactPointCreate, ContactPointDelete, ContactPointList, ContactPointUpdate, MuteTimingCreate,
    MuteTimingDelete, MuteTimingList, MuteTimingUpdate, NotificationPolicyGet,
    NotificationPolicyUpdate,
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
async fn contact_point_list() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/provisioning/contact-points"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"uid": "cp1", "name": "Slack", "type": "slack"}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = ContactPointList::new(&client);
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
    assert_eq!(result[0]["name"], "Slack");
}

#[tokio::test]
async fn contact_point_list_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/provisioning/contact-points"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"uid": "cp2", "name": "Email", "type": "email", "settings": {"to": "a@b.com"}}
        ])))
        .mount(&server)
        .await;

    let op = ContactPointList::new(&client);
    let pts = op.run().await.unwrap();
    assert_eq!(pts.len(), 1);
    assert_eq!(pts[0].name.as_deref(), Some("Email"));
}

#[tokio::test]
async fn contact_point_create() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/provisioning/contact-points"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({
            "uid": "new-cp", "name": "PD", "type": "pagerduty"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = ContactPointCreate::new(&client, json!({"name": "PD", "type": "pagerduty"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["uid"], "new-cp");
}

#[tokio::test]
async fn contact_point_update() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/provisioning/contact-points/cp1"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let op = ContactPointUpdate::new(&client, "cp1", json!({"name": "Slack v2"}));
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_object());
}

#[tokio::test]
async fn contact_point_delete() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/provisioning/contact-points/cp1"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "deleted"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = ContactPointDelete::new(&client, "cp1");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "deleted");
}

#[tokio::test]
async fn notification_policy_get() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/provisioning/policies"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "receiver": "default", "groupBy": ["alertname"]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = NotificationPolicyGet::new(&client);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["receiver"], "default");
}

#[tokio::test]
async fn notification_policy_get_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/provisioning/policies"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "receiver": "slack", "groupBy": ["team"], "routes": []
        })))
        .mount(&server)
        .await;

    let op = NotificationPolicyGet::new(&client);
    let output = op.run().await.unwrap();
    assert_eq!(output.receiver.as_deref(), Some("slack"));
}

#[tokio::test]
async fn notification_policy_update() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/provisioning/policies"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let op = NotificationPolicyUpdate::new(&client, json!({"receiver": "email"}));
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_object());
}

#[tokio::test]
async fn mute_timing_list() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/provisioning/mute-timings"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"name": "weekends", "timeIntervals": []}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = MuteTimingList::new(&client);
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
}

#[tokio::test]
async fn mute_timing_list_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/provisioning/mute-timings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"name": "typed-mt", "timeIntervals": [{"days_of_week": ["monday"]}]}
        ])))
        .mount(&server)
        .await;

    let op = MuteTimingList::new(&client);
    let timings = op.run().await.unwrap();
    assert_eq!(timings.len(), 1);
    assert_eq!(timings[0].name.as_deref(), Some("typed-mt"));
}

#[tokio::test]
async fn mute_timing_create() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/provisioning/mute-timings"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "name": "holidays", "timeIntervals": []
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = MuteTimingCreate::new(&client, json!({"name": "holidays"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["name"], "holidays");
}

#[tokio::test]
async fn mute_timing_update() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/provisioning/mute-timings/weekends"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "weekends", "timeIntervals": [{"days_of_week": ["saturday", "sunday"]}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = MuteTimingUpdate::new(&client, "weekends", json!({"timeIntervals": []}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["name"], "weekends");
}

#[tokio::test]
async fn mute_timing_delete() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/provisioning/mute-timings/weekends"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "deleted"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = MuteTimingDelete::new(&client, "weekends");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "deleted");
}
