//! Reserved labels (`app.kubernetes.io/managed-by`, `app.kubernetes.io/component`):
//! refused from the provider builders and from the step, never overwritten
//! in silence.

use std::collections::BTreeMap;

use crate::provider::{AgentConfig, LABEL_COMPONENT, LABEL_MANAGED_BY, LABEL_RUN_ID};

use super::K8sEphemeralProvider;

#[test]
#[should_panic(expected = "pod label 'app.kubernetes.io/component' is reserved")]
fn k8s_reserved_provider_pod_label_panics() {
    let _ = K8sEphemeralProvider::sandboxed("img:v1").pod_label(LABEL_COMPONENT, "agent");
}

#[test]
#[should_panic(expected = "pod label 'app.kubernetes.io/managed-by' is reserved")]
fn k8s_reserved_provider_pod_labels_map_panics() {
    let labels = BTreeMap::from([
        ("team".to_string(), "infra".to_string()),
        (LABEL_MANAGED_BY.to_string(), "helm".to_string()),
    ]);
    let _ = K8sEphemeralProvider::new("img:v1").pod_labels(labels);
}

#[test]
fn k8s_reserved_step_pod_label_is_an_invocation_error() {
    let provider = K8sEphemeralProvider::sandboxed("img:v1");
    let config = AgentConfig::new("hi").pod_label(LABEL_COMPONENT, "agent");
    let err = provider.merged_pod_inputs(&config).unwrap_err().to_string();
    assert!(
        err.contains("pod label 'app.kubernetes.io/component' is reserved"),
        "{err}"
    );
}

#[test]
fn k8s_reserved_labels_leave_other_labels_alone() {
    let provider = K8sEphemeralProvider::sandboxed("img:v1").pod_label("team", "infra");
    let config = AgentConfig::new("hi").pod_label(LABEL_RUN_ID, "run-1");
    let merged = provider.merged_pod_inputs(&config).unwrap();
    assert_eq!(merged.labels["team"], "infra");
    assert_eq!(merged.labels[LABEL_RUN_ID], "run-1");
}
