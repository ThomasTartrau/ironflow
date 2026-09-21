//! Serializable pod toleration used by the Kubernetes transport providers.
//!
//! [`K8sToleration`] is exposed instead of
//! `k8s_openapi::api::core::v1::Toleration` to keep `k8s_openapi` out of the
//! core's public API surface, and to give callers a type whose `key`, `operator`
//! and `effect` are required rather than all-optional. The providers build their
//! pod spec as [`serde_json`], into which this type serializes directly.

use serde::Serialize;

/// Match operator for a [`K8sToleration`].
///
/// The variants serialize to the exact PascalCase strings the pod spec expects
/// (`"Equal"`, `"Exists"`), so no serde rename is needed.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::claude::TolerationOperator;
///
/// assert_eq!(
///     serde_json::to_value(TolerationOperator::Equal).unwrap(),
///     serde_json::json!("Equal")
/// );
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum TolerationOperator {
    /// Key and value must both match the taint.
    Equal,
    /// Key must exist; the toleration's `value` is ignored.
    Exists,
}

/// Taint effect a [`K8sToleration`] applies to.
///
/// The variants serialize to the exact PascalCase strings the pod spec expects
/// (`"NoSchedule"`, `"PreferNoSchedule"`, `"NoExecute"`), so no serde rename is
/// needed.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::claude::TolerationEffect;
///
/// assert_eq!(
///     serde_json::to_value(TolerationEffect::NoSchedule).unwrap(),
///     serde_json::json!("NoSchedule")
/// );
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum TolerationEffect {
    /// Do not schedule new pods that lack the toleration onto the tainted node.
    NoSchedule,
    /// Avoid scheduling onto the tainted node when possible, but do not forbid it.
    PreferNoSchedule,
    /// Evict running pods that do not tolerate the taint.
    NoExecute,
}

/// A Kubernetes pod toleration, letting a pod schedule onto tainted nodes.
///
/// A pod targeting a node with a `NoSchedule` taint (e.g. a dedicated worker
/// node carrying `dedicated=worker:NoSchedule`) stays `Pending` forever unless
/// it carries a matching toleration. This serializes to the camelCase pod-spec
/// shape; `value` and `tolerationSeconds` are omitted when `None`.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::claude::{K8sToleration, TolerationEffect, TolerationOperator};
///
/// let toleration = K8sToleration {
///     key: "dedicated".to_string(),
///     operator: TolerationOperator::Equal,
///     value: Some("worker".to_string()),
///     effect: TolerationEffect::NoSchedule,
///     toleration_seconds: None,
/// };
/// assert_eq!(toleration.key, "dedicated");
/// ```
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sToleration {
    /// Taint key the toleration matches (e.g. `"dedicated"`).
    pub key: String,
    /// Match operator: [`TolerationOperator::Equal`] (key and value must match)
    /// or [`TolerationOperator::Exists`] (key must exist, `value` ignored).
    pub operator: TolerationOperator,
    /// Taint value to match. Required for [`TolerationOperator::Equal`], omitted
    /// for [`TolerationOperator::Exists`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Taint effect the toleration applies to. See [`TolerationEffect`].
    pub effect: TolerationEffect,
    /// For a [`TolerationEffect::NoExecute`] taint, how long the pod stays bound
    /// after the taint is added, in seconds. `None` means bound forever.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toleration_seconds: Option<i64>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{json, to_value};

    use super::{K8sToleration, TolerationEffect, TolerationOperator};
    use crate::providers::claude::k8s::common::{
        DEFAULT_INPUT_INIT_IMAGE, ImagePullPolicy, K8sResources, PodConfig, build_pod_spec,
    };

    /// Build a `PodConfig` carrying `tolerations`, everything else empty/default.
    ///
    /// Takes the borrowed dependencies as arguments so they outlive the returned
    /// config (they cannot be temporaries owned by the builder).
    fn config_with<'a>(
        tolerations: &'a [K8sToleration],
        resources: &'a K8sResources,
        pull_policy: &'a ImagePullPolicy,
        node_selector: &'a BTreeMap<String, String>,
        extra_labels: &'a BTreeMap<String, String>,
    ) -> PodConfig<'a> {
        PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources,
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: pull_policy,
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels,
            node_selector,
            tolerations,
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
        }
    }

    #[test]
    fn operator_and_effect_serialize_to_pascal_case() {
        assert_eq!(to_value(TolerationOperator::Equal).unwrap(), json!("Equal"));
        assert_eq!(
            to_value(TolerationOperator::Exists).unwrap(),
            json!("Exists")
        );
        assert_eq!(
            to_value(TolerationEffect::NoSchedule).unwrap(),
            json!("NoSchedule")
        );
        assert_eq!(
            to_value(TolerationEffect::PreferNoSchedule).unwrap(),
            json!("PreferNoSchedule")
        );
        assert_eq!(
            to_value(TolerationEffect::NoExecute).unwrap(),
            json!("NoExecute")
        );
    }

    #[test]
    fn serializes_camel_case_and_omits_none_fields() {
        // Equal toleration with no toleration_seconds: value present, the
        // NoExecute-only field absent, key renamed to camelCase.
        let value = to_value(K8sToleration {
            key: "dedicated".to_string(),
            operator: TolerationOperator::Equal,
            value: Some("worker".to_string()),
            effect: TolerationEffect::NoSchedule,
            toleration_seconds: None,
        })
        .unwrap();
        assert_eq!(
            value,
            json!({
                "key": "dedicated",
                "operator": "Equal",
                "value": "worker",
                "effect": "NoSchedule"
            })
        );
        assert!(value.get("tolerationSeconds").is_none());
    }

    #[test]
    fn serializes_exists_operator_omits_value() {
        // Exists toleration: value is None so the key must not appear at all.
        let value = to_value(K8sToleration {
            key: "gpu".to_string(),
            operator: TolerationOperator::Exists,
            value: None,
            effect: TolerationEffect::NoExecute,
            toleration_seconds: Some(30),
        })
        .unwrap();
        assert!(value.get("value").is_none());
        assert_eq!(value["tolerationSeconds"], 30);
    }

    #[test]
    fn build_pod_spec_with_tolerations() {
        let tolerations = vec![K8sToleration {
            key: "dedicated".to_string(),
            operator: TolerationOperator::Equal,
            value: Some("worker".to_string()),
            effect: TolerationEffect::NoSchedule,
            toleration_seconds: None,
        }];
        let resources = K8sResources::default();
        let pull_policy = ImagePullPolicy::default();
        let empty = BTreeMap::new();
        let pod = build_pod_spec(&config_with(
            &tolerations,
            &resources,
            &pull_policy,
            &empty,
            &empty,
        ))
        .unwrap();
        let tolerations = pod.spec.unwrap().tolerations.expect("tolerations present");
        assert_eq!(tolerations.len(), 1);
        let t = &tolerations[0];
        assert_eq!(t.key.as_deref(), Some("dedicated"));
        assert_eq!(t.operator.as_deref(), Some("Equal"));
        assert_eq!(t.value.as_deref(), Some("worker"));
        assert_eq!(t.effect.as_deref(), Some("NoSchedule"));
    }

    #[test]
    fn build_pod_spec_without_tolerations_omits_key() {
        let resources = K8sResources::default();
        let pull_policy = ImagePullPolicy::default();
        let empty = BTreeMap::new();
        let pod =
            build_pod_spec(&config_with(&[], &resources, &pull_policy, &empty, &empty)).unwrap();
        assert!(
            pod.spec.unwrap().tolerations.is_none(),
            "no tolerations key when the slice is empty"
        );
    }
}
