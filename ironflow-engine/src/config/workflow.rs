//! Configuration for workflow (sub-workflow) steps.

use std::ops::Not;

use ironflow_core::retry::RetryPolicy;
use ironflow_store::entities::MAX_CONCURRENCY_KEY_LEN;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Configuration for invoking a registered workflow as a sub-step.
///
/// The engine will look up the handler by [`workflow_name`](WorkflowStepConfig::workflow_name)
/// and execute it as a child run with its own steps and lifecycle.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::WorkflowStepConfig;
/// use serde_json::json;
///
/// let config = WorkflowStepConfig::new("build", json!({"branch": "main"}));
/// assert_eq!(config.workflow_name, "build");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStepConfig {
    /// Name of the registered workflow handler to invoke.
    pub workflow_name: String,
    /// Payload to pass to the child workflow run.
    pub payload: Value,
    /// Optional step-level retry policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,
    /// Concurrency key held by the child run while it is not terminal.
    ///
    /// Set through [`WorkflowOptions::concurrency_key`] and recorded in the
    /// step input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency_key: Option<String>,
    /// Tolerate a failed child: the step completes with the child's failure
    /// in its output instead of failing the parent.
    #[serde(default, skip_serializing_if = "Not::not")]
    pub allow_failure: bool,
}

impl WorkflowStepConfig {
    /// Create a new workflow step config.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::WorkflowStepConfig;
    /// use serde_json::json;
    ///
    /// let config = WorkflowStepConfig::new("deploy", json!({}));
    /// assert_eq!(config.workflow_name, "deploy");
    /// ```
    pub fn new(workflow_name: &str, payload: Value) -> Self {
        Self {
            workflow_name: workflow_name.to_string(),
            payload,
            retry: None,
            concurrency_key: None,
            allow_failure: false,
        }
    }

    /// Set a step-level retry policy.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::retry::RetryPolicy;
    /// use ironflow_engine::config::WorkflowStepConfig;
    /// use serde_json::json;
    ///
    /// let config = WorkflowStepConfig::new("build", json!({}))
    ///     .retry_policy(RetryPolicy::new(3));
    /// assert!(config.retry.is_some());
    /// ```
    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry = Some(policy);
        self
    }

    /// Tolerate a failed child run: the step completes with a
    /// [`SubWorkflowOutput`](crate::executor::SubWorkflowOutput) reporting the
    /// failure and the parent run ends as `Warning`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::WorkflowStepConfig;
    /// use serde_json::json;
    ///
    /// let config = WorkflowStepConfig::new("build", json!({})).allow_failure();
    /// assert!(config.allow_failure);
    /// ```
    pub fn allow_failure(mut self) -> Self {
        self.allow_failure = true;
        self
    }
}

/// Options of a sub-workflow step started with
/// [`workflow_with`](crate::context::WorkflowContext::workflow_with).
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::WorkflowOptions;
///
/// let options = WorkflowOptions::new().allow_failure();
/// assert!(options.allow_failure);
/// assert!(!WorkflowOptions::default().allow_failure);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkflowOptions {
    /// Tolerate a failed child run instead of failing the parent.
    pub allow_failure: bool,
    concurrency_key: Option<String>,
}

impl WorkflowOptions {
    /// Create options with every flag off.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::WorkflowOptions;
    ///
    /// assert_eq!(WorkflowOptions::new(), WorkflowOptions::default());
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Tolerate a failed child run: the step completes with the failure in its
    /// output and the parent run ends as `Warning`. A suspension is never
    /// tolerated.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::WorkflowOptions;
    ///
    /// assert!(WorkflowOptions::new().allow_failure().allow_failure);
    /// ```
    pub fn allow_failure(mut self) -> Self {
        self.allow_failure = true;
        self
    }

    /// Make the child run exclusive on `key`.
    ///
    /// While another non-terminal run (Pending, Running, Retrying, Sleeping,
    /// AwaitingApproval) holds the same key, no child run is created and the
    /// step completes with a
    /// [`SubWorkflowOutcome::Conflict`](crate::executor::SubWorkflowOutcome::Conflict).
    ///
    /// # Panics
    ///
    /// Panics if `key` is empty or only whitespace, or longer than
    /// [`MAX_CONCURRENCY_KEY_LEN`] bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::WorkflowOptions;
    ///
    /// let options = WorkflowOptions::new().concurrency_key("issue:12");
    /// assert_eq!(options.concurrency_key_ref(), Some("issue:12"));
    /// ```
    ///
    /// ```should_panic
    /// use ironflow_engine::config::WorkflowOptions;
    ///
    /// WorkflowOptions::new().concurrency_key("  ");
    /// ```
    pub fn concurrency_key(mut self, key: impl Into<String>) -> Self {
        let key = key.into();
        assert!(!key.trim().is_empty(), "concurrency key must not be empty");
        assert!(
            key.len() <= MAX_CONCURRENCY_KEY_LEN,
            "concurrency key must be at most {MAX_CONCURRENCY_KEY_LEN} bytes"
        );
        self.concurrency_key = Some(key);
        self
    }

    /// The concurrency key, if one was set.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::WorkflowOptions;
    ///
    /// let options = WorkflowOptions::new().concurrency_key("deploy:prod");
    /// assert_eq!(options.concurrency_key_ref(), Some("deploy:prod"));
    /// ```
    pub fn concurrency_key_ref(&self) -> Option<&str> {
        self.concurrency_key.as_deref()
    }

    /// Consume the options and return the concurrency key.
    pub(crate) fn into_concurrency_key(self) -> Option<String> {
        self.concurrency_key
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{from_str, json, to_string, to_value};

    #[test]
    fn new_sets_fields() {
        let config = WorkflowStepConfig::new("build", json!({"key": "val"}));
        assert_eq!(config.workflow_name, "build");
        assert_eq!(config.payload["key"], "val");
    }

    #[test]
    fn serde_roundtrip() {
        let config = WorkflowStepConfig::new("deploy", json!({"env": "prod"}));
        let json = serde_json::to_string(&config).unwrap();
        let back: WorkflowStepConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.workflow_name, "deploy");
        assert_eq!(back.payload["env"], "prod");
    }

    #[test]
    fn a_config_predating_retry_still_deserializes() {
        let config: WorkflowStepConfig =
            serde_json::from_str(r#"{"workflow_name":"build","payload":{"key":"val"}}"#)
                .expect("deserialize");
        assert!(config.retry.is_none());
    }

    #[test]
    fn a_config_without_concurrency_key_omits_it() {
        let config = WorkflowStepConfig::new("build", json!({}));
        let value = to_value(&config).expect("serialize");
        assert!(value.get("concurrency_key").is_none());
    }

    #[test]
    fn concurrency_key_roundtrip() {
        let mut config = WorkflowStepConfig::new("build", json!({}));
        config.concurrency_key = Some("issue:12".to_string());
        let json = to_string(&config).expect("serialize");
        let back: WorkflowStepConfig = from_str(&json).expect("deserialize");
        assert_eq!(back.concurrency_key.as_deref(), Some("issue:12"));
    }

    #[test]
    fn options_carry_the_concurrency_key() {
        let options = WorkflowOptions::new().concurrency_key("issue:12");
        assert_eq!(options.concurrency_key_ref(), Some("issue:12"));
        assert_eq!(options.into_concurrency_key().as_deref(), Some("issue:12"));
        assert!(WorkflowOptions::new().concurrency_key_ref().is_none());
    }

    #[test]
    fn options_accept_a_key_at_the_length_limit() {
        let key = "a".repeat(MAX_CONCURRENCY_KEY_LEN);
        let options = WorkflowOptions::new().concurrency_key(key.clone());
        assert_eq!(options.concurrency_key_ref(), Some(key.as_str()));
    }

    #[test]
    #[should_panic(expected = "concurrency key must not be empty")]
    fn options_reject_an_empty_key() {
        let _ = WorkflowOptions::new().concurrency_key("");
    }

    #[test]
    #[should_panic(expected = "concurrency key must not be empty")]
    fn options_reject_a_blank_key() {
        let _ = WorkflowOptions::new().concurrency_key(" \t ");
    }

    #[test]
    #[should_panic(expected = "concurrency key must be at most")]
    fn options_reject_a_key_over_the_limit() {
        let _ = WorkflowOptions::new().concurrency_key("a".repeat(MAX_CONCURRENCY_KEY_LEN + 1));
    }

    #[test]
    fn allow_failure_defaults_to_false() {
        let config = WorkflowStepConfig::new("build", json!({}));
        assert!(!config.allow_failure);
    }

    #[test]
    fn allow_failure_is_omitted_from_json_when_false() {
        let config = WorkflowStepConfig::new("build", json!({}));
        let value = serde_json::to_value(&config).expect("serialize");
        assert!(value.get("allow_failure").is_none());
    }

    #[test]
    fn allow_failure_roundtrip() {
        let config = WorkflowStepConfig::new("build", json!({})).allow_failure();
        let json = serde_json::to_string(&config).expect("serialize");
        let back: WorkflowStepConfig = serde_json::from_str(&json).expect("deserialize");
        assert!(back.allow_failure);
    }

    #[test]
    fn options_builder_sets_allow_failure() {
        assert!(!WorkflowOptions::new().allow_failure);
        assert!(WorkflowOptions::new().allow_failure().allow_failure);
    }

    #[test]
    fn retry_policy_roundtrip() {
        let config = WorkflowStepConfig::new("deploy", json!({})).retry_policy(RetryPolicy::new(3));
        let json = serde_json::to_string(&config).expect("serialize");
        let back: WorkflowStepConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.retry.as_ref().unwrap().max_retries(), 3);
    }
}
