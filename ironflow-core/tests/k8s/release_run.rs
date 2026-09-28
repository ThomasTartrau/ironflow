//! `K8sEphemeralProvider::release_run` deletes every pod, Job and prompt
//! ConfigMap of a run, its sub-workflows' included, and waits until the pods
//! are gone; the per-step cleanup of a retried step still runs.

use tokio::time::{Duration, timeout};

use ironflow_core::provider::{
    AgentConfig, AgentProvider, LABEL_ROOT_RUN_ID, LABEL_RUN_ID, LABEL_STEP,
};
use ironflow_core::providers::claude::K8sEphemeralProvider;

use crate::fake_api::{FakeK8s, NS, job, owned_by_job, pod, prompt_configmap};

const RUN: &str = "01a0e906-1f85-7000-a285-b190beb3afd8";
const CHILD: &str = "01a0e906-2222-7000-a285-b190beb3afd8";
const BY_RUN: &str =
    "app.kubernetes.io/managed-by=ironflow,ironflow.io/run-id=01a0e906-1f85-7000-a285-b190beb3afd8";
const BY_ROOT: &str = "app.kubernetes.io/managed-by=ironflow,ironflow.io/root-run-id=01a0e906-1f85-7000-a285-b190beb3afd8";
const PROMPTS_BY_RUN: &str = "app.kubernetes.io/managed-by=ironflow,app.kubernetes.io/component=prompt-data,ironflow.io/run-id=01a0e906-1f85-7000-a285-b190beb3afd8";
const PROMPTS_BY_ROOT: &str = "app.kubernetes.io/managed-by=ironflow,app.kubernetes.io/component=prompt-data,ironflow.io/root-run-id=01a0e906-1f85-7000-a285-b190beb3afd8";
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

fn provider(fake: &FakeK8s) -> K8sEphemeralProvider {
    K8sEphemeralProvider::new("img:v1")
        .namespace(NS)
        .cluster_config(fake.cluster_config())
        .previous_attempt_timeout(Duration::from_millis(500))
}

/// Answer the listings of a run with nothing, except the `(kind, selector)`
/// pairs the test answers itself.
async fn empty_run(fake: &FakeK8s, skip: &[(&str, &str)]) {
    for listing in [
        ("pods", BY_RUN),
        ("pods", BY_ROOT),
        ("jobs", BY_RUN),
        ("jobs", BY_ROOT),
        ("configmaps", PROMPTS_BY_RUN),
        ("configmaps", PROMPTS_BY_ROOT),
    ] {
        if !skip.contains(&listing) {
            fake.list_nothing(listing.0, listing.1).await;
        }
    }
}

#[tokio::test]
async fn k8s_release_run_deletes_the_pods_of_the_run_and_its_children() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        let top = [
            (LABEL_RUN_ID, RUN),
            (LABEL_ROOT_RUN_ID, RUN),
            (LABEL_STEP, "plan"),
        ];
        let child = [
            (LABEL_RUN_ID, CHILD),
            (LABEL_ROOT_RUN_ID, RUN),
            (LABEL_STEP, "implement"),
        ];
        let agent = pod("agent-top", "claude-runner", &top, "Running", None, None);
        let check = pod(
            "check-1",
            "pod-run",
            &[(LABEL_RUN_ID, RUN)],
            "Running",
            None,
            None,
        );
        let child_agent = pod(
            "agent-child",
            "claude-runner",
            &child,
            "Running",
            None,
            None,
        );
        fake.list("pods", BY_RUN, vec![agent.clone(), check]).await;
        fake.list("pods", BY_ROOT, vec![agent, child_agent]).await;
        let prompt_top = prompt_configmap("agent-top-prompt", &top, None);
        let prompt_child = prompt_configmap("agent-child-prompt", &child, None);
        fake.list("configmaps", PROMPTS_BY_RUN, vec![prompt_top.clone()])
            .await;
        fake.list(
            "configmaps",
            PROMPTS_BY_ROOT,
            vec![prompt_top, prompt_child],
        )
        .await;
        let answered = [
            ("pods", BY_RUN),
            ("pods", BY_ROOT),
            ("configmaps", PROMPTS_BY_RUN),
            ("configmaps", PROMPTS_BY_ROOT),
        ];
        empty_run(&fake, &answered).await;
        for name in ["agent-top", "check-1", "agent-child"] {
            fake.expect_delete("pods", name, 1).await;
            fake.pod_gone(name).await;
        }
        for name in ["agent-top-prompt", "agent-child-prompt"] {
            fake.expect_delete("configmaps", name, 1).await;
        }

        provider(&fake).release_run(RUN).await.expect("released");

        fake.server.verify().await;
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_release_run_deletes_jobs_before_their_pods() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        let labels = [(LABEL_RUN_ID, RUN)];
        fake.list("jobs", BY_RUN, vec![job("migrate", &labels, None, None)])
            .await;
        let job_pod = pod("migrate-x1", "job-run", &labels, "Running", None, None);
        fake.list("pods", BY_RUN, vec![owned_by_job(job_pod, "migrate")])
            .await;
        empty_run(&fake, &[("jobs", BY_RUN), ("pods", BY_RUN)]).await;
        fake.expect_background_delete("migrate", 1).await;
        fake.expect_delete("pods", "migrate-x1", 1).await;
        fake.pod_gone("migrate-x1").await;

        provider(&fake).release_run(RUN).await.expect("released");

        fake.server.verify().await;
        let requests = fake.server.received_requests().await.expect("recorded");
        let deletes: Vec<String> = requests
            .iter()
            .filter(|r| r.method.as_str() == "DELETE")
            .map(|r| r.url.path().to_string())
            .collect();
        let job_delete = deletes.iter().position(|p| p.ends_with("/jobs/migrate"));
        let pod_delete = deletes.iter().position(|p| p.ends_with("/pods/migrate-x1"));
        assert!(job_delete < pod_delete, "{deletes:?}");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_release_run_without_job_rights_still_releases_pods() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        fake.fail_list("jobs", BY_RUN, 403).await;
        fake.fail_list("jobs", BY_ROOT, 403).await;
        let check = pod(
            "check-1",
            "pod-run",
            &[(LABEL_RUN_ID, RUN)],
            "Running",
            None,
            None,
        );
        fake.list("pods", BY_RUN, vec![check]).await;
        empty_run(
            &fake,
            &[("jobs", BY_RUN), ("jobs", BY_ROOT), ("pods", BY_RUN)],
        )
        .await;
        fake.expect_delete("pods", "check-1", 1).await;
        fake.pod_gone("check-1").await;

        provider(&fake).release_run(RUN).await.expect("released");

        fake.server.verify().await;
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_release_run_fails_while_a_pod_is_still_there() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        let stuck = pod(
            "agent-top",
            "claude-runner",
            &[(LABEL_RUN_ID, RUN)],
            "Running",
            None,
            None,
        );
        fake.list("pods", BY_RUN, vec![stuck.clone()]).await;
        empty_run(&fake, &[("pods", BY_RUN)]).await;
        fake.expect_delete("pods", "agent-top", 1).await;
        fake.pod_stays(stuck).await;

        let err = provider(&fake)
            .release_run(RUN)
            .await
            .expect_err("pod stays");

        let message = err.to_string();
        assert!(message.contains("still terminating"), "{message}");
        assert!(message.contains(RUN), "{message}");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_release_run_fails_when_pods_cannot_be_listed() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        fake.fail_list("pods", BY_RUN, 500).await;
        empty_run(&fake, &[("pods", BY_RUN)]).await;

        let err = provider(&fake)
            .release_run(RUN)
            .await
            .expect_err("list fails");

        assert!(err.to_string().contains("failed to list pods"), "{err}");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_step_retry_still_deletes_the_previous_attempt() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        let step_selector = format!(
            "app.kubernetes.io/managed-by=ironflow,app.kubernetes.io/component=claude-runner,\
             ironflow.io/run-id={RUN},ironflow.io/step=investigate"
        );
        let prompt_selector = format!(
            "app.kubernetes.io/managed-by=ironflow,app.kubernetes.io/component=prompt-data,\
             ironflow.io/run-id={RUN},ironflow.io/step=investigate"
        );
        let labels = [(LABEL_RUN_ID, RUN), (LABEL_STEP, "investigate")];
        let previous = pod("agent-old", "claude-runner", &labels, "Running", None, None);
        fake.list("pods", &step_selector, vec![previous]).await;
        fake.list("configmaps", &prompt_selector, vec![]).await;
        fake.expect_delete("pods", "agent-old", 1).await;
        fake.pod_gone("agent-old").await;

        // The fake server refuses the new pod: the invocation stops right
        // after the cleanup it is meant to prove.
        let config = AgentConfig::new("hi").run_scope(RUN, "investigate");
        let err = provider(&fake)
            .invoke(&config)
            .await
            .expect_err("create refused");

        assert!(
            err.to_string().contains("failed to create K8s pod"),
            "{err}"
        );
        fake.server.verify().await;
    })
    .await
    .expect("test timed out");
}
