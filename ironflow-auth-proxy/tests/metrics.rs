//! The `/metrics` endpoint counts `/r/` requests by secret name and result.
//!
//! This test lives alone in its binary on purpose: the metrics recorder is
//! process-global and installs once per process.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::serve;
use metrics_exporter_prometheus::PrometheusBuilder;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::spawn;
use tokio::time::timeout;
use url::Url;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

use ironflow_auth_proxy::{AuthProxyConfig, AuthProxyState, REQUESTS_TOTAL, router};
use ironflow_core::auth_proxy::AuthProxyRegistry;

const ADMIN_KEY: &str = "0123456789abcdef0123456789abcdef";
const SECRET: &str = "ghp_test";

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[tokio::test]
async fn metrics_count_relay_results_by_secret() {
    let handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("metrics recorder installed once");

    timeout(Duration::from_secs(10), async {
        let upstream = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&upstream)
            .await;
        let upstream = Url::parse(&upstream.uri()).unwrap();
        let config = AuthProxyConfig::new(ADMIN_KEY).with_secret_upstream(upstream);
        let state = AuthProxyState::with_registry(config, AuthProxyRegistry::default())
            .unwrap()
            .with_metrics(handle);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = router(state);
        spawn(async move {
            serve(listener, app).await.unwrap();
        });
        let http = Client::new();

        let issued = http
            .post(format!("{base}/admin/v1/tokens"))
            .bearer_auth(ADMIN_KEY)
            .json(&json!({
                "run_id": "run-1",
                "step": "review",
                "expires_at": now() + 600,
                "credential": {
                    "name": "GITHUB_TOKEN",
                    "value": SECRET,
                    "injection": "bearer",
                    "hosts": ["api.github.com"],
                }
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(issued.status(), StatusCode::CREATED);
        let body: Value = issued.json().await.unwrap();
        let token = body["token"].as_str().unwrap().to_string();

        let relayed = http
            .get(format!("{base}/r/api.github.com/user"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(relayed.status(), StatusCode::OK);
        let refused = http
            .get(format!("{base}/r/gitlab.com/user"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);

        let resp = http.get(format!("{base}/metrics")).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let text = resp.text().await.unwrap();
        assert!(text.contains(&format!("{REQUESTS_TOTAL}{{")), "{text}");
        assert!(text.contains(r#"secret="GITHUB_TOKEN""#), "{text}");
        assert!(text.contains(r#"result="relayed""#), "{text}");
        assert!(text.contains(r#"result="forbidden_host""#), "{text}");
        assert!(!text.contains(SECRET), "{text}");
        assert!(!text.contains(&token), "{text}");
    })
    .await
    .expect("test timed out");
}
