//! Tests for [`PodRun`](super::PodRun): pure `build_pod` manifest assertions
//! and transport-mocked `run()` phase/cleanup behaviour.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use http::{Method, Request, Response, StatusCode};
use hyper::body::Bytes;
use ironflow_core::error::OperationError;
use ironflow_core::operation::Operation;
use tower::Service;
use tower::service_fn;

use k8s_openapi::api::core::v1::Toleration;

use super::{PodRun, ResourceSpec, SecuritySpec, active_deadline_secs};
use crate::KubeClient;

/// Wrap a canned `tower` service as a [`KubeClient`].
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

/// A dummy client that echoes `{}` -- enough for pure `build_pod` tests.
fn dummy_kube() -> KubeClient {
    kube_from(service_fn(|_r: Request<kube::client::Body>| async {
        Ok::<_, Infallible>(Response::new(kube::client::Body::from(Bytes::from_static(
            b"{}",
        ))))
    }))
}

// -- build_pod (pure) --

#[tokio::test]
async fn build_pod_sets_command_image_and_restart_policy() {
    let pod = PodRun::new(&dummy_kube(), "p1", "busybox", "echo hello").build_pod();
    let spec = pod.spec.unwrap();
    assert_eq!(spec.restart_policy.as_deref(), Some("Never"));
    let c = &spec.containers[0];
    assert_eq!(c.image.as_deref(), Some("busybox"));
    assert_eq!(
        c.command.as_ref().unwrap(),
        &vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "echo hello".to_string()
        ]
    );
}

#[tokio::test]
async fn build_pod_applies_node_selector() {
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true")
        .node_selector("disktype", "ssd")
        .build_pod();
    let ns = pod.spec.unwrap().node_selector.unwrap();
    assert_eq!(ns.get("disktype").map(String::as_str), Some("ssd"));
}

#[tokio::test]
async fn build_pod_applies_toleration() {
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true")
        .toleration(Toleration {
            key: Some("dedicated".to_string()),
            operator: Some("Equal".to_string()),
            value: Some("worker".to_string()),
            effect: Some("NoSchedule".to_string()),
            ..Default::default()
        })
        .build_pod();
    let tolerations = pod.spec.unwrap().tolerations.unwrap();
    assert_eq!(tolerations.len(), 1);
    let t = &tolerations[0];
    assert_eq!(t.key.as_deref(), Some("dedicated"));
    assert_eq!(t.operator.as_deref(), Some("Equal"));
    assert_eq!(t.value.as_deref(), Some("worker"));
    assert_eq!(t.effect.as_deref(), Some("NoSchedule"));
}

#[tokio::test]
async fn build_pod_applies_multiple_tolerations() {
    // Additive: two toleration() calls must both land in the pod spec.
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true")
        .toleration(Toleration {
            key: Some("dedicated".to_string()),
            operator: Some("Equal".to_string()),
            value: Some("worker".to_string()),
            effect: Some("NoSchedule".to_string()),
            ..Default::default()
        })
        .toleration(Toleration {
            key: Some("gpu".to_string()),
            operator: Some("Exists".to_string()),
            effect: Some("NoExecute".to_string()),
            ..Default::default()
        })
        .build_pod();
    let tolerations = pod.spec.unwrap().tolerations.unwrap();
    assert_eq!(tolerations.len(), 2);
    assert_eq!(tolerations[0].key.as_deref(), Some("dedicated"));
    assert_eq!(tolerations[1].key.as_deref(), Some("gpu"));
    assert_eq!(tolerations[1].operator.as_deref(), Some("Exists"));
}

#[tokio::test]
async fn build_pod_without_toleration_has_no_tolerations() {
    // Opt-in strict: the field stays absent unless the builder is called.
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true").build_pod();
    assert!(pod.spec.unwrap().tolerations.is_none());
}

#[tokio::test]
async fn build_pod_applies_image_pull_secret() {
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true")
        .image_pull_secret("regcred")
        .build_pod();
    let secrets = pod.spec.unwrap().image_pull_secrets.unwrap();
    assert_eq!(secrets[0].name, "regcred");
}

#[tokio::test]
async fn build_pod_applies_security_context() {
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true")
        .security(SecuritySpec {
            run_as_user: 1000,
            run_as_group: 3000,
            fs_group: 2000,
        })
        .build_pod();
    let spec = pod.spec.unwrap();
    let csc = spec.containers[0].security_context.as_ref().unwrap();
    assert_eq!(csc.run_as_non_root, Some(true));
    assert_eq!(csc.run_as_user, Some(1000));
    assert_eq!(csc.run_as_group, Some(3000));
    let psc = spec.security_context.as_ref().unwrap();
    assert_eq!(psc.fs_group, Some(2000));
    assert_eq!(psc.run_as_non_root, Some(true));
}

#[tokio::test]
async fn build_pod_sets_automount_service_account_token() {
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true")
        .automount_service_account_token(false)
        .build_pod();
    assert_eq!(
        pod.spec.unwrap().automount_service_account_token,
        Some(false)
    );
}

#[tokio::test]
async fn build_pod_omits_automount_by_default() {
    // Opt-in strict: the field stays absent unless the builder is called.
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true").build_pod();
    assert_eq!(pod.spec.unwrap().automount_service_account_token, None);
}

#[tokio::test]
async fn build_pod_sets_active_deadline_seconds() {
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true")
        .active_deadline_seconds(Duration::from_secs(120))
        .build_pod();
    assert_eq!(pod.spec.unwrap().active_deadline_seconds, Some(120));
}

#[tokio::test]
async fn build_pod_omits_active_deadline_seconds_by_default() {
    // Opt-in strict: no pod deadline unless the builder is called.
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true").build_pod();
    assert_eq!(pod.spec.unwrap().active_deadline_seconds, None);
}

#[test]
fn active_deadline_secs_maps_and_saturates() {
    assert_eq!(active_deadline_secs(None), None);
    assert_eq!(
        active_deadline_secs(Some(Duration::from_secs(120))),
        Some(120)
    );
    // Sub-second truncates to zero, never negative.
    assert_eq!(
        active_deadline_secs(Some(Duration::from_millis(500))),
        Some(0)
    );
    // Beyond i64::MAX seconds saturates instead of wrapping negative.
    assert_eq!(active_deadline_secs(Some(Duration::MAX)), Some(i64::MAX));
}

#[tokio::test]
async fn build_pod_sets_allow_privilege_escalation() {
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true")
        .allow_privilege_escalation(false)
        .build_pod();
    let csc = pod.spec.unwrap().containers[0]
        .security_context
        .clone()
        .unwrap();
    assert_eq!(csc.allow_privilege_escalation, Some(false));
}

#[tokio::test]
async fn build_pod_allow_privilege_escalation_without_security_spec() {
    // The container SecurityContext must be created even when security() was
    // never called, otherwise the toggle would be silently dropped.
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true")
        .allow_privilege_escalation(false)
        .build_pod();
    let csc = pod.spec.unwrap().containers[0]
        .security_context
        .clone()
        .unwrap();
    assert_eq!(csc.allow_privilege_escalation, Some(false));
    assert_eq!(csc.run_as_non_root, None);
    assert_eq!(csc.run_as_user, None);
    assert_eq!(csc.run_as_group, None);
}

#[tokio::test]
async fn build_pod_combines_security_and_allow_privilege_escalation() {
    // Both set: the two must coexist in the same SecurityContext with neither
    // overwriting the other.
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true")
        .security(SecuritySpec {
            run_as_user: 1000,
            run_as_group: 3000,
            fs_group: 2000,
        })
        .allow_privilege_escalation(false)
        .build_pod();
    let csc = pod.spec.unwrap().containers[0]
        .security_context
        .clone()
        .unwrap();
    assert_eq!(csc.allow_privilege_escalation, Some(false));
    assert_eq!(csc.run_as_non_root, Some(true));
    assert_eq!(csc.run_as_user, Some(1000));
    assert_eq!(csc.run_as_group, Some(3000));
}

#[tokio::test]
async fn build_pod_omits_privilege_escalation_context_by_default() {
    // No security() and no allow_privilege_escalation(): the container carries
    // no SecurityContext at all (no regression for existing callers).
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true").build_pod();
    assert!(pod.spec.unwrap().containers[0].security_context.is_none());
}

#[tokio::test]
async fn build_pod_mounts_pvc_and_working_dir() {
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true")
        .pvc("shared-claim", "/workspace")
        .working_dir("/workspace")
        .build_pod();
    let spec = pod.spec.unwrap();
    let c = &spec.containers[0];
    assert_eq!(c.working_dir.as_deref(), Some("/workspace"));
    let mounts = c.volume_mounts.as_ref().unwrap();
    assert_eq!(mounts.len(), 1);
    let mount = &mounts[0];
    assert_eq!(mount.mount_path, "/workspace");
    let volumes = spec.volumes.as_ref().unwrap();
    assert_eq!(volumes.len(), 1);
    let vol = &volumes[0];
    assert_eq!(
        vol.persistent_volume_claim.as_ref().unwrap().claim_name,
        "shared-claim"
    );
    // Retrocompat strict: a single .pvc() keeps the historical "workspace" name.
    assert_eq!(vol.name, "workspace");
    assert_eq!(mount.name, "workspace");
    assert_eq!(vol.name, mount.name);
}

#[tokio::test]
async fn build_pod_without_pvc_has_no_volumes() {
    // Zero .pvc() must leave volumes and volume_mounts absent (0.1.7 manifest).
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true").build_pod();
    let spec = pod.spec.unwrap();
    assert!(spec.volumes.is_none());
    assert!(spec.containers[0].volume_mounts.is_none());
}

#[tokio::test]
async fn build_pod_mounts_multiple_pvcs() {
    // Two .pvc() calls must be additive: 2 volumes + 2 mounts with distinct names.
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true")
        .pvc("workspace-claim", "/workspace")
        .pvc("cache-claim", "/cache")
        .build_pod();
    let spec = pod.spec.unwrap();
    let volumes = spec.volumes.as_ref().unwrap();
    let mounts = spec.containers[0].volume_mounts.as_ref().unwrap();
    assert_eq!(
        volumes.len(),
        2,
        "second .pvc() must not overwrite the first"
    );
    assert_eq!(mounts.len(), 2);
    let names: Vec<&str> = volumes.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["workspace", "workspace-1"]);
    // Volume names must be unique.
    assert_ne!(volumes[0].name, volumes[1].name);
}

#[tokio::test]
async fn build_pod_multiple_pvcs_map_claims() {
    // Each volume maps the right claim, and each mount its matching volume + path.
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true")
        .pvc("workspace-claim", "/workspace")
        .pvc("cache-claim", "/cache")
        .build_pod();
    let spec = pod.spec.unwrap();
    let volumes = spec.volumes.as_ref().unwrap();
    let mounts = spec.containers[0].volume_mounts.as_ref().unwrap();

    assert_eq!(
        volumes[0]
            .persistent_volume_claim
            .as_ref()
            .unwrap()
            .claim_name,
        "workspace-claim"
    );
    assert_eq!(
        volumes[1]
            .persistent_volume_claim
            .as_ref()
            .unwrap()
            .claim_name,
        "cache-claim"
    );

    assert_eq!(mounts[0].name, volumes[0].name);
    assert_eq!(mounts[0].mount_path, "/workspace");
    assert_eq!(mounts[1].name, volumes[1].name);
    assert_eq!(mounts[1].mount_path, "/cache");
}

#[tokio::test]
async fn build_pod_sets_resource_requests_and_limits() {
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true")
        .resources(ResourceSpec {
            cpu_request: Some("100m".to_string()),
            cpu_limit: Some("500m".to_string()),
            memory_request: Some("128Mi".to_string()),
            memory_limit: Some("512Mi".to_string()),
        })
        .build_pod();
    let res = pod.spec.unwrap().containers[0].resources.clone().unwrap();
    assert_eq!(res.requests.as_ref().unwrap()["cpu"].0, "100m");
    assert_eq!(res.limits.as_ref().unwrap()["memory"].0, "512Mi");
}

// -- input() / kind() --

#[tokio::test]
async fn kind_is_k8s() {
    let op = PodRun::new(&dummy_kube(), "p", "busybox", "true");
    assert_eq!(op.kind(), "k8s");
}

#[tokio::test]
async fn input_exposes_metadata_no_secret() {
    let op = PodRun::new(&dummy_kube(), "runner", "busybox", "echo hi").namespace("ci");
    let input = op.input().unwrap();
    assert_eq!(input["image"], "busybox");
    assert_eq!(input["namespace"], "ci");
    assert_eq!(input["name"], "runner");
    assert_eq!(input["command"], "echo hi");
}

// -- run(): phase transitions via a stateful routing service --

/// Build a `PodRun` over `svc` with fast polling and a short timeout.
fn pod_run_with<S>(svc: S) -> PodRun
where
    S: Service<
            Request<kube::client::Body>,
            Response = Response<kube::client::Body>,
            Error = Infallible,
        > + Send
        + 'static,
    S::Future: Send + 'static,
{
    PodRun::new(&kube_from(svc), "job-pod", "busybox", "echo hi")
        .poll_interval(Duration::from_millis(1))
        .timeout(Duration::from_secs(5))
}

/// Route by method + path. `get_phases` are returned in order for each
/// successive GET on the pod (simulating phase transitions); the last value
/// repeats. `deleted` counts DELETE calls.
fn routing_service(
    get_phases: Vec<&'static str>,
    deleted: Arc<AtomicUsize>,
) -> impl Service<
    Request<kube::client::Body>,
    Response = Response<kube::client::Body>,
    Error = Infallible,
    Future = impl Send,
> + Clone {
    let get_idx = Arc::new(AtomicUsize::new(0));
    let phases = Arc::new(get_phases);
    service_fn(move |req: Request<kube::client::Body>| {
        let get_idx = get_idx.clone();
        let phases = phases.clone();
        let deleted = deleted.clone();
        async move {
            let method = req.method().clone();
            let path = req.uri().path().to_string();
            let body: String = if method == Method::DELETE {
                deleted.fetch_add(1, Ordering::SeqCst);
                r#"{"kind":"Status","apiVersion":"v1","status":"Success"}"#.to_string()
            } else if path.ends_with("/log") {
                "line-1\nline-2\n".to_string()
            } else if method == Method::POST {
                r#"{"kind":"Pod","apiVersion":"v1","metadata":{"name":"job-pod","namespace":"default"},"spec":{"containers":[]},"status":{"phase":"Pending"}}"#.to_string()
            } else {
                let i = get_idx.fetch_add(1, Ordering::SeqCst);
                let phase = phases
                    .get(i)
                    .or_else(|| phases.last())
                    .copied()
                    .unwrap_or("Pending");
                format!(
                    r#"{{"kind":"Pod","apiVersion":"v1","metadata":{{"name":"job-pod","namespace":"default"}},"spec":{{"containers":[]}},"status":{{"phase":"{phase}"}}}}"#
                )
            };
            Ok::<_, Infallible>(Response::new(kube::client::Body::from(Bytes::from(body))))
        }
    })
}

#[tokio::test]
async fn run_succeeded_collects_logs_and_deletes() {
    let deleted = Arc::new(AtomicUsize::new(0));
    let run = pod_run_with(routing_service(
        vec!["Pending", "Succeeded"],
        deleted.clone(),
    ));
    let out = run.run().await.unwrap();
    assert!(out.success);
    assert_eq!(out.phase, "Succeeded");
    assert!(out.logs.contains("line-1"));
    assert_eq!(deleted.load(Ordering::SeqCst), 1, "pod must be deleted");
}

#[tokio::test]
async fn run_failed_is_ok_with_success_false() {
    let deleted = Arc::new(AtomicUsize::new(0));
    let run = pod_run_with(routing_service(vec!["Failed"], deleted.clone()));
    let out = run.run().await.unwrap();
    assert!(!out.success, "command failure must not be an error");
    assert_eq!(out.phase, "Failed");
    assert_eq!(deleted.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn run_create_error_is_external() {
    // POST (create) returns 500 -> External error.
    let svc = service_fn(|req: Request<kube::client::Body>| async move {
        if req.method() == Method::POST {
            let mut r = Response::new(kube::client::Body::from(Bytes::from_static(
                br#"{"kind":"Status","status":"Failure","message":"boom","code":500}"#,
            )));
            *r.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            Ok::<_, Infallible>(r)
        } else {
            Ok::<_, Infallible>(Response::new(kube::client::Body::from(Bytes::from_static(
                b"{}",
            ))))
        }
    });
    let err = pod_run_with(svc).run().await.unwrap_err();
    match err {
        OperationError::External { origin, .. } => assert_eq!(origin, "kubernetes"),
        other => panic!("expected External kubernetes error, got: {other}"),
    }
}

#[tokio::test]
async fn run_timeout_is_external_and_deletes() {
    let deleted = Arc::new(AtomicUsize::new(0));
    // Pod never reaches a terminal phase -> wait loop runs until timeout.
    let run = PodRun::new(
        &kube_from(routing_service(vec!["Pending"], deleted.clone())),
        "job-pod",
        "busybox",
        "sleep",
    )
    .poll_interval(Duration::from_millis(1))
    .timeout(Duration::from_millis(20));
    let err = run.run().await.unwrap_err();
    match err {
        OperationError::External { origin, message } => {
            assert_eq!(origin, "kubernetes");
            assert!(message.contains("did not finish"), "got: {message}");
        }
        other => panic!("expected External timeout, got: {other}"),
    }
    assert_eq!(
        deleted.load(Ordering::SeqCst),
        1,
        "pod must be deleted even on timeout"
    );
}
