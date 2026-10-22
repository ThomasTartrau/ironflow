//! `K8sEphemeralProvider::owner_pod`: the agent pod and its prompt ConfigMap
//! carry an `ownerReference` to the worker pod, so the Kubernetes garbage
//! collector deletes them when the worker pod goes away.

use serde_json::{Value, json};
use tokio::time::{Duration, timeout};

use ironflow_core::provider::{AgentConfig, AgentProvider};
use ironflow_core::providers::claude::K8sEphemeralProvider;
use ironflow_core::providers::claude::common::PROMPT_STDIN_THRESHOLD;

use crate::fake_api::{FakeK8s, NS};

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

fn provider(fake: &FakeK8s) -> K8sEphemeralProvider {
    K8sEphemeralProvider::new("img:v1")
        .namespace(NS)
        .cluster_config(fake.cluster_config())
}

/// A prompt over the stdin threshold: it goes through a prompt ConfigMap.
fn large_prompt() -> AgentConfig {
    AgentConfig::new(&"x".repeat(PROMPT_STDIN_THRESHOLD + 1))
}

fn worker_reference() -> Value {
    json!([{
        "apiVersion": "v1",
        "kind": "Pod",
        "name": "w",
        "uid": "uid-1",
        "controller": false,
        "blockOwnerDeletion": false,
    }])
}

#[tokio::test]
async fn k8s_owner_pod_puts_the_owner_reference_on_the_pod_and_its_prompt_configmap() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        fake.owner("w", "uid-1").await;
        fake.accept_configmaps().await;

        // The fake server refuses the pod: the invocation stops right after
        // the POST under test.
        let err = provider(&fake)
            .owner_pod("w", "uid-1")
            .invoke(&large_prompt())
            .await
            .expect_err("pod refused");
        assert!(
            err.to_string().contains("failed to create K8s pod"),
            "{err}"
        );

        let pods = fake.posted("pods").await;
        assert_eq!(pods.len(), 1);
        assert_eq!(pods[0]["metadata"]["ownerReferences"], worker_reference());
        let configmaps = fake.posted("configmaps").await;
        assert_eq!(configmaps.len(), 1);
        assert_eq!(
            configmaps[0]["metadata"]["ownerReferences"],
            worker_reference()
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_without_owner_pod_no_owner_reference_and_no_owner_lookup() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        fake.accept_configmaps().await;

        let err = provider(&fake)
            .invoke(&large_prompt())
            .await
            .expect_err("pod refused");
        assert!(
            err.to_string().contains("failed to create K8s pod"),
            "{err}"
        );

        let pods = fake.posted("pods").await;
        assert_eq!(pods.len(), 1);
        assert!(pods[0]["metadata"].get("ownerReferences").is_none());
        let configmaps = fake.posted("configmaps").await;
        assert!(configmaps[0]["metadata"].get("ownerReferences").is_none());
        let requests = fake.server.received_requests().await.expect("recorded");
        assert!(
            requests.iter().all(|r| r.method.as_str() != "GET"),
            "no owner lookup without owner_pod"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_owner_pod_with_an_empty_name_or_uid_fails_without_any_request() {
    timeout(TEST_TIMEOUT, async {
        for (name, uid) in [("", "uid-1"), ("w", ""), ("  ", "uid-1"), ("w", " ")] {
            let fake = FakeK8s::start().await;

            let err = provider(&fake)
                .owner_pod(name, uid)
                .invoke(&AgentConfig::new("hi"))
                .await
                .expect_err("empty owner refused");

            let message = err.to_string();
            assert!(message.contains("owner_pod"), "{message}");
            let requests = fake.server.received_requests().await.expect("recorded");
            assert!(requests.is_empty(), "({name:?}, {uid:?}): {requests:?}");
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_owner_pod_absent_from_the_namespace_fails_before_any_creation() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        fake.owner_missing("w").await;
        fake.accept_configmaps().await;

        let err = provider(&fake)
            .owner_pod("w", "uid-1")
            .invoke(&large_prompt())
            .await
            .expect_err("owner missing");

        let message = err.to_string();
        assert!(message.contains("owner pod 'w'"), "{message}");
        assert!(message.contains(&format!("namespace '{NS}'")), "{message}");
        assert!(fake.posted("pods").await.is_empty());
        assert!(fake.posted("configmaps").await.is_empty());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_owner_pod_with_another_uid_fails_before_any_creation() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        fake.owner("w", "uid-2").await;
        fake.accept_configmaps().await;

        let err = provider(&fake)
            .owner_pod("w", "uid-1")
            .invoke(&large_prompt())
            .await
            .expect_err("owner replaced");

        let message = err.to_string();
        assert!(message.contains("uid-2"), "{message}");
        assert!(message.contains("uid-1"), "{message}");
        assert!(fake.posted("pods").await.is_empty());
        assert!(fake.posted("configmaps").await.is_empty());
    })
    .await
    .expect("test timed out");
}
