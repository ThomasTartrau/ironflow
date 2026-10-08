//! Tests for [`JobRun`](super::JobRun): pure `build_job` manifest assertions
//! and transport-mocked `run()` completion/cleanup behaviour.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use http::{Method, Request, Response};
use hyper::body::Bytes;
use ironflow_core::operation::Operation;
use k8s_openapi::api::core::v1::{
    EmptyDirVolumeSource, PersistentVolumeClaimVolumeSource, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use tower::Service;
use tower::service_fn;

use super::JobRun;
use crate::KubeClient;

fn kube_from<S>(svc: S) -> KubeClient
where
    S: Service<
            Request<kube::client::Body>,
            Response = Response<kube::client::Body>,
            Error = Infallible,
        > + Send
        + 'static,
    S::Future: Send + 'static,
{
    KubeClient::from_raw(kube::Client::new(svc, "default"))
}

fn dummy_kube() -> KubeClient {
    kube_from(service_fn(|_r: Request<kube::client::Body>| async {
        Ok::<_, Infallible>(Response::new(kube::client::Body::from(Bytes::from_static(
            b"{}",
        ))))
    }))
}

// -- build_job (pure) --

#[tokio::test]
async fn build_job_sets_template_backoff_and_restart_policy() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .backoff_limit(3)
        .build_job();
    let spec = job.spec.unwrap();
    assert_eq!(spec.backoff_limit, Some(3));
    let pod_spec = spec.template.spec.unwrap();
    assert_eq!(pod_spec.restart_policy.as_deref(), Some("Never"));
    let c = &pod_spec.containers[0];
    assert_eq!(c.image.as_deref(), Some("migrate:1"));
    assert_eq!(
        c.command.as_ref().unwrap(),
        &vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "migrate up".to_string()
        ]
    );
}

#[tokio::test]
async fn build_job_without_pvc_has_no_volumes() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up").build_job();
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    assert!(pod_spec.volumes.is_none());
    assert!(pod_spec.containers[0].volume_mounts.is_none());
}

#[tokio::test]
async fn build_job_mounts_single_pvc() {
    // A single .pvc() keeps the "workspace" volume name, mirroring PodRun.
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .pvc("workspace-claim", "/workspace")
        .build_job();
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    let volumes = pod_spec.volumes.as_ref().unwrap();
    let mounts = pod_spec.containers[0].volume_mounts.as_ref().unwrap();
    assert_eq!(volumes.len(), 1);
    assert_eq!(mounts.len(), 1);
    assert_eq!(volumes[0].name, "workspace");
    assert_eq!(mounts[0].name, "workspace");
    assert_eq!(mounts[0].mount_path, "/workspace");
    assert_eq!(
        volumes[0]
            .persistent_volume_claim
            .as_ref()
            .unwrap()
            .claim_name,
        "workspace-claim"
    );
}

#[tokio::test]
async fn build_job_mounts_multiple_pvcs() {
    // Two .pvc() calls are additive: 2 volumes + 2 mounts, distinct names.
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .pvc("workspace-claim", "/workspace")
        .pvc("cache-claim", "/cache")
        .build_job();
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    let volumes = pod_spec.volumes.as_ref().unwrap();
    let mounts = pod_spec.containers[0].volume_mounts.as_ref().unwrap();
    assert_eq!(
        volumes.len(),
        2,
        "second .pvc() must not overwrite the first"
    );
    assert_eq!(mounts.len(), 2);
    let names: Vec<&str> = volumes.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["workspace", "workspace-1"]);
    assert_eq!(
        volumes[1]
            .persistent_volume_claim
            .as_ref()
            .unwrap()
            .claim_name,
        "cache-claim"
    );
    assert_eq!(mounts[1].name, volumes[1].name);
    assert_eq!(mounts[1].mount_path, "/cache");
}

#[tokio::test]
async fn build_job_applies_arbitrary_volume_with_sub_path() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .pvc("shared-claim", "/workspace")
        .volume(
            Volume {
                name: "cache-subpath".to_string(),
                persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                    claim_name: "shared-claim".to_string(),
                    read_only: None,
                }),
                ..Default::default()
            },
            VolumeMount {
                name: "cache-subpath".to_string(),
                mount_path: "/cache".to_string(),
                sub_path: Some("cache-dir".to_string()),
                ..Default::default()
            },
        )
        .build_job();
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    let volumes = pod_spec.volumes.as_ref().unwrap();
    assert_eq!(volumes.len(), 2);
    assert_eq!(volumes[0].name, "workspace");
    assert_eq!(volumes[1].name, "cache-subpath");
    let mounts = pod_spec.containers[0].volume_mounts.as_ref().unwrap();
    assert_eq!(mounts[1].sub_path.as_deref(), Some("cache-dir"));
    assert_eq!(
        volumes[0]
            .persistent_volume_claim
            .as_ref()
            .unwrap()
            .claim_name,
        "shared-claim"
    );
    assert_eq!(
        volumes[1]
            .persistent_volume_claim
            .as_ref()
            .unwrap()
            .claim_name,
        "shared-claim"
    );
}

#[tokio::test]
async fn build_job_applies_arbitrary_empty_dir_volume_with_size_limit() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .volume(
            Volume {
                name: "scratch".to_string(),
                empty_dir: Some(EmptyDirVolumeSource {
                    size_limit: Some(Quantity("1Gi".to_string())),
                    ..Default::default()
                }),
                ..Default::default()
            },
            VolumeMount {
                name: "scratch".to_string(),
                mount_path: "/scratch".to_string(),
                ..Default::default()
            },
        )
        .build_job();
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    let volumes = pod_spec.volumes.as_ref().unwrap();
    assert_eq!(volumes.len(), 1);
    assert_eq!(
        volumes[0]
            .empty_dir
            .as_ref()
            .unwrap()
            .size_limit
            .as_ref()
            .unwrap()
            .0,
        "1Gi"
    );
    let mounts = pod_spec.containers[0].volume_mounts.as_ref().unwrap();
    assert_eq!(mounts[0].read_only, None);
}

#[tokio::test]
async fn build_job_arbitrary_volume_readonly() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .volume(
            Volume {
                name: "scratch".to_string(),
                empty_dir: Some(EmptyDirVolumeSource::default()),
                ..Default::default()
            },
            VolumeMount {
                name: "scratch".to_string(),
                mount_path: "/scratch".to_string(),
                read_only: Some(true),
                ..Default::default()
            },
        )
        .build_job();
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    let mounts = pod_spec.containers[0].volume_mounts.as_ref().unwrap();
    assert_eq!(mounts[0].read_only, Some(true));
}

#[tokio::test]
async fn build_job_arbitrary_volumes_after_pvcs_in_call_order() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .pvc("claim-a", "/a")
        .volume(
            Volume {
                name: "v1".to_string(),
                ..Default::default()
            },
            VolumeMount {
                name: "v1".to_string(),
                mount_path: "/v1".to_string(),
                ..Default::default()
            },
        )
        .pvc("claim-b", "/b")
        .volume(
            Volume {
                name: "v2".to_string(),
                ..Default::default()
            },
            VolumeMount {
                name: "v2".to_string(),
                mount_path: "/v2".to_string(),
                ..Default::default()
            },
        )
        .build_job();
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    let volumes = pod_spec.volumes.as_ref().unwrap();
    let names: Vec<&str> = volumes.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["workspace", "workspace-1", "v1", "v2"]);
}

#[tokio::test]
#[should_panic(expected = "must match")]
async fn build_job_volume_panics_on_name_mismatch() {
    let _ = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up").volume(
        Volume {
            name: "a".to_string(),
            ..Default::default()
        },
        VolumeMount {
            name: "b".to_string(),
            mount_path: "/x".to_string(),
            ..Default::default()
        },
    );
}

#[tokio::test]
#[should_panic(expected = "reserved for .pvc()")]
async fn build_job_volume_panics_on_reserved_name_workspace() {
    let _ = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up").volume(
        Volume {
            name: "workspace".to_string(),
            ..Default::default()
        },
        VolumeMount {
            name: "workspace".to_string(),
            mount_path: "/x".to_string(),
            ..Default::default()
        },
    );
}

#[tokio::test]
#[should_panic(expected = "reserved for .pvc()")]
async fn build_job_volume_panics_on_reserved_name_workspace_prefix() {
    let _ = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up").volume(
        Volume {
            name: "workspace-custom".to_string(),
            ..Default::default()
        },
        VolumeMount {
            name: "workspace-custom".to_string(),
            mount_path: "/x".to_string(),
            ..Default::default()
        },
    );
}

#[tokio::test]
#[should_panic(expected = "already used")]
async fn build_job_volume_panics_on_duplicate_name() {
    let _ = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .volume(
            Volume {
                name: "cache".to_string(),
                ..Default::default()
            },
            VolumeMount {
                name: "cache".to_string(),
                mount_path: "/x".to_string(),
                ..Default::default()
            },
        )
        .volume(
            Volume {
                name: "cache".to_string(),
                ..Default::default()
            },
            VolumeMount {
                name: "cache".to_string(),
                mount_path: "/y".to_string(),
                ..Default::default()
            },
        );
}

#[tokio::test]
async fn build_job_without_volume_has_no_extra_volumes() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .pvc("claim", "/w")
        .build_job();
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    assert_eq!(pod_spec.volumes.as_ref().unwrap().len(), 1);
}

#[tokio::test]
async fn build_job_applies_env_var() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .env("RUST_LOG", "debug")
        .build_job();
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    let env = pod_spec.containers[0].env.clone().unwrap();
    assert_eq!(env.len(), 1);
    assert_eq!(env[0].name, "RUST_LOG");
    assert_eq!(env[0].value, Some("debug".to_string()));
    assert!(env[0].value_from.is_none());
}

#[tokio::test]
async fn build_job_env_replaces_duplicate_name() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .env("RUST_LOG", "debug")
        .env("RUST_LOG", "trace")
        .build_job();
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    let env = pod_spec.containers[0].env.clone().unwrap();
    assert_eq!(env.len(), 1);
    assert_eq!(env[0].name, "RUST_LOG");
    assert_eq!(env[0].value, Some("trace".to_string()));
}

#[tokio::test]
async fn build_job_multiple_envs() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .env("A", "1")
        .env("B", "2")
        .build_job();
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    let env = pod_spec.containers[0].env.clone().unwrap();
    assert_eq!(env.len(), 2);
    assert_eq!(env[0].name, "A");
    assert_eq!(env[0].value, Some("1".to_string()));
    assert_eq!(env[1].name, "B");
    assert_eq!(env[1].value, Some("2".to_string()));
}

#[tokio::test]
async fn build_job_without_env_has_no_env() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up").build_job();
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    assert!(pod_spec.containers[0].env.is_none());
}

#[tokio::test]
async fn kind_and_input() {
    let op = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up").backoff_limit(2);
    assert_eq!(op.kind(), "k8s");
    let input = op.input().unwrap();
    assert_eq!(input["name"], "migrate");
    assert_eq!(input["backoff_limit"], 2);
}

#[tokio::test]
async fn build_job_sets_automount_service_account_token() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .automount_service_account_token(false)
        .build_job();
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    assert_eq!(pod_spec.automount_service_account_token, Some(false));
}

#[tokio::test]
async fn build_job_sets_allow_privilege_escalation() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .allow_privilege_escalation(false)
        .build_job();
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    let csc = pod_spec.containers[0].security_context.clone().unwrap();
    assert_eq!(csc.allow_privilege_escalation, Some(false));
}

#[tokio::test]
async fn build_job_omits_hardening_by_default() {
    // Opt-in strict: no builder called -> both fields absent, no regression.
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up").build_job();
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    assert_eq!(pod_spec.automount_service_account_token, None);
    assert!(pod_spec.containers[0].security_context.is_none());
}

#[tokio::test]
async fn build_job_sets_active_deadline_seconds_on_jobspec_and_template() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .active_deadline_seconds(Duration::from_secs(300))
        .build_job();
    let job_spec = job.spec.unwrap();
    // JobSpec deadline bounds the whole Job (retries included)...
    assert_eq!(job_spec.active_deadline_seconds, Some(300));
    // ...and the template deadline caps each individual pod attempt.
    assert_eq!(
        job_spec.template.spec.unwrap().active_deadline_seconds,
        Some(300)
    );
}

#[tokio::test]
async fn build_job_deadline_bounds_total_with_retries() {
    // With backoff_limit > 0, only the JobSpec deadline bounds the total
    // wall-clock; the per-pod template deadline alone would not.
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .backoff_limit(2)
        .active_deadline_seconds(Duration::from_secs(300))
        .build_job();
    let job_spec = job.spec.unwrap();
    assert_eq!(job_spec.backoff_limit, Some(2));
    assert_eq!(job_spec.active_deadline_seconds, Some(300));
}

#[tokio::test]
async fn build_job_omits_active_deadline_seconds_by_default() {
    // Opt-in strict: no deadline on JobSpec or template unless the builder is called.
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up").build_job();
    let job_spec = job.spec.unwrap();
    assert_eq!(job_spec.active_deadline_seconds, None);
    assert_eq!(
        job_spec.template.spec.unwrap().active_deadline_seconds,
        None
    );
}

// -- run(): condition transitions via a stateful routing service --

/// Route by method + path. `conditions` are consumed in order for each GET on
/// the Job (`None` = still running). `deleted` counts DELETE calls.
fn routing_service(
    conditions: Vec<Option<&'static str>>,
    deleted: Arc<AtomicUsize>,
) -> impl Service<
    Request<kube::client::Body>,
    Response = Response<kube::client::Body>,
    Error = Infallible,
    Future = impl Send,
> + Clone {
    routing_service_with_logs(conditions, deleted, "migration-output\n".to_string())
}

/// Same as [`routing_service`], the pod log endpoint returning `logs`.
fn routing_service_with_logs(
    conditions: Vec<Option<&'static str>>,
    deleted: Arc<AtomicUsize>,
    logs: String,
) -> impl Service<
    Request<kube::client::Body>,
    Response = Response<kube::client::Body>,
    Error = Infallible,
    Future = impl Send,
> + Clone {
    let get_idx = Arc::new(AtomicUsize::new(0));
    let conditions = Arc::new(conditions);
    let logs = Arc::new(logs);
    service_fn(move |req: Request<kube::client::Body>| {
        let get_idx = get_idx.clone();
        let conditions = conditions.clone();
        let logs = logs.clone();
        let deleted = deleted.clone();
        async move {
            let method = req.method().clone();
            let path = req.uri().path().to_string();
            let body: String = if method == Method::POST {
                r#"{"kind":"Job","apiVersion":"batch/v1","metadata":{"name":"migrate","namespace":"default"},"spec":{"template":{}},"status":{}}"#.to_string()
            } else if path.ends_with("/log") {
                logs.to_string()
            } else if method == Method::DELETE {
                deleted.fetch_add(1, Ordering::SeqCst);
                r#"{"kind":"Status","apiVersion":"v1","status":"Success"}"#.to_string()
            } else if path.ends_with("/pods") {
                r#"{"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"1"},"items":[{"metadata":{"name":"migrate-abc","namespace":"default"},"spec":{"containers":[]},"status":{}}]}"#.to_string()
            } else if path.contains("/jobs/") {
                let i = get_idx.fetch_add(1, Ordering::SeqCst);
                let cond = conditions
                    .get(i)
                    .or_else(|| conditions.last())
                    .copied()
                    .flatten();
                let status = match cond {
                    Some(t) => format!(r#"{{"conditions":[{{"type":"{t}","status":"True"}}]}}"#),
                    None => "{}".to_string(),
                };
                format!(
                    r#"{{"kind":"Job","apiVersion":"batch/v1","metadata":{{"name":"migrate","namespace":"default"}},"spec":{{"template":{{}}}},"status":{status}}}"#
                )
            } else {
                "{}".to_string()
            };
            Ok::<_, Infallible>(Response::new(kube::client::Body::from(Bytes::from(body))))
        }
    })
}

fn job_run_with<S>(svc: S) -> JobRun
where
    S: Service<
            Request<kube::client::Body>,
            Response = Response<kube::client::Body>,
            Error = Infallible,
        > + Send
        + 'static,
    S::Future: Send + 'static,
{
    JobRun::new(&kube_from(svc), "migrate", "migrate:1", "migrate up")
        .poll_interval(Duration::from_millis(1))
        .timeout(Duration::from_secs(5))
}

#[tokio::test]
async fn run_complete_is_success_with_logs() {
    let deleted = Arc::new(AtomicUsize::new(0));
    let run = job_run_with(routing_service(
        vec![None, Some("Complete")],
        deleted.clone(),
    ));
    let out = run.run().await.unwrap();
    assert!(out.success);
    assert_eq!(out.phase, "Succeeded");
    assert!(out.logs.contains("migration-output"));
    assert!(!out.logs_truncated, "short logs must be kept whole");
    assert_eq!(deleted.load(Ordering::SeqCst), 1, "job must be deleted");
}

#[tokio::test]
async fn run_keeps_only_the_tail_of_logs_over_the_default_limit() {
    let logs = format!("{}migration-tail\n", "x".repeat(2 * 1024 * 1024));
    let run = job_run_with(routing_service_with_logs(
        vec![Some("Complete")],
        Arc::new(AtomicUsize::new(0)),
        logs,
    ));
    let out = run.run().await.unwrap();
    assert!(out.success);
    assert!(out.logs_truncated);
    assert!(
        out.logs.starts_with("[... 1048591 bytes truncated ...]\n"),
        "got: {}",
        &out.logs[..60]
    );
    assert!(out.logs.ends_with("migration-tail\n"));
    let marker_len = "[... 1048591 bytes truncated ...]\n".len();
    assert_eq!(out.logs.len(), marker_len + 1024 * 1024);
}

#[tokio::test]
async fn run_honours_max_log_bytes() {
    let run = job_run_with(routing_service_with_logs(
        vec![Some("Failed")],
        Arc::new(AtomicUsize::new(0)),
        "0123456789".to_string(),
    ))
    .max_log_bytes(4);
    let out = run.run().await.unwrap();
    assert!(!out.success);
    assert!(out.logs_truncated);
    assert_eq!(out.logs, "[... 6 bytes truncated ...]\n6789");
}

#[tokio::test]
async fn run_failed_is_ok_with_success_false() {
    let deleted = Arc::new(AtomicUsize::new(0));
    let run = job_run_with(routing_service(vec![Some("Failed")], deleted.clone()));
    let out = run.run().await.unwrap();
    assert!(!out.success, "job failure must not be an error");
    assert_eq!(out.phase, "Failed");
    assert_eq!(deleted.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn build_job_pvc_volume_renders_sub_path_and_read_only() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
        .pvc_volume("shared-claim", "/repos", Some("team-a"), true)
        .pvc_volume("shared-claim", "/rw", None, false)
        .build_job();
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    assert_eq!(pod_spec.volumes.as_ref().unwrap().len(), 1);
    let mounts = pod_spec.containers[0].volume_mounts.as_ref().unwrap();
    assert_eq!(mounts.len(), 2);
    assert_eq!(mounts[0].sub_path.as_deref(), Some("team-a"));
    assert_eq!(mounts[0].read_only, Some(true));
    assert!(mounts[1].read_only.is_none());
}
