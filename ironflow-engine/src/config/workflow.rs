//! Configuration for workflow (sub-workflow) steps.

use std::collections::HashMap;
use std::ops::Not;

use ironflow_core::retry::RetryPolicy;
use ironflow_store::entities::{MAX_CONCURRENCY_KEY_LEN, validate_priority};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Prefix of the run labels the engine sets itself on a child run.
const ENGINE_LABEL_PREFIX: &str = "ironflow.io/";

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
    /// Queue priority of the child run.
    ///
    /// Set through [`WorkflowOptions::priority`] and recorded in the step
    /// input. `None` gives the child the priority of its own handler.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i16>,
    /// Tolerate a failed child: the step completes with the child's failure
    /// in its output instead of failing the parent.
    #[serde(default, skip_serializing_if = "Not::not")]
    pub allow_failure: bool,
    /// Labels set on the child run, over the ones it inherits from the
    /// parent run.
    ///
    /// Set through [`WorkflowOptions::label`] and recorded in the step input.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub labels: HashMap<String, String>,
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
            priority: None,
            allow_failure: false,
            labels: HashMap::new(),
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
/// [`workflow_with`](crate::context::WorkflowContext::workflow_with) or
/// [`workflows_with`](crate::context::WorkflowContext::workflows_with).
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
    priority: Option<i16>,
    step_name: Option<String>,
    labels: HashMap<String, String>,
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

    /// Set the queue priority of the child run, overriding the priority of
    /// the child handler.
    ///
    /// The child executes inside its parent's worker; the priority is
    /// recorded on the child run and orders it whenever it waits in the queue.
    ///
    /// # Panics
    ///
    /// Panics if `priority` is outside
    /// [`MIN_PRIORITY`](ironflow_store::entities::MIN_PRIORITY)`..=`[`MAX_PRIORITY`](ironflow_store::entities::MAX_PRIORITY).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::WorkflowOptions;
    ///
    /// let options = WorkflowOptions::new().priority(20);
    /// assert_eq!(options.priority_ref(), Some(20));
    /// ```
    ///
    /// ```should_panic
    /// use ironflow_engine::config::WorkflowOptions;
    ///
    /// WorkflowOptions::new().priority(101);
    /// ```
    pub fn priority(mut self, priority: i16) -> Self {
        if let Err(message) = validate_priority(priority) {
            panic!("{message}");
        }
        self.priority = Some(priority);
        self
    }

    /// The priority of the child run, if one was set.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::WorkflowOptions;
    ///
    /// assert_eq!(WorkflowOptions::new().priority_ref(), None);
    /// assert_eq!(WorkflowOptions::new().priority(-5).priority_ref(), Some(-5));
    /// ```
    pub fn priority_ref(&self) -> Option<i16> {
        self.priority
    }

    /// Name the `Workflow` step of the parent run.
    ///
    /// The step is named after the child handler by default. The name is what
    /// the run timeline shows and what identifies the step on resume: a
    /// handler that delegates to the same child several times gives each call
    /// its own name.
    ///
    /// # Panics
    ///
    /// Panics if `name` is empty or only whitespace.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::WorkflowOptions;
    ///
    /// let options = WorkflowOptions::new().step_name("delegate-1-alice");
    /// assert_eq!(options.step_name_ref(), Some("delegate-1-alice"));
    /// ```
    ///
    /// ```should_panic
    /// use ironflow_engine::config::WorkflowOptions;
    ///
    /// WorkflowOptions::new().step_name(" ");
    /// ```
    pub fn step_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        assert!(!name.trim().is_empty(), "step name must not be empty");
        self.step_name = Some(name);
        self
    }

    /// The step name, if one was set.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::WorkflowOptions;
    ///
    /// assert_eq!(WorkflowOptions::new().step_name_ref(), None);
    /// ```
    pub fn step_name_ref(&self) -> Option<&str> {
        self.step_name.as_deref()
    }

    /// Set a label on the child run.
    ///
    /// The child inherits the labels of its parent run; a label set here
    /// replaces an inherited one with the same key. The last value set for a
    /// key wins.
    ///
    /// # Panics
    ///
    /// Panics if `key` starts with `ironflow.io/`: those labels link the child
    /// to its parent and root runs and are set by the engine.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::WorkflowOptions;
    ///
    /// let options = WorkflowOptions::new().label("paperflow.io/agent", "alice");
    /// assert_eq!(options.labels_ref()["paperflow.io/agent"], "alice");
    /// ```
    ///
    /// ```should_panic
    /// use ironflow_engine::config::WorkflowOptions;
    ///
    /// WorkflowOptions::new().label("ironflow.io/parent-run-id", "x");
    /// ```
    pub fn label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        assert!(
            !key.starts_with(ENGINE_LABEL_PREFIX),
            "label {key:?} is reserved for the engine"
        );
        self.labels.insert(key, value.into());
        self
    }

    /// Set several labels on the child run, as [`label`](Self::label) does
    /// for each pair in order.
    ///
    /// # Panics
    ///
    /// Panics if a key starts with `ironflow.io/`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::WorkflowOptions;
    ///
    /// let options = WorkflowOptions::new().labels([("tour", "1"), ("agent", "alice")]);
    /// assert_eq!(options.labels_ref().len(), 2);
    /// ```
    pub fn labels<K, V>(self, labels: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        labels
            .into_iter()
            .fold(self, |options, (key, value)| options.label(key, value))
    }

    /// The labels set on the child run.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::WorkflowOptions;
    ///
    /// assert!(WorkflowOptions::new().labels_ref().is_empty());
    /// ```
    pub fn labels_ref(&self) -> &HashMap<String, String> {
        &self.labels
    }

    /// Consume the options and build the step config for `workflow_name`,
    /// with the name of the step: the one set, otherwise the handler name.
    pub(crate) fn into_step_config(
        self,
        workflow_name: &str,
        payload: Value,
    ) -> (String, WorkflowStepConfig) {
        let step_name = self.step_name.unwrap_or_else(|| workflow_name.to_string());
        let config = WorkflowStepConfig {
            concurrency_key: self.concurrency_key,
            priority: self.priority,
            allow_failure: self.allow_failure,
            labels: self.labels,
            ..WorkflowStepConfig::new(workflow_name, payload)
        };
        (step_name, config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironflow_store::entities::{MAX_PRIORITY, MIN_PRIORITY};
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
        let (_, config) = options.into_step_config("build", json!({}));
        assert_eq!(config.concurrency_key.as_deref(), Some("issue:12"));
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
    fn options_priority_is_carried_and_split() {
        let options = WorkflowOptions::new()
            .concurrency_key("issue:12")
            .priority(MAX_PRIORITY);
        assert_eq!(options.priority_ref(), Some(MAX_PRIORITY));
        let (_, config) = options.into_step_config("build", json!({}));
        assert_eq!(config.concurrency_key.as_deref(), Some("issue:12"));
        assert_eq!(config.priority, Some(MAX_PRIORITY));
        assert_eq!(
            WorkflowOptions::new().priority(MIN_PRIORITY).priority_ref(),
            Some(MIN_PRIORITY)
        );
        assert!(WorkflowOptions::new().priority_ref().is_none());
    }

    #[test]
    #[should_panic(expected = "priority must be between -100 and 100")]
    fn options_priority_rejects_above_the_maximum() {
        let _ = WorkflowOptions::new().priority(MAX_PRIORITY + 1);
    }

    #[test]
    #[should_panic(expected = "priority must be between -100 and 100")]
    fn options_priority_rejects_below_the_minimum() {
        let _ = WorkflowOptions::new().priority(MIN_PRIORITY - 1);
    }

    #[test]
    fn step_config_priority_is_omitted_when_unset_and_round_trips() {
        let config = WorkflowStepConfig::new("build", json!({}));
        assert!(
            to_value(&config)
                .expect("serialize")
                .get("priority")
                .is_none()
        );

        let mut config = WorkflowStepConfig::new("build", json!({}));
        config.priority = Some(-30);
        let back: WorkflowStepConfig =
            from_str(&to_string(&config).expect("serialize")).expect("deserialize");
        assert_eq!(back.priority, Some(-30));
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
    fn options_carry_the_step_name() {
        let options = WorkflowOptions::new().step_name("delegate-1-alice");
        assert_eq!(options.step_name_ref(), Some("delegate-1-alice"));
        assert!(WorkflowOptions::new().step_name_ref().is_none());
    }

    #[test]
    #[should_panic(expected = "step name must not be empty")]
    fn options_reject_an_empty_step_name() {
        let _ = WorkflowOptions::new().step_name("");
    }

    #[test]
    #[should_panic(expected = "step name must not be empty")]
    fn options_reject_a_blank_step_name() {
        let _ = WorkflowOptions::new().step_name(" \n ");
    }

    #[test]
    fn options_merge_labels_and_the_last_value_wins() {
        let options = WorkflowOptions::new()
            .label("paperflow.io/tour", "1")
            .labels([("paperflow.io/agent", "alice"), ("paperflow.io/tour", "2")]);
        let labels = options.labels_ref();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels["paperflow.io/tour"], "2");
        assert_eq!(labels["paperflow.io/agent"], "alice");
        assert!(WorkflowOptions::new().labels_ref().is_empty());
    }

    #[test]
    #[should_panic(expected = "reserved for the engine")]
    fn options_reject_an_engine_label() {
        let _ = WorkflowOptions::new().label("ironflow.io/parent-run-id", "x");
    }

    #[test]
    #[should_panic(expected = "reserved for the engine")]
    fn options_reject_an_engine_label_among_several() {
        let _ = WorkflowOptions::new().labels([("team", "a"), ("ironflow.io/root-run-id", "x")]);
    }

    #[test]
    fn step_config_defaults_the_step_name_to_the_handler() {
        let (name, config) = WorkflowOptions::new().into_step_config("review", json!({}));
        assert_eq!(name, "review");
        assert_eq!(config.workflow_name, "review");
        assert!(config.labels.is_empty());
    }

    #[test]
    fn step_config_takes_every_option() {
        let (name, config) = WorkflowOptions::new()
            .step_name("delegate-1-alice")
            .label("agent", "alice")
            .concurrency_key("issue:12")
            .priority(7)
            .allow_failure()
            .into_step_config("review", json!({"n": 1}));
        assert_eq!(name, "delegate-1-alice");
        assert_eq!(config.workflow_name, "review");
        assert_eq!(config.payload, json!({"n": 1}));
        assert_eq!(config.labels["agent"], "alice");
        assert_eq!(config.concurrency_key.as_deref(), Some("issue:12"));
        assert_eq!(config.priority, Some(7));
        assert!(config.allow_failure);
    }

    #[test]
    fn step_config_labels_are_omitted_when_empty_and_round_trip() {
        let config = WorkflowStepConfig::new("build", json!({}));
        assert!(
            to_value(&config)
                .expect("serialize")
                .get("labels")
                .is_none()
        );

        let (_, config) = WorkflowOptions::new()
            .label("agent", "alice")
            .into_step_config("build", json!({}));
        let back: WorkflowStepConfig =
            from_str(&to_string(&config).expect("serialize")).expect("deserialize");
        assert_eq!(back.labels["agent"], "alice");
    }

    #[test]
    fn a_config_predating_labels_still_deserializes() {
        let config: WorkflowStepConfig =
            from_str(r#"{"workflow_name":"build","payload":{}}"#).expect("deserialize");
        assert!(config.labels.is_empty());
    }

    #[test]
    fn retry_policy_roundtrip() {
        let config = WorkflowStepConfig::new("deploy", json!({})).retry_policy(RetryPolicy::new(3));
        let json = serde_json::to_string(&config).expect("serialize");
        let back: WorkflowStepConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.retry.as_ref().unwrap().max_retries(), 3);
    }
}
