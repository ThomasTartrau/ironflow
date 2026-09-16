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

use super::{PodRun, ResourceSpec, SecuritySpec};
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
async fn build_pod_mounts_pvc_and_working_dir() {
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true")
        .pvc("shared-claim", "/workspace")
        .working_dir("/workspace")
        .build_pod();
    let spec = pod.spec.unwrap();
    let c = &spec.containers[0];
    assert_eq!(c.working_dir.as_deref(), Some("/workspace"));
    let mount = &c.volume_mounts.as_ref().unwrap()[0];
    assert_eq!(mount.mount_path, "/workspace");
    let vol = &spec.volumes.as_ref().unwrap()[0];
    assert_eq!(
        vol.persistent_volume_claim.as_ref().unwrap().claim_name,
        "shared-claim"
    );
    assert_eq!(vol.name, mount.name);
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
