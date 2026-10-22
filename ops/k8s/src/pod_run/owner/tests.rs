//! Tests for `owner_pod` on [`PodRun`] and [`JobRun`]: the `ownerReference`
//! of the manifest, and the checks of `run()` before any creation.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use http::{Method, Request, Response, StatusCode};
use hyper::body::Bytes;
use ironflow_core::error::OperationError;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use tower::service_fn;

use crate::KubeClient;
use crate::job_run::JobRun;
use crate::pod_run::PodRun;
use crate::pod_run::tests::{dummy_kube, kube_from};

fn worker_reference() -> Vec<OwnerReference> {
    vec![OwnerReference {
        api_version: "v1".to_string(),
        kind: "Pod".to_string(),
        name: "w".to_string(),
        uid: "uid-1".to_string(),
        controller: Some(false),
        block_owner_deletion: Some(false),
    }]
}

/// A fake API server: `GET pods/w` answers `owner_status`, with a pod of
/// uid `owner_uid` on 200; every POST is refused with a 500, so `run()`
/// stops right after the creation under test. Records `(method, path)`.
fn fake_api(
    owner_status: StatusCode,
    owner_uid: &'static str,
    requests: Arc<Mutex<Vec<(Method, String)>>>,
) -> KubeClient {
    kube_from(service_fn(move |req: Request<kube::client::Body>| {
        let requests = requests.clone();
        async move {
            let path = req.uri().path().to_string();
            requests
                .lock()
                .unwrap()
                .push((req.method().clone(), path.clone()));
            let (status, body) = if req.method() == Method::POST {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"refused","code":500}"#
                        .to_string(),
                )
            } else if path.ends_with("/pods/w") && owner_status == StatusCode::OK {
                (
                    StatusCode::OK,
                    format!(
                        r#"{{"kind":"Pod","apiVersion":"v1","metadata":{{"name":"w","namespace":"default","uid":"{owner_uid}"}},"spec":{{"containers":[]}}}}"#
                    ),
                )
            } else {
                (
                    StatusCode::NOT_FOUND,
                    r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"pods \"w\" not found","reason":"NotFound","code":404}"#
                        .to_string(),
                )
            };
            let mut response = Response::new(kube::client::Body::from(Bytes::from(body)));
            *response.status_mut() = status;
            Ok::<_, Infallible>(response)
        }
    }))
}

fn posts(requests: &Mutex<Vec<(Method, String)>>) -> usize {
    requests
        .lock()
        .unwrap()
        .iter()
        .filter(|(method, _)| method == Method::POST)
        .count()
}

fn external_message(err: OperationError) -> String {
    match err {
        OperationError::External { origin, message } => {
            assert_eq!(origin, "kubernetes");
            message
        }
        other => panic!("expected External kubernetes error, got: {other}"),
    }
}

// -- PodRun --

#[tokio::test]
async fn pod_run_owner_pod_sets_the_owner_reference_of_the_pod() {
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true")
        .owner_pod("w", "uid-1")
        .build_pod();
    assert_eq!(pod.metadata.owner_references, Some(worker_reference()));
}

#[tokio::test]
async fn pod_run_without_owner_pod_has_no_owner_reference() {
    let pod = PodRun::new(&dummy_kube(), "p", "busybox", "true").build_pod();
    assert_eq!(pod.metadata.owner_references, None);
}

#[tokio::test]
async fn pod_run_with_its_owner_running_creates_the_pod() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let kube = fake_api(StatusCode::OK, "uid-1", requests.clone());
    let err = PodRun::new(&kube, "p", "busybox", "true")
        .owner_pod("w", "uid-1")
        .run()
        .await
        .unwrap_err();
    assert!(external_message(err).contains("refused"));
    let seen = requests.lock().unwrap().clone();
    assert_eq!(
        seen,
        [
            (Method::GET, "/api/v1/namespaces/default/pods/w".to_string()),
            (Method::POST, "/api/v1/namespaces/default/pods".to_string()),
        ]
    );
}

#[tokio::test]
async fn pod_run_owner_pod_empty_fails_without_any_request() {
    for (name, uid) in [("", "uid-1"), ("w", ""), (" ", "uid-1")] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let kube = fake_api(StatusCode::OK, "uid-1", requests.clone());
        let err = PodRun::new(&kube, "p", "busybox", "true")
            .owner_pod(name, uid)
            .run()
            .await
            .unwrap_err();
        assert!(external_message(err).contains("owner_pod"));
        assert!(requests.lock().unwrap().is_empty(), "({name:?}, {uid:?})");
    }
}

#[tokio::test]
async fn pod_run_owner_absent_from_the_namespace_fails_before_creating_the_pod() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let kube = fake_api(StatusCode::NOT_FOUND, "uid-1", requests.clone());
    let err = PodRun::new(&kube, "p", "busybox", "true")
        .owner_pod("w", "uid-1")
        .run()
        .await
        .unwrap_err();
    let message = external_message(err);
    assert!(
        message.contains("owner pod 'w' not found in namespace 'default'"),
        "{message}"
    );
    assert_eq!(posts(&requests), 0);
}

#[tokio::test]
async fn pod_run_owner_with_another_uid_fails_before_creating_the_pod() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let kube = fake_api(StatusCode::OK, "uid-2", requests.clone());
    let err = PodRun::new(&kube, "p", "busybox", "true")
        .owner_pod("w", "uid-1")
        .run()
        .await
        .unwrap_err();
    let message = external_message(err);
    assert!(
        message.contains("uid-2") && message.contains("uid-1"),
        "{message}"
    );
    assert_eq!(posts(&requests), 0);
}

// -- JobRun --

#[tokio::test]
async fn job_run_owner_pod_sets_the_owner_reference_of_the_job_only() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "up")
        .owner_pod("w", "uid-1")
        .build_job();
    assert_eq!(job.metadata.owner_references, Some(worker_reference()));
    // The pods of the Job are owned by the Job: the garbage collector
    // deletes them with it.
    let template = job.spec.unwrap().template.metadata.unwrap();
    assert_eq!(template.owner_references, None);
}

#[tokio::test]
async fn job_run_without_owner_pod_has_no_owner_reference() {
    let job = JobRun::new(&dummy_kube(), "migrate", "migrate:1", "up").build_job();
    assert_eq!(job.metadata.owner_references, None);
}

#[tokio::test]
async fn job_run_with_its_owner_running_creates_the_job() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let kube = fake_api(StatusCode::OK, "uid-1", requests.clone());
    let err = JobRun::new(&kube, "migrate", "migrate:1", "up")
        .owner_pod("w", "uid-1")
        .run()
        .await
        .unwrap_err();
    assert!(external_message(err).contains("refused"));
    let seen = requests.lock().unwrap().clone();
    assert_eq!(
        seen,
        [
            (Method::GET, "/api/v1/namespaces/default/pods/w".to_string()),
            (
                Method::POST,
                "/apis/batch/v1/namespaces/default/jobs".to_string()
            ),
        ]
    );
}

#[tokio::test]
async fn job_run_owner_pod_empty_fails_without_any_request() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let kube = fake_api(StatusCode::OK, "uid-1", requests.clone());
    let err = JobRun::new(&kube, "migrate", "migrate:1", "up")
        .owner_pod("w", "")
        .run()
        .await
        .unwrap_err();
    assert!(external_message(err).contains("owner_pod"));
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn job_run_owner_absent_from_the_namespace_fails_before_creating_the_job() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let kube = fake_api(StatusCode::NOT_FOUND, "uid-1", requests.clone());
    let err = JobRun::new(&kube, "migrate", "migrate:1", "up")
        .owner_pod("w", "uid-1")
        .run()
        .await
        .unwrap_err();
    assert!(external_message(err).contains("not found in namespace 'default'"));
    assert_eq!(posts(&requests), 0);
}
