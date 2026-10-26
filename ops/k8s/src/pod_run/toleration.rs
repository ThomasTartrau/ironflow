//! Conversion of an [`ironflow_core`] [`K8sToleration`] into the pod-spec [`Toleration`].

use ironflow_core::providers::claude::K8sToleration;
use k8s_openapi::api::core::v1::Toleration;

/// Convert a parsed [`K8sToleration`] into the `k8s_openapi` type.
///
/// An inherent conversion rather than a `From` impl: both types are foreign to
/// this crate.
pub(crate) fn to_k8s_toleration(t: &K8sToleration) -> Toleration {
    Toleration {
        key: Some(t.key.clone()),
        operator: Some(t.operator.as_str().to_string()),
        value: t.value.clone(),
        effect: Some(t.effect.as_str().to_string()),
        toleration_seconds: t.toleration_seconds,
    }
}
