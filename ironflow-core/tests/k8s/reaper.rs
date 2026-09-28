//! The orphan reaper covers every ironflow pod, whatever its component, and
//! the Jobs of `JobRun`.

use tokio::time::{Duration, timeout};

use ironflow_core::provider::LABEL_RUN_ID;
use ironflow_core::providers::claude::K8sEphemeralProvider;
use ironflow_core::providers::claude::k8s::reap_orphans;

use crate::fake_api::{FakeK8s, NS, job, now, owned_by_job, pod, prompt_configmap};

const MANAGED: &str = "app.kubernetes.io/managed-by=ironflow";
const PROMPTS: &str =
    "app.kubernetes.io/managed-by=ironflow,app.kubernetes.io/component=prompt-data";
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::test]
async fn k8s_reaper_selects_ironflow_pods_of_any_component() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        let run = [(LABEL_RUN_ID, "run-1")];
        let pods = vec![pod(
            "check-1",
            "pod-run",
            &run,
            "Running",
            None,
            Some(now() - 30),
        )];
        fake.list("pods", MANAGED, pods).await;
        fake.list("jobs", MANAGED, vec![]).await;
        fake.list("configmaps", PROMPTS, vec![]).await;
        fake.expect_delete("pods", "check-1", 1).await;

        let provider = K8sEphemeralProvider::new("img:v1")
            .namespace(NS)
            .cluster_config(fake.cluster_config());
        let report = provider.reap_orphans().await.expect("reaping pass");

        assert_eq!(report.pods_deleted, 1);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_reaper_deletes_orphans_and_keeps_live_objects() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        let (past, future) = (Some(now() - 30), Some(now() + 3600));
        let pods = vec![
            pod("check-expired", "pod-run", &[], "Running", None, past),
            pod(
                "agent-killed",
                "claude-runner",
                &[],
                "Failed",
                Some("DeadlineExceeded"),
                future,
            ),
            pod("agent-live", "claude-runner", &[], "Running", None, future),
            pod(
                "agent-unannotated",
                "claude-runner",
                &[],
                "Running",
                None,
                None,
            ),
            owned_by_job(
                pod(
                    "migrate-abcde",
                    "job-run",
                    &[],
                    "Failed",
                    Some("DeadlineExceeded"),
                    past,
                ),
                "migrate",
            ),
        ];
        let jobs = vec![
            job("job-expired", &[], None, past),
            job("job-killed", &[], Some("DeadlineExceeded"), future),
            job("job-live", &[], None, future),
            job("job-unannotated", &[], None, None),
        ];
        let configmaps = vec![
            prompt_configmap("prompt-expired", &[], past),
            prompt_configmap("prompt-live", &[], future),
        ];
        fake.list("pods", MANAGED, pods).await;
        fake.list("jobs", MANAGED, jobs).await;
        fake.list("configmaps", PROMPTS, configmaps).await;
        for name in ["check-expired", "agent-killed"] {
            fake.expect_delete("pods", name, 1).await;
        }
        for name in ["agent-live", "agent-unannotated", "migrate-abcde"] {
            fake.expect_delete("pods", name, 0).await;
        }
        for name in ["job-expired", "job-killed"] {
            fake.expect_background_delete(name, 1).await;
        }
        for name in ["job-live", "job-unannotated"] {
            fake.expect_delete("jobs", name, 0).await;
        }
        fake.expect_delete("configmaps", "prompt-expired", 1).await;
        fake.expect_delete("configmaps", "prompt-live", 0).await;

        let report = reap_orphans(&fake.cluster_config(), NS)
            .await
            .expect("reaping pass");

        assert_eq!(report.pods_deleted, 2);
        assert_eq!(report.jobs_deleted, 2);
        assert_eq!(report.configmaps_deleted, 1);
        fake.server.verify().await;
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_reaper_without_job_rights_still_reaps_pods() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        let pods = vec![pod(
            "check-1",
            "pod-run",
            &[],
            "Succeeded",
            None,
            Some(now() - 30),
        )];
        fake.list("pods", MANAGED, pods).await;
        fake.fail_list("jobs", MANAGED, 403).await;
        fake.list("configmaps", PROMPTS, vec![]).await;
        fake.expect_delete("pods", "check-1", 1).await;

        let report = reap_orphans(&fake.cluster_config(), NS)
            .await
            .expect("a 403 on Jobs does not fail the pass");

        assert_eq!(report.pods_deleted, 1);
        assert_eq!(report.jobs_deleted, 0);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_reaper_fails_when_pods_cannot_be_listed() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        fake.list("jobs", MANAGED, vec![]).await;
        fake.fail_list("pods", MANAGED, 500).await;

        let err = reap_orphans(&fake.cluster_config(), NS)
            .await
            .expect_err("a failed pod listing fails the pass");

        let message = err.to_string();
        assert!(
            message.contains("failed to list ironflow pods"),
            "{message}"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_reaper_fails_on_a_job_listing_error_other_than_forbidden() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        fake.fail_list("jobs", MANAGED, 500).await;

        let err = reap_orphans(&fake.cluster_config(), NS)
            .await
            .expect_err("only a 403 on Jobs is tolerated");

        assert!(
            err.to_string().contains("failed to list ironflow Jobs"),
            "{err}"
        );
    })
    .await
    .expect("test timed out");
}
