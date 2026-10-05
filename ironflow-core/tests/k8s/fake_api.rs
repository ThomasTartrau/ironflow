//! A fake Kubernetes API server on a real TCP port (wiremock), reached by the
//! real `kube` client through an inline kubeconfig.
//!
//! Each list route only answers the exact label selector the test expects:
//! a request with another selector gets a 404 and fails the pass under test.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ironflow_core::provider::{LABEL_COMPONENT, LABEL_EXPIRES_AT, LABEL_MANAGED_BY};
use ironflow_core::providers::claude::K8sClusterConfig;

/// Namespace every fake object lives in.
pub const NS: &str = "agents";

/// Unix time now, in seconds.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Collection path of `kind` (`pods`, `configmaps`, `jobs`,
/// `persistentvolumeclaims`) in [`NS`].
pub fn collection(kind: &str) -> String {
    match kind {
        "jobs" => format!("/apis/batch/v1/namespaces/{NS}/jobs"),
        other => format!("/api/v1/namespaces/{NS}/{other}"),
    }
}

pub struct FakeK8s {
    pub server: MockServer,
}

impl FakeK8s {
    pub async fn start() -> Self {
        Self {
            server: MockServer::start().await,
        }
    }

    /// Kubeconfig pointing the `kube` client at this server.
    pub fn cluster_config(&self) -> K8sClusterConfig {
        K8sClusterConfig::KubeconfigInline(format!(
            "apiVersion: v1
kind: Config
clusters:
- name: fake
  cluster:
    server: {}
contexts:
- name: fake
  context:
    cluster: fake
    user: fake
current-context: fake
users:
- name: fake
  user: {{}}
",
            self.server.uri()
        ))
    }

    /// Answer `GET <kind>?labelSelector=<selector>` with `items`, at least
    /// once.
    pub async fn list(&self, kind: &str, selector: &str, items: Vec<Value>) {
        Mock::given(method("GET"))
            .and(path(collection(kind)))
            .and(query_param("labelSelector", selector))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "apiVersion": "v1",
                "kind": "List",
                "metadata": { "resourceVersion": "1" },
                "items": items,
            })))
            .expect(1..)
            .mount(&self.server)
            .await;
    }

    /// Answer `GET <kind>?labelSelector=<selector>` with no item, whether or
    /// not the code under test gets that far.
    pub async fn list_nothing(&self, kind: &str, selector: &str) {
        Mock::given(method("GET"))
            .and(path(collection(kind)))
            .and(query_param("labelSelector", selector))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "apiVersion": "v1",
                "kind": "List",
                "metadata": { "resourceVersion": "1" },
                "items": [],
            })))
            .mount(&self.server)
            .await;
    }

    /// Answer `GET <kind>?labelSelector=<selector>` with an API error.
    pub async fn fail_list(&self, kind: &str, selector: &str, status: u16) {
        Mock::given(method("GET"))
            .and(path(collection(kind)))
            .and(query_param("labelSelector", selector))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({
                "apiVersion": "v1",
                "kind": "Status",
                "status": "Failure",
                "message": "refused by the fake API server",
                "reason": "Forbidden",
                "code": status,
            })))
            .mount(&self.server)
            .await;
    }

    /// Answer the wait on pod `name` (a list on `metadata.name`) with no
    /// pod: the pod is gone.
    pub async fn pod_gone(&self, name: &str) {
        Mock::given(method("GET"))
            .and(path(collection("pods")))
            .and(query_param(
                "fieldSelector",
                format!("metadata.name={name}"),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "apiVersion": "v1",
                "kind": "List",
                "metadata": { "resourceVersion": "2" },
                "items": [],
            })))
            .expect(1..)
            .mount(&self.server)
            .await;
    }

    /// Answer the wait on pod `pod` with the pod still there, and a watch
    /// that never reports its deletion in time.
    pub async fn pod_stays(&self, pod: Value) {
        let name = pod["metadata"]["name"]
            .as_str()
            .expect("pod name")
            .to_string();
        let field = format!("metadata.name={name}");
        Mock::given(method("GET"))
            .and(path(collection("pods")))
            .and(query_param("fieldSelector", field.as_str()))
            .and(query_param_is_missing("watch"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "apiVersion": "v1",
                "kind": "List",
                "metadata": { "resourceVersion": "2" },
                "items": [pod],
            })))
            .mount(&self.server)
            .await;
        Mock::given(method("GET"))
            .and(path(collection("pods")))
            .and(query_param("fieldSelector", field.as_str()))
            .and(query_param("watch", "true"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
            .mount(&self.server)
            .await;
    }

    /// Expect exactly `times` deletes of `<kind>/<name>`.
    pub async fn expect_delete(&self, kind: &str, name: &str, times: u64) {
        Mock::given(method("DELETE"))
            .and(path(format!("{}/{name}", collection(kind))))
            .respond_with(ResponseTemplate::new(200).set_body_json(status_success()))
            .expect(times)
            .mount(&self.server)
            .await;
    }

    /// Expect exactly `times` deletes of Job `name` with `Background`
    /// propagation, so its pods go with it.
    pub async fn expect_background_delete(&self, name: &str, times: u64) {
        Mock::given(method("DELETE"))
            .and(path(format!("{}/{name}", collection("jobs"))))
            .and(body_partial_json(
                json!({ "propagationPolicy": "Background" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(status_success()))
            .expect(times)
            .mount(&self.server)
            .await;
    }
}

fn status_success() -> Value {
    json!({ "apiVersion": "v1", "kind": "Status", "status": "Success", "metadata": {} })
}

/// Metadata with the ironflow labels of `component`, extra labels, and an
/// optional expiry.
fn metadata(name: &str, component: &str, extra: &[(&str, &str)], expires: Option<u64>) -> Value {
    let mut labels = json!({ LABEL_MANAGED_BY: "ironflow", LABEL_COMPONENT: component });
    for (key, value) in extra {
        labels[*key] = json!(value);
    }
    let mut meta = json!({
        "name": name,
        "namespace": NS,
        "uid": format!("uid-{name}"),
        "labels": labels,
    });
    if let Some(expires) = expires {
        meta["annotations"] = json!({ LABEL_EXPIRES_AT: expires.to_string() });
    }
    meta
}

/// A pod of `component` in `phase` (`reason` for a failed pod).
pub fn pod(
    name: &str,
    component: &str,
    extra: &[(&str, &str)],
    phase: &str,
    reason: Option<&str>,
    expires: Option<u64>,
) -> Value {
    let mut pod = json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": metadata(name, component, extra, expires),
        "spec": { "containers": [] },
        "status": { "phase": phase },
    });
    if let Some(reason) = reason {
        pod["status"]["reason"] = json!(reason);
    }
    pod
}

/// Mark `pod` as controlled by Job `job`.
pub fn owned_by_job(mut pod: Value, job: &str) -> Value {
    pod["metadata"]["ownerReferences"] = json!([{
        "apiVersion": "batch/v1",
        "kind": "Job",
        "name": job,
        "uid": format!("uid-{job}"),
        "controller": true,
    }]);
    pod
}

/// A Job of `JobRun`, failed with `failed_reason` when set.
pub fn job(
    name: &str,
    extra: &[(&str, &str)],
    failed_reason: Option<&str>,
    expires: Option<u64>,
) -> Value {
    let mut job = json!({
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": metadata(name, "job-run", extra, expires),
        "spec": { "template": { "spec": { "containers": [] } } },
        "status": {},
    });
    if let Some(reason) = failed_reason {
        job["status"]["conditions"] = json!([{
            "type": "Failed",
            "status": "True",
            "reason": reason,
        }]);
    }
    job
}

/// A prompt ConfigMap.
pub fn prompt_configmap(name: &str, extra: &[(&str, &str)], expires: Option<u64>) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": metadata(name, "prompt-data", extra, expires),
        "data": { "prompt": "hi" },
    })
}

/// A persistent environment claim, marked as being deleted when `deleting`.
pub fn environment_claim(name: &str, expires: Option<u64>, deleting: bool) -> Value {
    let mut claim = json!({
        "apiVersion": "v1",
        "kind": "PersistentVolumeClaim",
        "metadata": metadata(name, "environment", &[], expires),
        "spec": {
            "accessModes": ["ReadWriteOnce"],
            "resources": { "requests": { "storage": "1Gi" } },
        },
    });
    if deleting {
        claim["metadata"]["deletionTimestamp"] = json!("2026-10-05T12:00:00Z");
    }
    claim
}
