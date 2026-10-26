//! Serializable pod toleration used by the Kubernetes transport providers.
//!
//! [`K8sToleration`] is exposed instead of
//! `k8s_openapi::api::core::v1::Toleration` to keep `k8s_openapi` out of the
//! core's public API surface, and to give callers a type whose `key`, `operator`
//! and `effect` are required rather than all-optional. The providers build their
//! pod spec as [`serde_json`], into which this type serializes directly.

use std::str::FromStr;

use serde::Serialize;
use thiserror::Error;

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

impl TolerationOperator {
    /// The exact string the pod spec expects for this operator.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::claude::TolerationOperator;
    ///
    /// assert_eq!(TolerationOperator::Exists.as_str(), "Exists");
    /// ```
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Equal => "Equal",
            Self::Exists => "Exists",
        }
    }
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

impl TolerationEffect {
    /// The exact string the pod spec expects for this effect.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::claude::TolerationEffect;
    ///
    /// assert_eq!(TolerationEffect::NoExecute.as_str(), "NoExecute");
    /// ```
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoSchedule => "NoSchedule",
            Self::PreferNoSchedule => "PreferNoSchedule",
            Self::NoExecute => "NoExecute",
        }
    }
}

impl FromStr for TolerationEffect {
    type Err = TolerationParseError;

    /// Parse the exact PascalCase effect name (`NoSchedule`, `PreferNoSchedule`, `NoExecute`).
    ///
    /// # Errors
    ///
    /// Returns [`TolerationParseError::UnknownEffect`] for any other string.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::claude::TolerationEffect;
    ///
    /// assert_eq!("NoSchedule".parse::<TolerationEffect>(), Ok(TolerationEffect::NoSchedule));
    /// assert!("noschedule".parse::<TolerationEffect>().is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "NoSchedule" => Ok(Self::NoSchedule),
            "PreferNoSchedule" => Ok(Self::PreferNoSchedule),
            "NoExecute" => Ok(Self::NoExecute),
            other => Err(TolerationParseError::UnknownEffect(other.to_string())),
        }
    }
}

/// Error returned when parsing a `key[=value]:Effect` toleration fails.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::claude::{K8sToleration, TolerationParseError};
///
/// let result = "gpu".parse::<K8sToleration>();
/// assert_eq!(
///     result,
///     Err(TolerationParseError::MissingEffect("gpu".to_string()))
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TolerationParseError {
    /// The entry is blank.
    #[error("empty toleration entry")]
    Empty,
    /// The entry has no `:Effect` suffix; carries the offending entry.
    #[error("toleration `{0}` has no `:Effect` suffix")]
    MissingEffect(String),
    /// The taint key is empty; carries the offending entry.
    #[error("toleration `{0}` has an empty key")]
    EmptyKey(String),
    /// `key=` was written with no value; carries the offending entry.
    #[error("toleration `{0}` has an empty value after `=`")]
    EmptyValue(String),
    /// The effect is not one of `NoSchedule`, `PreferNoSchedule`, `NoExecute`; carries it.
    #[error("unknown toleration effect `{0}`")]
    UnknownEffect(String),
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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

impl K8sToleration {
    /// Parse a comma-separated list of `key[=value]:Effect` tolerations.
    ///
    /// A blank string yields an empty list. Any empty entry (e.g. a trailing
    /// comma) is an error.
    ///
    /// # Errors
    ///
    /// Returns the [`TolerationParseError`] of the first entry that fails to parse.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::claude::{K8sToleration, TolerationParseError};
    ///
    /// # fn main() -> Result<(), TolerationParseError> {
    /// let list = K8sToleration::parse_list("dedicated=worker:NoSchedule,gpu:NoExecute")?;
    /// assert_eq!(list.len(), 2);
    /// assert!(K8sToleration::parse_list("  ")?.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub fn parse_list(s: &str) -> Result<Vec<Self>, TolerationParseError> {
        if s.trim().is_empty() {
            return Ok(Vec::new());
        }
        s.split(',').map(str::parse).collect()
    }
}

impl FromStr for K8sToleration {
    type Err = TolerationParseError;

    /// Parse `key[=value]:Effect`: `key=value` is an `Equal` toleration, a bare
    /// `key` is an `Exists` toleration.
    ///
    /// # Errors
    ///
    /// Returns a [`TolerationParseError`] for a blank entry, a missing effect,
    /// an empty key or value, or an unknown effect.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::claude::{K8sToleration, TolerationOperator, TolerationParseError};
    ///
    /// # fn main() -> Result<(), TolerationParseError> {
    /// let t: K8sToleration = "dedicated=worker:NoSchedule".parse()?;
    /// assert_eq!(t.operator, TolerationOperator::Equal);
    /// assert_eq!(t.value.as_deref(), Some("worker"));
    /// # Ok(())
    /// # }
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let entry = s.trim();
        if entry.is_empty() {
            return Err(TolerationParseError::Empty);
        }
        let (head, effect) = entry
            .split_once(':')
            .ok_or_else(|| TolerationParseError::MissingEffect(entry.to_string()))?;
        let effect = effect.trim().parse::<TolerationEffect>()?;
        let (key, operator, value) = match head.split_once('=') {
            Some((key, value)) => {
                let value = value.trim();
                if value.is_empty() {
                    return Err(TolerationParseError::EmptyValue(entry.to_string()));
                }
                (
                    key.trim(),
                    TolerationOperator::Equal,
                    Some(value.to_string()),
                )
            }
            None => (head.trim(), TolerationOperator::Exists, None),
        };
        if key.is_empty() {
            return Err(TolerationParseError::EmptyKey(entry.to_string()));
        }
        Ok(Self {
            key: key.to_string(),
            operator,
            value,
            effect,
            toleration_seconds: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{json, to_value};

    use super::{K8sToleration, TolerationEffect, TolerationOperator, TolerationParseError};
    use crate::providers::claude::k8s::common::{
        DEFAULT_INPUT_INIT_IMAGE, ImagePullPolicy, K8sResources, PodConfig, PodHardening,
        build_pod_spec,
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
            hardening: PodHardening::default(),
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
    fn parse_list_accepts_equal_and_exists() {
        let list = K8sToleration::parse_list(
            " dedicated=worker:NoSchedule , gpu:NoExecute,spot:PreferNoSchedule",
        )
        .unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].key, "dedicated");
        assert_eq!(list[0].operator, TolerationOperator::Equal);
        assert_eq!(list[0].value.as_deref(), Some("worker"));
        assert_eq!(list[0].effect, TolerationEffect::NoSchedule);
        assert_eq!(list[1].key, "gpu");
        assert_eq!(list[1].operator, TolerationOperator::Exists);
        assert_eq!(list[1].value, None);
        assert_eq!(list[1].effect, TolerationEffect::NoExecute);
        assert_eq!(list[2].effect, TolerationEffect::PreferNoSchedule);
        assert!(list.iter().all(|t| t.toleration_seconds.is_none()));
    }

    #[test]
    fn parse_list_blank_is_empty() {
        assert!(K8sToleration::parse_list("").unwrap().is_empty());
        assert!(K8sToleration::parse_list("  \t ").unwrap().is_empty());
    }

    #[test]
    fn parse_rejects_invalid_toleration() {
        let err = |s: &str| s.parse::<K8sToleration>().unwrap_err();
        assert_eq!(err(""), TolerationParseError::Empty);
        assert_eq!(
            err("key"),
            TolerationParseError::MissingEffect("key".into())
        );
        assert_eq!(
            err("key=val"),
            TolerationParseError::MissingEffect("key=val".into())
        );
        assert_eq!(
            err(":NoSchedule"),
            TolerationParseError::EmptyKey(":NoSchedule".into())
        );
        assert_eq!(
            err("key=:NoSchedule"),
            TolerationParseError::EmptyValue("key=:NoSchedule".into())
        );
        assert_eq!(
            err("key:noschedule"),
            TolerationParseError::UnknownEffect("noschedule".into())
        );
        assert_eq!(
            err("key:Bogus"),
            TolerationParseError::UnknownEffect("Bogus".into())
        );
        assert_eq!(
            err("a=b:c:NoSchedule"),
            TolerationParseError::UnknownEffect("c:NoSchedule".into())
        );
        assert_eq!(
            K8sToleration::parse_list("a:NoSchedule,").unwrap_err(),
            TolerationParseError::Empty
        );
        assert_eq!(
            K8sToleration::parse_list("a:NoSchedule,,b:NoSchedule").unwrap_err(),
            TolerationParseError::Empty
        );
    }

    #[test]
    fn as_str_matches_serde_output() {
        for op in [TolerationOperator::Equal, TolerationOperator::Exists] {
            assert_eq!(to_value(op).unwrap(), json!(op.as_str()));
        }
        for effect in [
            TolerationEffect::NoSchedule,
            TolerationEffect::PreferNoSchedule,
            TolerationEffect::NoExecute,
        ] {
            assert_eq!(to_value(effect).unwrap(), json!(effect.as_str()));
            assert_eq!(effect.as_str().parse::<TolerationEffect>(), Ok(effect));
        }
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
