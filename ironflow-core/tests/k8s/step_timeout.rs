//! `AgentConfig::timeout` on the K8s ephemeral provider: a step past its
//! timeout fails with a timeout error and its pod is deleted, and a step
//! timeout longer than the provider one pushes the pod expiry back.
//!
//! The real `kube` client talks to the fake API server (see [`fake_api`]).

use std::sync::Arc;

use serde_json::json;
use tokio::time::{Duration, timeout};
use wiremock::matchers::{method, path, path_regex, query_param};
use wiremock::{Mock, ResponseTemplate};

use ironflow_core::error::{AgentError, OperationError};
use ironflow_core::operations::agent::Agent;
use ironflow_core::provider::{AgentConfig, AgentProvider, LABEL_EXPIRES_AT, LogSink};
use ironflow_core::providers::claude::K8sEphemeralProvider;

use crate::fake_api::{FakeK8s, NS, collection, count, now, pod, wait_for};

const TEST_TIMEOUT: Duration = Duration::from_secs(30);

fn provider(fake: &FakeK8s) -> K8sEphemeralProvider {
    K8sEphemeralProvider::new("img:v1")
        .namespace(NS)
        .cluster_config(fake.cluster_config())
}

/// A log sink that drops every line.
struct Discard;

impl LogSink for Discard {
    fn log(&self, _stream: &str, _line: &str) {}
}

/// Accept the creation of any pod, answer it as running, stream its logs
/// with nothing until the test is over (the agent is still working), and
/// accept its deletion.
async fn pods_running_forever(fake: &FakeK8s) {
    let created = pod("created", "agent", &[], "Pending", None, None);
    Mock::given(method("POST"))
        .and(path(collection("pods")))
        .respond_with(ResponseTemplate::new(201).set_body_json(created))
        .mount(&fake.server)
        .await;
    Mock::given(method("GET"))
        .and(path(collection("pods")))
        .and(query_param("watch", "true"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
        .mount(&fake.server)
        .await;
    Mock::given(method("GET"))
        .and(path(collection("pods")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "apiVersion": "v1",
            "kind": "List",
            "metadata": { "resourceVersion": "2" },
            "items": [pod("agent-pod", "agent", &[], "Running", None, None)],
        })))
        .mount(&fake.server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(format!("^{}/[^/]+$", collection("pods"))))
        .respond_with(ResponseTemplate::new(200).set_body_json(pod(
            "agent-pod",
            "agent",
            &[],
            "Running",
            None,
            None,
        )))
        .mount(&fake.server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(format!("^{}/[^/]+/log$", collection("pods"))))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(60)))
        .mount(&fake.server)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex(format!("^{}/[^/]+$", collection("pods"))))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "apiVersion": "v1", "kind": "Status", "status": "Success", "metadata": {}
        })))
        .mount(&fake.server)
        .await;
}

/// Run a step with a 500 ms timeout against a pod that never finishes, with
/// or without a log sink, and check it fails with that timeout and its pod is
/// deleted.
async fn assert_step_timeout_deletes_the_pod(streaming: bool) {
    let fake = FakeK8s::start().await;
    pods_running_forever(&fake).await;
    let provider = provider(&fake);
    let step_timeout = Duration::from_millis(500);
    let mut agent = Agent::from_config(AgentConfig::new("triage").timeout(step_timeout));
    if streaming {
        agent = agent.log_sink(Arc::new(Discard));
    }

    let result = agent.run(&provider).await;

    match result {
        Err(OperationError::Agent(AgentError::Timeout { limit })) => {
            assert_eq!(limit, step_timeout);
        }
        other => panic!("expected an agent timeout, got {other:?}"),
    }
    let posted = fake.posted("pods").await;
    let name = posted[0]["metadata"]["name"]
        .as_str()
        .expect("pod name")
        .to_string();
    let pod_path = format!("{}/{name}", collection("pods"));
    wait_for(&fake.server, "the pod deletion", |r| {
        count(r, "DELETE", &pod_path) > 0
    })
    .await;
}

/// The worker path: it always streams the logs of an agent step.
#[tokio::test]
async fn k8s_step_past_its_timeout_fails_with_a_timeout_and_deletes_its_pod() {
    timeout(TEST_TIMEOUT, assert_step_timeout_deletes_the_pod(true))
        .await
        .expect("test timed out");
}

/// The library path: without a log sink, `Agent::run` calls `invoke`, which
/// watches the pod until it completes instead of streaming its logs.
#[tokio::test]
async fn k8s_step_past_its_timeout_without_log_sink_deletes_its_pod() {
    timeout(TEST_TIMEOUT, assert_step_timeout_deletes_the_pod(false))
        .await
        .expect("test timed out");
}

#[tokio::test]
async fn k8s_step_timeout_longer_than_the_provider_one_sets_the_pod_expiry() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        let step_timeout = Duration::from_secs(7200);
        let before = now();

        // The fake server refuses the pod: the invocation stops right after
        // the POST under test.
        let err = provider(&fake)
            .timeout(Duration::from_secs(60))
            .invoke(&AgentConfig::new("implement").timeout(step_timeout))
            .await
            .expect_err("pod refused");
        assert!(
            err.to_string().contains("failed to create K8s pod"),
            "{err}"
        );

        let pods = fake.posted("pods").await;
        assert_eq!(pods.len(), 1);
        let expires_at: u64 = pods[0]["metadata"]["annotations"][LABEL_EXPIRES_AT]
            .as_str()
            .expect("expires-at annotation")
            .parse()
            .expect("expires-at in seconds");
        assert!(
            expires_at >= before + step_timeout.as_secs(),
            "pod expires at {expires_at}, before the step timeout"
        );
    })
    .await
    .expect("test timed out");
}
