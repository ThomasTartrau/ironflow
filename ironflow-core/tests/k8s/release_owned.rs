//! `K8sEphemeralProvider::release_owned` deletes, at boot, what a previous
//! process of the same worker pod left running: the objects with an
//! `ownerReference` to the worker pod's uid, and the pods of its Jobs.

use tokio::time::{Duration, timeout};

use ironflow_core::provider::LABEL_RUN_ID;
use ironflow_core::providers::claude::K8sEphemeralProvider;

use crate::fake_api::{FakeK8s, NS, job, owned_by_job, owned_by_worker, pod, prompt_configmap};

const MANAGED: &str = "app.kubernetes.io/managed-by=ironflow";
const PROMPTS: &str =
    "app.kubernetes.io/managed-by=ironflow,app.kubernetes.io/component=prompt-data";
const WORKER: &str = "worker-0";
const UID: &str = "uid-worker-0";
const OTHER: &str = "uid-worker-1";
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

fn provider(fake: &FakeK8s) -> K8sEphemeralProvider {
    K8sEphemeralProvider::new("img:v1")
        .namespace(NS)
        .cluster_config(fake.cluster_config())
        .previous_attempt_timeout(Duration::from_millis(500))
        .owner_pod(WORKER, UID)
}

#[tokio::test]
async fn k8s_release_owned_deletes_what_the_worker_pod_owns_and_nothing_else() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        fake.owner(WORKER, UID).await;
        let run = [(LABEL_RUN_ID, "run-1")];
        let agent = |name: &str| pod(name, "claude-runner", &run, "Running", None, None);
        let job_pod = |name: &str| pod(name, "job-run", &run, "Running", None, None);
        fake.list(
            "pods",
            MANAGED,
            vec![
                owned_by_worker(agent("agent-mine"), UID),
                owned_by_worker(
                    pod("check-mine", "pod-run", &run, "Running", None, None),
                    UID,
                ),
                owned_by_job(job_pod("migrate-x1"), "migrate"),
                owned_by_worker(agent("agent-other"), OTHER),
                agent("agent-unowned"),
                owned_by_job(job_pod("other-job-x1"), "other-job"),
            ],
        )
        .await;
        fake.list(
            "jobs",
            MANAGED,
            vec![
                owned_by_worker(job("migrate", &run, None, None), UID),
                owned_by_worker(job("other-job", &run, None, None), OTHER),
                job("unowned-job", &run, None, None),
            ],
        )
        .await;
        fake.list(
            "configmaps",
            PROMPTS,
            vec![
                owned_by_worker(prompt_configmap("agent-mine-prompt", &run, None), UID),
                owned_by_worker(prompt_configmap("agent-other-prompt", &run, None), OTHER),
                prompt_configmap("agent-unowned-prompt", &run, None),
            ],
        )
        .await;
        for name in ["agent-mine", "check-mine", "migrate-x1"] {
            fake.expect_delete("pods", name, 1).await;
            fake.pod_gone(name).await;
        }
        for name in ["agent-other", "agent-unowned", "other-job-x1"] {
            fake.expect_delete("pods", name, 0).await;
        }
        fake.expect_background_delete("migrate", 1).await;
        for name in ["other-job", "unowned-job"] {
            fake.expect_delete("jobs", name, 0).await;
        }
        fake.expect_delete("configmaps", "agent-mine-prompt", 1)
            .await;
        for name in ["agent-other-prompt", "agent-unowned-prompt"] {
            fake.expect_delete("configmaps", name, 0).await;
        }

        provider(&fake).release_owned().await.expect("released");

        fake.server.verify().await;
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_release_owned_without_owner_pod_fails_without_any_request() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;

        let err = K8sEphemeralProvider::new("img:v1")
            .namespace(NS)
            .cluster_config(fake.cluster_config())
            .release_owned()
            .await
            .expect_err("no owner");

        assert!(
            err.to_string().contains("release_owned needs owner_pod"),
            "{err}"
        );
        let requests = fake.server.received_requests().await.expect("recorded");
        assert!(requests.is_empty(), "{requests:?}");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_release_owned_with_an_owner_outside_the_namespace_deletes_nothing() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        fake.owner_missing(WORKER).await;

        let err = provider(&fake).release_owned().await.expect_err("no owner");

        assert!(err.to_string().contains("not found in namespace"), "{err}");
        let requests = fake.server.received_requests().await.expect("recorded");
        assert!(
            requests.iter().all(|r| r.method.as_str() == "GET"),
            "{requests:?}"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_release_owned_fails_while_an_owned_pod_is_still_there() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        fake.owner(WORKER, UID).await;
        let stuck = owned_by_worker(
            pod(
                "agent-mine",
                "claude-runner",
                &[(LABEL_RUN_ID, "run-1")],
                "Running",
                None,
                None,
            ),
            UID,
        );
        fake.list("pods", MANAGED, vec![stuck.clone()]).await;
        fake.list_nothing("jobs", MANAGED).await;
        fake.list_nothing("configmaps", PROMPTS).await;
        fake.expect_delete("pods", "agent-mine", 1).await;
        fake.pod_stays(stuck).await;

        let err = provider(&fake)
            .release_owned()
            .await
            .expect_err("pod stays");

        let message = err.to_string();
        assert!(message.contains("still terminating"), "{message}");
        assert!(message.contains(WORKER), "{message}");
    })
    .await
    .expect("test timed out");
}
