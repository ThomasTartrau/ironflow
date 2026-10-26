//! Tests for [`PodUp`](super::PodUp): the manifest it creates, and `run()`
//! over a canned Kubernetes API.

use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use http::{Method, Request, Response, StatusCode};
use hyper::body::Bytes;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_core::provider::{LABEL_COMPONENT, LABEL_EXPIRES_AT, LABEL_MANAGED_BY, LABEL_RUN_ID};
use k8s_openapi::api::core::v1::{ExecAction, Pod, Probe};
use kube::client::Body;
use serde_json::{from_str, from_value};
use tokio::time::timeout;
use tower::Service;
use tower::service_fn;

use super::{PodUp, PodUpOutput};
use crate::pod_run::tests::{dummy_kube, kube_from};
use crate::pod_run::{MAX_STDIN_BYTES, PodRun};

const LIFETIME: Duration = Duration::from_secs(3600);

/// What the canned API saw: each request as `"METHOD path?query"`, a POST
/// followed by a space and its body.
type Seen = Arc<Mutex<Vec<String>>>;

/// Requests the canned API refuses with a `Status` failure.
#[derive(Clone, Copy, Default)]
struct Fail {
    post: bool,
    get: bool,
    delete: bool,
    log: bool,
}

/// A `Status` failure with `code` and `message`.
fn failure(code: StatusCode, message: &str) -> (StatusCode, String) {
    (
        code,
        format!(
            r#"{{"kind":"Status","apiVersion":"v1","status":"Failure","message":"{message}","code":{}}}"#,
            code.as_u16()
        ),
    )
}

/// A canned API serving the pod `svc`: each GET on the pod returns the next
/// of `statuses` (the JSON of a whole `PodStatus`, the last one repeats), the
/// log endpoint returns `logs`, a POST answers with a pod, and every request
/// in `fail` answers a failure.
fn api(
    statuses: Vec<String>,
    logs: String,
    fail: Fail,
    seen: Seen,
) -> impl Service<Request<Body>, Response = Response<Body>, Error = Infallible, Future = impl Send> + Clone
{
    let gets = Arc::new(AtomicUsize::new(0));
    let statuses = Arc::new(statuses);
    let logs = Arc::new(logs);
    service_fn(move |req: Request<Body>| {
        let gets = gets.clone();
        let statuses = statuses.clone();
        let logs = logs.clone();
        let seen = seen.clone();
        async move {
            let method = req.method().clone();
            let uri = req.uri().clone();
            let mut entry = format!("{method} {uri}");
            if method == Method::POST {
                let body = req.into_body().collect_bytes().await.unwrap();
                entry = format!("{entry} {}", String::from_utf8(body.to_vec()).unwrap());
            }
            seen.lock().unwrap().push(entry);
            let (status, body) = if method == Method::DELETE {
                if fail.delete {
                    failure(StatusCode::INTERNAL_SERVER_ERROR, "etcd unavailable")
                } else {
                    (
                        StatusCode::OK,
                        r#"{"kind":"Status","apiVersion":"v1","status":"Success"}"#.to_string(),
                    )
                }
            } else if uri.path().ends_with("/log") {
                if fail.log {
                    failure(StatusCode::BAD_REQUEST, "container not started")
                } else {
                    (StatusCode::OK, logs.to_string())
                }
            } else if method == Method::POST {
                if fail.post {
                    failure(StatusCode::FORBIDDEN, "quota exceeded")
                } else {
                    (
                        StatusCode::CREATED,
                        r#"{"kind":"Pod","apiVersion":"v1","metadata":{"name":"svc"},"spec":{"containers":[]}}"#.to_string(),
                    )
                }
            } else if fail.get {
                failure(StatusCode::INTERNAL_SERVER_ERROR, "apiserver overloaded")
            } else {
                let i = gets.fetch_add(1, Ordering::SeqCst);
                let pod_status = statuses.get(i).or_else(|| statuses.last()).unwrap();
                (
                    StatusCode::OK,
                    format!(
                        r#"{{"kind":"Pod","apiVersion":"v1","metadata":{{"name":"svc"}},"spec":{{"containers":[]}},"status":{pod_status}}}"#
                    ),
                )
            };
            let mut response = Response::new(Body::from(Bytes::from(body)));
            *response.status_mut() = status;
            Ok::<_, Infallible>(response)
        }
    })
}

/// A `PodUp` over `statuses`, polling every millisecond.
fn pod_up(statuses: &[&str], logs: &str, seen: &Seen) -> PodUp {
    pod_up_failing(statuses, logs, Fail::default(), seen)
}

/// Same as [`pod_up`], the requests of `fail` refused.
fn pod_up_failing(statuses: &[&str], logs: &str, fail: Fail, seen: &Seen) -> PodUp {
    let svc = api(
        statuses.iter().map(|s| s.to_string()).collect(),
        logs.to_string(),
        fail,
        seen.clone(),
    );
    PodUp::new(
        PodRun::new(&kube_from(svc), "svc", "busybox", "serve")
            .poll_interval(Duration::from_millis(1)),
        LIFETIME,
    )
    .ready_timeout(Duration::from_secs(5))
}

/// The pod the canned API received in the POST.
fn created_pod(seen: &Seen) -> Pod {
    let requests = seen.lock().unwrap();
    let post = requests.iter().find(|r| r.starts_with("POST")).unwrap();
    let (_, body) = post.split_once(" {").unwrap();
    from_str(&format!("{{{body}")).unwrap()
}

fn deletes(seen: &Seen) -> usize {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|r| r.starts_with("DELETE"))
        .count()
}

fn kubernetes_message(err: OperationError) -> String {
    match err {
        OperationError::External { origin, message } => {
            assert_eq!(origin, "kubernetes");
            message
        }
        other => panic!("expected an External kubernetes error, got: {other}"),
    }
}

/// The JSON of the `ContainerStatus` of the `svc` container in `state`.
fn container(state: &str) -> String {
    format!(
        r#"{{"name":"svc","image":"busybox","imageID":"","ready":false,"restartCount":0,"state":{state}}}"#
    )
}

const PENDING: &str = r#"{"phase":"Pending"}"#;
const READY: &str =
    r#"{"phase":"Running","podIP":"10.0.0.7","conditions":[{"type":"Ready","status":"True"}]}"#;

// -- build_pod (pure) --

#[tokio::test]
async fn build_pod_sets_the_readiness_probe_on_the_container() {
    let probe = Probe {
        exec: Some(ExecAction {
            command: Some(vec!["curl".to_string(), "-sf".to_string()]),
        }),
        period_seconds: Some(5),
        ..Default::default()
    };
    let pod = PodUp::new(
        PodRun::new(&dummy_kube(), "svc", "busybox", "serve"),
        LIFETIME,
    )
    .readiness_probe(probe.clone())
    .build_pod();
    let container = &pod.spec.unwrap().containers[0];
    assert_eq!(container.readiness_probe, Some(probe));
}

#[tokio::test]
async fn build_pod_without_probe_leaves_it_unset() {
    let pod = PodUp::new(
        PodRun::new(&dummy_kube(), "svc", "busybox", "serve"),
        LIFETIME,
    )
    .build_pod();
    assert_eq!(pod.spec.unwrap().containers[0].readiness_probe, None);
}

#[tokio::test]
async fn build_pod_expires_after_the_lifetime_not_the_run_timeout() {
    // The reaper deletes the pod at expires-at: it must cover the lifetime of
    // the service, not the 300s timeout of a run-to-completion PodRun.
    let before = Utc::now().timestamp();
    let pod = PodUp::new(
        PodRun::new(&dummy_kube(), "svc", "busybox", "serve")
            .timeout(Duration::from_secs(10))
            .expiry_margin(Duration::from_secs(30)),
        LIFETIME,
    )
    .build_pod();
    let after = Utc::now().timestamp();
    let expires_at: i64 = pod.metadata.annotations.unwrap()[LABEL_EXPIRES_AT]
        .parse()
        .unwrap();
    assert!(
        (before + 3630..=after + 3630).contains(&expires_at),
        "expires-at {expires_at} is not creation + lifetime + margin"
    );
}

#[tokio::test]
async fn build_pod_carries_the_ironflow_labels() {
    let pod = PodUp::new(
        PodRun::new(&dummy_kube(), "svc", "busybox", "serve").label(LABEL_RUN_ID, "r1"),
        LIFETIME,
    )
    .build_pod();
    let labels = pod.metadata.labels.unwrap();
    assert_eq!(labels[LABEL_MANAGED_BY], "ironflow");
    assert_eq!(labels[LABEL_COMPONENT], "pod-run");
    assert_eq!(labels[LABEL_RUN_ID], "r1");
}

// -- run() --

#[tokio::test]
async fn run_returns_the_ip_once_ready_and_keeps_the_pod() {
    let seen = Seen::default();
    let out = pod_up(&[PENDING, READY], "", &seen).run().await.unwrap();
    assert_eq!(
        out,
        PodUpOutput {
            pod: "svc".to_string(),
            namespace: "default".to_string(),
            ip: "10.0.0.7".to_string(),
        }
    );
    assert_eq!(deletes(&seen), 0, "a ready service pod must keep running");
}

#[tokio::test]
async fn run_waits_while_ready_has_no_ip_yet() {
    let seen = Seen::default();
    let ready_without_ip = r#"{"phase":"Running","conditions":[{"type":"Ready","status":"True"}]}"#;
    let not_ready = r#"{"phase":"Running","podIP":"10.0.0.7","conditions":[{"type":"Ready","status":"False"}]}"#;
    let out = pod_up(&[ready_without_ip, not_ready, READY], "", &seen)
        .run()
        .await
        .unwrap();
    assert_eq!(out.ip, "10.0.0.7");
    let gets = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.starts_with("GET") && !r.contains("/log"))
        .count();
    assert_eq!(gets, 3);
}

#[tokio::test]
async fn run_fails_with_reason_and_log_tail_when_the_pod_exits() {
    let seen = Seen::default();
    let failed = format!(
        r#"{{"phase":"Failed","containerStatuses":[{}]}}"#,
        container(r#"{"terminated":{"exitCode":2,"reason":"Error"}}"#)
    );
    let err = pod_up(&[PENDING, &failed], "listening\nboom: port in use\n", &seen)
        .run()
        .await
        .unwrap_err();
    let message = kubernetes_message(err);
    assert!(
        message.contains("pod 'svc' ended before getting ready"),
        "got: {message}"
    );
    assert!(message.contains("phase Failed"), "got: {message}");
    assert!(message.contains("reason Error"), "got: {message}");
    assert!(message.contains("exit code 2"), "got: {message}");
    assert!(message.ends_with("boom: port in use\n"), "got: {message}");
    assert_eq!(deletes(&seen), 1, "a pod that never got ready is deleted");
}

#[tokio::test]
async fn run_reads_only_the_tail_of_the_logs() {
    let seen = Seen::default();
    let _ = pod_up(&[r#"{"phase":"Succeeded"}"#], "done\n", &seen)
        .run()
        .await
        .unwrap_err();
    let requests = seen.lock().unwrap();
    let log = requests.iter().find(|r| r.contains("/log")).unwrap();
    assert!(log.contains("tailLines=40"), "got: {log}");
}

#[tokio::test]
async fn run_caps_the_log_tail_at_max_log_bytes() {
    let seen = Seen::default();
    let svc = api(
        vec![r#"{"phase":"Failed"}"#.to_string()],
        "0123456789".to_string(),
        Fail::default(),
        seen.clone(),
    );
    let err = PodUp::new(
        PodRun::new(&kube_from(svc), "svc", "busybox", "serve").max_log_bytes(4),
        LIFETIME,
    )
    .run()
    .await
    .unwrap_err();
    let message = kubernetes_message(err);
    assert!(
        message.ends_with("[... 6 bytes truncated ...]\n6789"),
        "got: {message}"
    );
}

#[tokio::test]
async fn run_fails_with_the_waiting_reason_when_not_ready_in_time() {
    let seen = Seen::default();
    let pulling = format!(
        r#"{{"phase":"Pending","containerStatuses":[{}]}}"#,
        container(
            r#"{"waiting":{"reason":"ImagePullBackOff","message":"Back-off pulling image \"busybox:nope\""}}"#
        )
    );
    // Bounded: a wait loop that ignored `ready_timeout` would hang forever.
    let err = timeout(
        Duration::from_secs(10),
        pod_up(&[&pulling], "", &seen)
            .ready_timeout(Duration::from_millis(20))
            .run(),
    )
    .await
    .expect("ready_timeout must end the wait")
    .unwrap_err();
    let message = kubernetes_message(err);
    assert!(
        message.contains("pod 'svc' was not ready within 20ms"),
        "got: {message}"
    );
    assert!(
        message.contains("ImagePullBackOff: Back-off pulling image \"busybox:nope\""),
        "got: {message}"
    );
    assert!(message.contains("(no logs)"), "got: {message}");
    assert_eq!(deletes(&seen), 1, "a pod that never got ready is deleted");
}

#[tokio::test]
async fn run_create_error_is_kubernetes_and_deletes_nothing() {
    let seen = Seen::default();
    let svc = api(
        vec![READY.to_string()],
        String::new(),
        Fail {
            post: true,
            ..Fail::default()
        },
        seen.clone(),
    );
    let err = PodUp::new(
        PodRun::new(&kube_from(svc), "svc", "busybox", "serve"),
        LIFETIME,
    )
    .run()
    .await
    .unwrap_err();
    let message = kubernetes_message(err);
    assert!(message.contains("quota exceeded"), "got: {message}");
    assert_eq!(deletes(&seen), 0);
}

#[tokio::test]
async fn run_creates_the_pod_with_the_probe_labels_and_lifetime() {
    let seen = Seen::default();
    let probe = Probe {
        exec: Some(ExecAction {
            command: Some(vec!["ready".to_string()]),
        }),
        ..Default::default()
    };
    let before = Utc::now().timestamp();
    pod_up(&[READY], "", &seen)
        .readiness_probe(probe.clone())
        .run()
        .await
        .unwrap();
    let pod = created_pod(&seen);
    assert_eq!(pod.spec.unwrap().containers[0].readiness_probe, Some(probe));
    assert_eq!(pod.metadata.labels.unwrap()[LABEL_MANAGED_BY], "ironflow");
    let expires_at: i64 = pod.metadata.annotations.unwrap()[LABEL_EXPIRES_AT]
        .parse()
        .unwrap();
    // Creation + lifetime + the default 60s margin.
    assert!(expires_at >= before + 3660, "got {expires_at}");
}

#[tokio::test]
async fn run_deletes_the_pod_when_it_cannot_be_read() {
    let seen = Seen::default();
    let fail = Fail {
        get: true,
        ..Fail::default()
    };
    let err = pod_up_failing(&[READY], "", fail, &seen)
        .run()
        .await
        .unwrap_err();
    let message = kubernetes_message(err);
    assert!(
        message.contains("failed to read pod 'svc'"),
        "got: {message}"
    );
    assert!(message.contains("apiserver overloaded"), "got: {message}");
    assert_eq!(deletes(&seen), 1);
}

#[tokio::test]
async fn run_says_when_the_pod_could_not_be_deleted() {
    let seen = Seen::default();
    let fail = Fail {
        delete: true,
        ..Fail::default()
    };
    let err = pod_up_failing(&[r#"{"phase":"Failed"}"#], "", fail, &seen)
        .run()
        .await
        .unwrap_err();
    let message = kubernetes_message(err);
    assert!(message.contains("(not deleted: "), "got: {message}");
    assert!(message.contains("etcd unavailable"), "got: {message}");
}

#[tokio::test]
async fn run_says_when_the_logs_cannot_be_read() {
    let seen = Seen::default();
    let fail = Fail {
        log: true,
        ..Fail::default()
    };
    let err = pod_up_failing(&[r#"{"phase":"Failed"}"#], "", fail, &seen)
        .run()
        .await
        .unwrap_err();
    let message = kubernetes_message(err);
    assert!(message.contains("(logs unavailable: "), "got: {message}");
    assert!(message.contains("container not started"), "got: {message}");
    assert_eq!(deletes(&seen), 1);
}

#[tokio::test]
async fn run_accepts_the_longest_durations() {
    // `Duration::MAX` reads as "no limit": it must not overflow a deadline
    // or the expiry annotation.
    let seen = Seen::default();
    let svc = api(
        vec![READY.to_string()],
        String::new(),
        Fail::default(),
        seen.clone(),
    );
    let out = PodUp::new(
        PodRun::new(&kube_from(svc), "svc", "busybox", "serve"),
        Duration::MAX,
    )
    .ready_timeout(Duration::MAX)
    .run()
    .await
    .unwrap();
    assert_eq!(out.ip, "10.0.0.7");
}

#[tokio::test]
async fn run_refuses_an_oversized_stdin_before_creating_the_pod() {
    let seen = Seen::default();
    let svc = api(
        vec![READY.to_string()],
        String::new(),
        Fail::default(),
        seen.clone(),
    );
    let err = PodUp::new(
        PodRun::new(&kube_from(svc), "svc", "busybox", "serve")
            .stdin("x".repeat(MAX_STDIN_BYTES + 1)),
        LIFETIME,
    )
    .run()
    .await
    .unwrap_err();
    let message = kubernetes_message(err);
    assert!(message.contains("stdin of pod 'svc'"), "got: {message}");
    assert!(seen.lock().unwrap().is_empty(), "no request may be sent");
}

// -- Operation --

#[tokio::test]
async fn execute_serializes_the_output() {
    let seen = Seen::default();
    let op = pod_up(&[READY], "", &seen);
    let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    let value = op.execute(&ctx).await.unwrap();
    let out: PodUpOutput = from_value(value).unwrap();
    assert_eq!(out.ip, "10.0.0.7");
}

#[tokio::test]
async fn input_describes_the_pod_and_both_durations() {
    let op = PodUp::new(
        PodRun::new(&dummy_kube(), "svc", "busybox", "serve").namespace("qa"),
        LIFETIME,
    )
    .ready_timeout(Duration::from_secs(90));
    let input = op.input().unwrap();
    assert_eq!(input["name"], "svc");
    assert_eq!(input["namespace"], "qa");
    assert_eq!(input["image"], "busybox");
    assert_eq!(input["command"], "serve");
    assert_eq!(input["ready_timeout_secs"], 90);
    assert_eq!(input["lifetime_secs"], 3600);
}

#[test]
fn output_round_trips_through_json() {
    let out: PodUpOutput = from_str(r#"{"pod":"svc","namespace":"qa","ip":"10.0.0.7"}"#).unwrap();
    assert_eq!(out.namespace, "qa");
}
