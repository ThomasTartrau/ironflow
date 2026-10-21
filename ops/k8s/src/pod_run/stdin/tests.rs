//! Tests for [`PodRun::stdin`](crate::pod_run::PodRun::stdin): the manifest
//! `build_pod` renders and the size check of `run()`.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use http::{Request, Response};
use hyper::body::Bytes;
use ironflow_core::error::OperationError;
use tower::service_fn;

use super::{MAX_STDIN_BYTES, STDIN_ANNOTATION};
use crate::pod_run::PodRun;
use crate::pod_run::tests::{dummy_kube, pod_run_with, routing_service};

#[tokio::test]
async fn build_pod_stdin_projects_the_input_into_a_file_read_by_the_command() {
    let pod = PodRun::new(&dummy_kube(), "p1", "busybox", "wc -c")
        .stdin("{\"doc\": 1}")
        .build_pod();

    let annotations = pod.metadata.annotations.unwrap();
    assert_eq!(
        annotations.get(STDIN_ANNOTATION).map(String::as_str),
        Some("{\"doc\": 1}")
    );

    let spec = pod.spec.unwrap();
    let volume = &spec.volumes.as_ref().unwrap()[0];
    assert_eq!(volume.name, "ironflow-stdin");
    let items = volume
        .downward_api
        .as_ref()
        .unwrap()
        .items
        .as_ref()
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].path, "stdin");
    assert_eq!(
        items[0].field_ref.as_ref().unwrap().field_path,
        "metadata.annotations['ironflow.io/stdin']"
    );

    let container = &spec.containers[0];
    let mount = &container.volume_mounts.as_ref().unwrap()[0];
    assert_eq!(mount.name, "ironflow-stdin");
    assert_eq!(mount.mount_path, "/var/run/ironflow");
    assert_eq!(mount.read_only, Some(true));
    assert_eq!(
        container.command.as_ref().unwrap(),
        &vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "exec < /var/run/ironflow/stdin\nwc -c".to_string()
        ]
    );
}

#[tokio::test]
async fn build_pod_stdin_comes_after_the_caller_volumes() {
    let pod = PodRun::new(&dummy_kube(), "p1", "busybox", "cat")
        .pvc("claim", "/workspace")
        .stdin("x")
        .build_pod();
    let spec = pod.spec.unwrap();
    let names: Vec<&str> = spec
        .volumes
        .as_ref()
        .unwrap()
        .iter()
        .map(|v| v.name.as_str())
        .collect();
    assert_eq!(names, ["workspace", "ironflow-stdin"]);
    let mounts: Vec<&str> = spec.containers[0]
        .volume_mounts
        .as_ref()
        .unwrap()
        .iter()
        .map(|m| m.mount_path.as_str())
        .collect();
    assert_eq!(mounts, ["/workspace", "/var/run/ironflow"]);
}

#[tokio::test]
async fn build_pod_without_stdin_keeps_the_plain_command() {
    let pod = PodRun::new(&dummy_kube(), "p1", "busybox", "echo hi").build_pod();
    assert!(
        !pod.metadata
            .annotations
            .unwrap()
            .contains_key(STDIN_ANNOTATION)
    );
    let spec = pod.spec.unwrap();
    assert_eq!(spec.volumes, None);
    assert_eq!(
        spec.containers[0].command.as_ref().unwrap(),
        &vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "echo hi".to_string()
        ]
    );
}

#[tokio::test]
async fn build_pod_empty_stdin_gives_the_command_an_empty_file() {
    let pod = PodRun::new(&dummy_kube(), "p1", "busybox", "cat")
        .stdin("")
        .build_pod();
    assert_eq!(
        pod.metadata
            .annotations
            .unwrap()
            .get(STDIN_ANNOTATION)
            .map(String::as_str),
        Some("")
    );
    assert_eq!(
        pod.spec.unwrap().containers[0].command.as_ref().unwrap()[2],
        "exec < /var/run/ironflow/stdin\ncat"
    );
}

#[tokio::test]
async fn run_refuses_oversized_stdin_without_creating_the_pod() {
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = requests.clone();
    let svc = service_fn(move |_req: Request<kube::client::Body>| {
        counted.fetch_add(1, Ordering::SeqCst);
        async {
            Ok::<_, Infallible>(Response::new(kube::client::Body::from(Bytes::from_static(
                b"{}",
            ))))
        }
    });
    let err = pod_run_with(svc)
        .stdin("x".repeat(MAX_STDIN_BYTES + 1))
        .run()
        .await
        .unwrap_err();
    match err {
        OperationError::External { origin, message } => {
            assert_eq!(origin, "kubernetes");
            assert!(
                message.contains(&format!("{} bytes", MAX_STDIN_BYTES + 1)),
                "got: {message}"
            );
        }
        other => panic!("expected External kubernetes error, got: {other}"),
    }
    assert_eq!(requests.load(Ordering::SeqCst), 0, "no pod may be created");
}

#[tokio::test]
async fn run_accepts_stdin_of_exactly_the_limit() {
    let out = pod_run_with(routing_service(
        vec!["Succeeded"],
        Arc::new(AtomicUsize::new(0)),
    ))
    .stdin("x".repeat(MAX_STDIN_BYTES))
    .run()
    .await
    .unwrap();
    assert!(out.success);
}
