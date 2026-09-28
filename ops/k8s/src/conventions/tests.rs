//! The ironflow conventions on [`PodRun`] and [`JobRun`] manifests: labels
//! the orphan reaper and the run cleanup select on, and the expiry annotation.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use http::{Request, Response};
use hyper::body::Bytes;
use ironflow_core::provider::{
    LABEL_COMPONENT, LABEL_EXPIRES_AT, LABEL_MANAGED_BY, LABEL_RUN_ID, MANAGED_BY_IRONFLOW,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use tower::service_fn;

use crate::KubeClient;
use crate::job_run::JobRun;
use crate::pod_run::PodRun;

/// A client answering `{}` to everything: enough for pure manifest tests.
fn dummy_kube() -> KubeClient {
    let svc = service_fn(|_r: Request<kube::client::Body>| async {
        let body = kube::client::Body::from(Bytes::from_static(b"{}"));
        Ok::<_, Infallible>(Response::new(body))
    });
    KubeClient::from_raw(kube::Client::new(svc, "default"))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn labels(meta: &ObjectMeta) -> BTreeMap<String, String> {
    meta.labels.clone().unwrap_or_default()
}

fn expires_at(meta: &ObjectMeta) -> u64 {
    let annotations = meta.annotations.as_ref().expect("annotations");
    annotations[LABEL_EXPIRES_AT].parse().expect("unix seconds")
}

fn pod_run() -> PodRun {
    PodRun::new(&dummy_kube(), "check-1", "busybox", "true")
}

// -- PodRun --

#[tokio::test]
async fn pod_run_carries_ironflow_labels_without_caller_labels() {
    let pod = pod_run().build_pod();
    let expected = BTreeMap::from([
        (
            LABEL_MANAGED_BY.to_string(),
            MANAGED_BY_IRONFLOW.to_string(),
        ),
        (LABEL_COMPONENT.to_string(), "pod-run".to_string()),
    ]);
    assert_eq!(labels(&pod.metadata), expected);
}

#[tokio::test]
async fn pod_run_keeps_caller_labels_next_to_ironflow_labels() {
    let pod = pod_run().label(LABEL_RUN_ID, "run-1").build_pod();
    let labels = labels(&pod.metadata);
    assert_eq!(labels.len(), 3);
    assert_eq!(labels[LABEL_RUN_ID], "run-1");
    assert_eq!(labels[LABEL_MANAGED_BY], MANAGED_BY_IRONFLOW);
    assert_eq!(labels[LABEL_COMPONENT], "pod-run");
}

#[tokio::test]
async fn pod_run_expires_after_timeout_plus_default_margin() {
    let before = now_unix();
    let pod = pod_run().timeout(Duration::from_secs(300)).build_pod();
    let after = now_unix();
    let expiry = expires_at(&pod.metadata);
    assert!(expiry >= before + 360 && expiry <= after + 360, "{expiry}");
}

#[tokio::test]
async fn pod_run_expiry_margin_moves_the_annotation() {
    let before = now_unix();
    let pod = pod_run()
        .timeout(Duration::from_secs(300))
        .expiry_margin(Duration::from_secs(900))
        .build_pod();
    let after = now_unix();
    let expiry = expires_at(&pod.metadata);
    assert!(
        expiry >= before + 1200 && expiry <= after + 1200,
        "{expiry}"
    );
}

#[tokio::test]
#[should_panic(expected = "pod label 'app.kubernetes.io/component' is reserved")]
async fn pod_run_label_refuses_component() {
    let _ = pod_run().label(LABEL_COMPONENT, "agent");
}

#[tokio::test]
#[should_panic(expected = "pod label 'app.kubernetes.io/managed-by' is reserved")]
async fn pod_run_label_refuses_managed_by() {
    let _ = pod_run().label(LABEL_MANAGED_BY, "someone-else");
}

#[tokio::test]
#[should_panic(expected = "pod label 'app.kubernetes.io/component' is reserved")]
async fn pod_run_labels_refuses_a_reserved_key_in_the_map() {
    let map = BTreeMap::from([
        ("team".to_string(), "infra".to_string()),
        (LABEL_COMPONENT.to_string(), "agent".to_string()),
    ]);
    let _ = pod_run().labels(map);
}

// -- JobRun --

fn job_run() -> JobRun {
    JobRun::new(&dummy_kube(), "migrate", "migrate:1", "migrate up")
}

#[tokio::test]
async fn job_run_carries_ironflow_labels_on_job_and_pod_template() {
    let job = job_run().build_job();
    let expected = BTreeMap::from([
        (
            LABEL_MANAGED_BY.to_string(),
            MANAGED_BY_IRONFLOW.to_string(),
        ),
        (LABEL_COMPONENT.to_string(), "job-run".to_string()),
    ]);
    assert_eq!(labels(&job.metadata), expected);
    let template = job
        .spec
        .unwrap()
        .template
        .metadata
        .expect("template metadata");
    assert_eq!(labels(&template), expected);
}

#[tokio::test]
async fn job_run_expiry_is_on_the_job_only() {
    let before = now_unix();
    let job = job_run().timeout(Duration::from_secs(600)).build_job();
    let after = now_unix();
    let expiry = expires_at(&job.metadata);
    assert!(expiry >= before + 660 && expiry <= after + 660, "{expiry}");
    // Pods of a Job are reaped through their Job, never on their own.
    let template = job
        .spec
        .unwrap()
        .template
        .metadata
        .expect("template metadata");
    assert!(template.annotations.is_none());
}

#[tokio::test]
async fn job_run_caller_labels_reach_job_and_pods() {
    let job = job_run()
        .label(LABEL_RUN_ID, "run-1")
        .expiry_margin(Duration::from_secs(0))
        .build_job();
    assert_eq!(labels(&job.metadata)[LABEL_RUN_ID], "run-1");
    let template = job
        .spec
        .unwrap()
        .template
        .metadata
        .expect("template metadata");
    assert_eq!(labels(&template)[LABEL_RUN_ID], "run-1");
    assert_eq!(labels(&template).len(), 3);
}

#[tokio::test]
#[should_panic(expected = "pod label 'app.kubernetes.io/managed-by' is reserved")]
async fn job_run_label_refuses_managed_by() {
    let _ = job_run().label(LABEL_MANAGED_BY, "helm");
}

#[tokio::test]
#[should_panic(expected = "pod label 'app.kubernetes.io/component' is reserved")]
async fn job_run_labels_refuses_a_reserved_key_in_the_map() {
    let map = BTreeMap::from([(LABEL_COMPONENT.to_string(), "worker".to_string())]);
    let _ = job_run().labels(map);
}
