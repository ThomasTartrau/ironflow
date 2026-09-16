//! Tests for [`JobRun`](super::JobRun): pure `build_job` manifest assertions
//! and transport-mocked `run()` completion/cleanup behaviour.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use http::{Method, Request, Response};
use hyper::body::Bytes;
use ironflow_core::operation::Operation;
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
async fn kind_and_input() {
    let op = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up").backoff_limit(2);
    assert_eq!(op.kind(), "k8s");
    let input = op.input().unwrap();
    assert_eq!(input["name"], "migrate");
    assert_eq!(input["backoff_limit"], 2);
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
    let get_idx = Arc::new(AtomicUsize::new(0));
    let conditions = Arc::new(conditions);
    service_fn(move |req: Request<kube::client::Body>| {
        let get_idx = get_idx.clone();
        let conditions = conditions.clone();
        let deleted = deleted.clone();
        async move {
            let method = req.method().clone();
            let path = req.uri().path().to_string();
            let body: String = if method == Method::POST {
                r#"{"kind":"Job","apiVersion":"batch/v1","metadata":{"name":"migrate","namespace":"default"},"spec":{"template":{}},"status":{}}"#.to_string()
            } else if path.ends_with("/log") {
                "migration-output\n".to_string()
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
    assert_eq!(deleted.load(Ordering::SeqCst), 1, "job must be deleted");
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
