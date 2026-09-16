//! Tests for [`ApplyConfigMap`](super::ApplyConfigMap) and
//! [`ApplySecret`](super::ApplySecret): pure manifest builders, secret-key
//! masking, the apply-body typemeta injection, and transport-mocked `run()`.

use std::collections::BTreeMap;
use std::convert::Infallible;

use http::{Request, Response};
use hyper::body::Bytes;
use ironflow_core::operation::Operation;
use serde_json::json;
use tower::Service;
use tower::service_fn;

use super::{ApplyConfigMap, ApplySecret, apply_body};
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

fn data(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// -- pure builders --

#[tokio::test]
async fn build_config_map_carries_data() {
    let cm = ApplyConfigMap::new(
        &dummy_kube(),
        "ci",
        "app-config",
        data(&[("LOG_LEVEL", "info")]),
    )
    .build_config_map();
    assert_eq!(cm.metadata.name.as_deref(), Some("app-config"));
    assert_eq!(cm.metadata.namespace.as_deref(), Some("ci"));
    assert_eq!(cm.data.unwrap()["LOG_LEVEL"], "info");
}

#[tokio::test]
async fn build_secret_puts_values_in_string_data() {
    let secret = ApplySecret::new(
        &dummy_kube(),
        "ci",
        "app-secrets",
        data(&[("API_TOKEN", "s3cr3t")]),
    )
    .build_secret();
    assert_eq!(secret.type_.as_deref(), Some("Opaque"));
    assert_eq!(secret.string_data.unwrap()["API_TOKEN"], "s3cr3t");
}

// -- security: input() must not leak secret values --

#[tokio::test]
async fn secret_input_exposes_keys_not_values() {
    let op = ApplySecret::new(
        &dummy_kube(),
        "ci",
        "app-secrets",
        data(&[("API_TOKEN", "s3cr3t"), ("DB_PASS", "hunter2")]),
    );
    let input = op.input().unwrap();
    let rendered = input.to_string();
    // Keys present...
    assert_eq!(input["name"], "app-secrets");
    assert!(rendered.contains("API_TOKEN"));
    assert!(rendered.contains("DB_PASS"));
    // ...values absent.
    assert!(
        !rendered.contains("s3cr3t"),
        "secret value leaked into input(): {rendered}"
    );
    assert!(
        !rendered.contains("hunter2"),
        "secret value leaked into input(): {rendered}"
    );
}

// -- apply_body injects typemeta required by server-side apply --

#[test]
fn apply_body_injects_api_version_and_kind() {
    let body = apply_body(&json!({"metadata": {"name": "x"}}), "v1", "ConfigMap").unwrap();
    assert_eq!(body["apiVersion"], "v1");
    assert_eq!(body["kind"], "ConfigMap");
    assert_eq!(body["metadata"]["name"], "x");
}

// -- run(): server-side apply against a canned transport --

#[tokio::test]
async fn config_map_run_returns_name_and_namespace() {
    let svc = service_fn(|_r: Request<kube::client::Body>| async {
        Ok::<_, Infallible>(Response::new(kube::client::Body::from(Bytes::from_static(
            br#"{"kind":"ConfigMap","apiVersion":"v1","metadata":{"name":"app-config","namespace":"ci"},"data":{"LOG_LEVEL":"info"}}"#,
        ))))
    });
    let out = ApplyConfigMap::new(
        &kube_from(svc),
        "ci",
        "app-config",
        data(&[("LOG_LEVEL", "info")]),
    )
    .run()
    .await
    .unwrap();
    assert_eq!(out.name, "app-config");
    assert_eq!(out.namespace, "ci");
}

#[tokio::test]
async fn secret_run_returns_name_and_namespace() {
    let svc = service_fn(|_r: Request<kube::client::Body>| async {
        Ok::<_, Infallible>(Response::new(kube::client::Body::from(Bytes::from_static(
            br#"{"kind":"Secret","apiVersion":"v1","metadata":{"name":"app-secrets","namespace":"ci"},"type":"Opaque"}"#,
        ))))
    });
    let out = ApplySecret::new(
        &kube_from(svc),
        "ci",
        "app-secrets",
        data(&[("API_TOKEN", "s3cr3t")]),
    )
    .run()
    .await
    .unwrap();
    assert_eq!(out.name, "app-secrets");
    assert_eq!(out.namespace, "ci");
}
