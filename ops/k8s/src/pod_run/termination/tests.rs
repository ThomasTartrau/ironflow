//! Tests for the exit code and reason [`PodRun::run`](crate::pod_run::PodRun::run)
//! reads on the terminal pod, over a canned Kubernetes API.

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use serde_json::from_str;

use crate::pod_run::tests::{pod_run_with, routing_service_with_statuses};
use crate::pod_run::{PodRun, PodRunOutput};

/// A `PodRun` over a pod whose every GET returns `status`.
fn pod_run_with_status(status: &str) -> PodRun {
    pod_run_with(routing_service_with_statuses(
        vec![status.to_string()],
        Arc::new(AtomicUsize::new(0)),
        "out\n".to_string(),
    ))
}

/// The JSON of a `ContainerStatus` named `name` in `state`.
fn container_status(name: &str, state: &str) -> String {
    format!(
        r#"{{"name":"{name}","image":"busybox","imageID":"","ready":false,"restartCount":0,"state":{state}}}"#
    )
}

#[tokio::test]
async fn run_failed_exit_3_reports_exit_code_and_container_reason() {
    let status = format!(
        r#"{{"phase":"Failed","containerStatuses":[{}]}}"#,
        container_status(
            "job-pod",
            r#"{"terminated":{"exitCode":3,"reason":"Error"}}"#
        )
    );
    let out = pod_run_with_status(&status).run().await.unwrap();
    assert!(!out.success);
    assert_eq!(out.exit_code, Some(3));
    assert_eq!(out.reason.as_deref(), Some("Error"));
}

#[tokio::test]
async fn run_deadline_exceeded_reports_reason_without_exit_code() {
    // Killed by activeDeadlineSeconds: the pod carries the reason, and the
    // container status may still be the last one seen, `running`.
    let status = format!(
        r#"{{"phase":"Failed","reason":"DeadlineExceeded","message":"Pod was active on the node longer than the specified deadline","containerStatuses":[{}]}}"#,
        container_status(
            "job-pod",
            r#"{"running":{"startedAt":"2026-10-21T08:00:00Z"}}"#
        )
    );
    let out = pod_run_with_status(&status).run().await.unwrap();
    assert!(!out.success);
    assert_eq!(out.phase, "Failed");
    assert_eq!(out.reason.as_deref(), Some("DeadlineExceeded"));
    assert_eq!(out.exit_code, None);
}

#[tokio::test]
async fn run_reads_exit_code_of_own_container_not_of_first_status() {
    // An injected sidecar can come first in containerStatuses.
    let status = format!(
        r#"{{"phase":"Failed","containerStatuses":[{},{}]}}"#,
        container_status(
            "istio-proxy",
            r#"{"terminated":{"exitCode":0,"reason":"Completed"}}"#
        ),
        container_status(
            "job-pod",
            r#"{"terminated":{"exitCode":137,"reason":"OOMKilled"}}"#
        )
    );
    let out = pod_run_with_status(&status).run().await.unwrap();
    assert_eq!(out.exit_code, Some(137));
    assert_eq!(out.reason.as_deref(), Some("OOMKilled"));
}

#[tokio::test]
async fn run_pod_reason_wins_over_container_reason() {
    let status = format!(
        r#"{{"phase":"Failed","reason":"Evicted","containerStatuses":[{}]}}"#,
        container_status(
            "job-pod",
            r#"{"terminated":{"exitCode":137,"reason":"Error"}}"#
        )
    );
    let out = pod_run_with_status(&status).run().await.unwrap();
    assert_eq!(out.reason.as_deref(), Some("Evicted"));
    assert_eq!(out.exit_code, Some(137));
}

#[tokio::test]
async fn run_succeeded_reports_exit_code_zero() {
    let status = format!(
        r#"{{"phase":"Succeeded","containerStatuses":[{}]}}"#,
        container_status(
            "job-pod",
            r#"{"terminated":{"exitCode":0,"reason":"Completed"}}"#
        )
    );
    let out = pod_run_with_status(&status).run().await.unwrap();
    assert!(out.success);
    assert_eq!(out.exit_code, Some(0));
    assert_eq!(out.reason.as_deref(), Some("Completed"));
}

#[tokio::test]
async fn run_without_container_status_reports_neither() {
    let out = pod_run_with_status(r#"{"phase":"Failed"}"#)
        .run()
        .await
        .unwrap();
    assert_eq!(out.exit_code, None);
    assert_eq!(out.reason, None);
}

#[test]
fn pod_run_output_without_exit_code_and_reason_deserializes_as_none() {
    // An output persisted before these fields existed.
    let out: PodRunOutput =
        from_str(r#"{"success":false,"logs":"","phase":"Failed","logs_truncated":false}"#).unwrap();
    assert_eq!(out.exit_code, None);
    assert_eq!(out.reason, None);
}
