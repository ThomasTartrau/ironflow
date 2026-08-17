//! [`RunCreator`] trait and [`CreateRunOpts`] builder -- centralised run creation.
//!
//! [`RunCreator`] is a minimal trait with a single method: create a run from
//! a [`NewRun`]. It is intentionally thinner than [`RunStore`] so that any
//! store implementation can be used as a run creator through the blanket impl.
//!
//! [`CreateRunOpts`] is a builder for the optional fields of a run creation
//! request. Combined with [`WorkflowHandler::create_run`](crate::handler::WorkflowHandler::create_run), it assembles a
//! [`NewRun`] from the handler's own metadata, removing duplication across
//! call sites.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_engine::run_creator::{CreateRunOpts, RunCreator};
//! use ironflow_store::entities::TriggerKind;
//! use ironflow_store::memory::InMemoryStore;
//!
//! # async fn example() -> Result<(), ironflow_engine::error::EngineError> {
//! let store = InMemoryStore::new();
//! let creator: &dyn RunCreator = &store;
//!
//! let new_run = CreateRunOpts::new()
//!     .trigger(TriggerKind::Api)
//!     .build("deploy", Some("1.0.0"), None);
//!
//! let creation = creator.create_run(new_run).await?;
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

use ironflow_store::entities::{NewRun, RunActor, RunCreation, TriggerKind};
use ironflow_store::store::RunStore;

use crate::error::EngineError;

/// Future returned by [`RunCreator::create_run`].
pub type RunCreatorFuture<'a> =
    Pin<Box<dyn Future<Output = Result<RunCreation, EngineError>> + Send + 'a>>;

/// Minimal trait for creating workflow runs.
///
/// Automatically implemented for every [`RunStore`] via a blanket impl,
/// so any store (InMemory, Postgres, ApiRunStore) is a valid [`RunCreator`].
///
/// # Examples
///
/// ```no_run
/// use ironflow_engine::run_creator::RunCreator;
/// use ironflow_store::entities::{NewRun, TriggerKind};
///
/// # async fn example(creator: &dyn RunCreator) -> Result<(), ironflow_engine::error::EngineError> {
/// let new_run = NewRun {
///     workflow_name: "deploy".to_string(),
///     trigger: TriggerKind::Manual,
///     payload: serde_json::json!({}),
///     max_retries: 0,
///     handler_version: None,
///     labels: Default::default(),
///     scheduled_at: None,
///     created_by: None,
///     idempotency_key: None,
///     max_cost_usd: None,
/// };
/// let creation = creator.create_run(new_run).await?;
/// # Ok(())
/// # }
/// ```
pub trait RunCreator: Send + Sync {
    /// Create a new workflow run.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the run could not be created.
    fn create_run(&self, req: NewRun) -> RunCreatorFuture<'_>;
}

impl<T: RunStore + ?Sized> RunCreator for T {
    fn create_run(&self, req: NewRun) -> RunCreatorFuture<'_> {
        Box::pin(async move {
            RunStore::create_run(self, req)
                .await
                .map_err(EngineError::from)
        })
    }
}

/// Builder for optional run creation fields.
///
/// Builds a [`NewRun`] from handler metadata plus user-supplied overrides.
/// Use with [`WorkflowHandler::create_run`] to avoid duplicating workflow
/// name, version, and cost cap at every call site.
///
/// [`WorkflowHandler::create_run`]: crate::handler::WorkflowHandler::create_run
///
/// # Examples
///
/// ```
/// use ironflow_engine::run_creator::CreateRunOpts;
/// use ironflow_store::entities::TriggerKind;
/// use serde_json::json;
///
/// let new_run = CreateRunOpts::new()
///     .trigger(TriggerKind::Webhook { path: "/hooks/gh".to_string() })
///     .payload(json!({"ref": "main"}))
///     .max_retries(3)
///     .build("deploy", Some("2.0.0"), None);
///
/// assert_eq!(new_run.workflow_name, "deploy");
/// assert_eq!(new_run.max_retries, 3);
/// assert_eq!(new_run.handler_version, Some("2.0.0".to_string()));
/// ```
#[derive(Debug, Clone, Default)]
pub struct CreateRunOpts {
    trigger: Option<TriggerKind>,
    payload: Option<Value>,
    max_retries: Option<u32>,
    scheduled_at: Option<DateTime<Utc>>,
    created_by: Option<RunActor>,
    idempotency_key: Option<String>,
    labels: Option<HashMap<String, String>>,
    max_cost_usd: Option<Decimal>,
}

impl CreateRunOpts {
    /// Create a new builder with all fields unset.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::run_creator::CreateRunOpts;
    ///
    /// let opts = CreateRunOpts::new();
    /// let new_run = opts.build("my-workflow", None, None);
    /// assert_eq!(new_run.workflow_name, "my-workflow");
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Set how the run was triggered.
    ///
    /// Defaults to [`TriggerKind::Manual`] if not set.
    pub fn trigger(mut self, trigger: TriggerKind) -> Self {
        self.trigger = Some(trigger);
        self
    }

    /// Set the trigger-specific payload.
    ///
    /// Defaults to `json!({})` if not set.
    pub fn payload(mut self, payload: Value) -> Self {
        self.payload = Some(payload);
        self
    }

    /// Set the maximum retry attempts.
    ///
    /// Defaults to `0` if not set.
    pub fn max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = Some(max_retries);
        self
    }

    /// Schedule the run for later execution.
    pub fn scheduled_at(mut self, at: DateTime<Utc>) -> Self {
        self.scheduled_at = Some(at);
        self
    }

    /// Set the authenticated principal creating this run.
    pub fn created_by(mut self, actor: RunActor) -> Self {
        self.created_by = Some(actor);
        self
    }

    /// Set an idempotency key to prevent duplicate runs.
    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Set user-defined labels for categorization.
    pub fn labels(mut self, labels: HashMap<String, String>) -> Self {
        self.labels = Some(labels);
        self
    }

    /// Set the maximum cumulative cost allowed for this run.
    pub fn max_cost_usd(mut self, cap: Decimal) -> Self {
        self.max_cost_usd = Some(cap);
        self
    }

    /// Assemble a [`NewRun`] from these options and handler metadata.
    ///
    /// * `workflow_name` -- typically from [`WorkflowHandler::name`].
    /// * `handler_version` -- typically from [`WorkflowHandler::version`].
    /// * `default_max_cost_usd` -- typically from [`WorkflowHandler::default_max_cost_usd`].
    ///   Applied only when [`max_cost_usd`](Self::max_cost_usd) was not set.
    ///
    /// [`WorkflowHandler::name`]: crate::handler::WorkflowHandler::name
    /// [`WorkflowHandler::version`]: crate::handler::WorkflowHandler::version
    /// [`WorkflowHandler::default_max_cost_usd`]: crate::handler::WorkflowHandler::default_max_cost_usd
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::run_creator::CreateRunOpts;
    /// use rust_decimal::Decimal;
    ///
    /// let new_run = CreateRunOpts::new()
    ///     .build("my-handler", Some("3.0.0"), Some(Decimal::new(1000, 2)));
    ///
    /// assert_eq!(new_run.workflow_name, "my-handler");
    /// assert_eq!(new_run.handler_version, Some("3.0.0".to_string()));
    /// assert_eq!(new_run.max_cost_usd, Some(Decimal::new(1000, 2)));
    /// ```
    pub fn build(
        self,
        workflow_name: &str,
        handler_version: Option<&str>,
        default_max_cost_usd: Option<Decimal>,
    ) -> NewRun {
        NewRun {
            workflow_name: workflow_name.to_string(),
            trigger: self.trigger.unwrap_or(TriggerKind::Manual),
            payload: self.payload.unwrap_or_else(|| serde_json::json!({})),
            max_retries: self.max_retries.unwrap_or(0),
            handler_version: handler_version.map(str::to_string),
            labels: self.labels.unwrap_or_default(),
            scheduled_at: self.scheduled_at,
            created_by: self.created_by,
            idempotency_key: self.idempotency_key,
            max_cost_usd: self.max_cost_usd.or(default_max_cost_usd),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn create_run_opts_default_produces_correct_defaults() {
        let opts = CreateRunOpts::new();
        let new_run = opts.build("test-workflow", None, None);

        assert_eq!(new_run.workflow_name, "test-workflow");
        assert_eq!(new_run.trigger, TriggerKind::Manual);
        assert_eq!(new_run.payload, json!({}));
        assert_eq!(new_run.max_retries, 0);
        assert_eq!(new_run.handler_version, None);
        assert!(new_run.labels.is_empty());
        assert_eq!(new_run.scheduled_at, None);
        assert_eq!(new_run.created_by, None);
        assert_eq!(new_run.idempotency_key, None);
        assert_eq!(new_run.max_cost_usd, None);
    }

    #[test]
    fn create_run_opts_builder_sets_all_fields() {
        let labels = HashMap::from([("env".to_string(), "prod".to_string())]);
        let scheduled = Utc::now();

        let new_run = CreateRunOpts::new()
            .trigger(TriggerKind::Webhook {
                path: "/hooks/gh".to_string(),
            })
            .payload(json!({"ref": "main"}))
            .max_retries(3)
            .scheduled_at(scheduled)
            .idempotency_key("key-123")
            .labels(labels.clone())
            .max_cost_usd(Decimal::new(500, 2))
            .build("deploy", Some("2.0.0"), None);

        assert_eq!(new_run.workflow_name, "deploy");
        assert_eq!(
            new_run.trigger,
            TriggerKind::Webhook {
                path: "/hooks/gh".to_string()
            }
        );
        assert_eq!(new_run.payload, json!({"ref": "main"}));
        assert_eq!(new_run.max_retries, 3);
        assert_eq!(new_run.handler_version, Some("2.0.0".to_string()));
        assert_eq!(new_run.scheduled_at, Some(scheduled));
        assert_eq!(new_run.idempotency_key, Some("key-123".to_string()));
        assert_eq!(new_run.labels, labels);
        assert_eq!(new_run.max_cost_usd, Some(Decimal::new(500, 2)));
    }

    #[test]
    fn create_run_opts_build_uses_handler_metadata() {
        let new_run =
            CreateRunOpts::new().build("my-handler", Some("3.0.0"), Some(Decimal::new(1000, 2)));

        assert_eq!(new_run.workflow_name, "my-handler");
        assert_eq!(new_run.handler_version, Some("3.0.0".to_string()));
        assert_eq!(new_run.max_cost_usd, Some(Decimal::new(1000, 2)));
    }

    #[test]
    fn create_run_opts_explicit_max_cost_overrides_handler_default() {
        let new_run = CreateRunOpts::new()
            .max_cost_usd(Decimal::new(200, 2))
            .build("handler", Some("1"), Some(Decimal::new(1000, 2)));

        assert_eq!(new_run.max_cost_usd, Some(Decimal::new(200, 2)));
    }

    #[tokio::test]
    async fn run_creator_blanket_impl_with_in_memory_store() {
        use ironflow_store::memory::InMemoryStore;

        let store = InMemoryStore::new();
        let creator: &dyn RunCreator = &store;

        let new_run =
            CreateRunOpts::new()
                .trigger(TriggerKind::Api)
                .build("blanket-test", None, None);

        let creation = creator.create_run(new_run).await.expect("create_run");
        let run = creation.into_run();
        assert_eq!(run.workflow_name, "blanket-test");
    }

    #[tokio::test]
    async fn create_run_with_reused_idempotency_key_returns_existing() {
        use ironflow_store::memory::InMemoryStore;

        let store = InMemoryStore::new();
        let creator: &dyn RunCreator = &store;

        let first = creator
            .create_run(
                CreateRunOpts::new()
                    .idempotency_key("dedup-1")
                    .build("idem-test", None, None),
            )
            .await
            .expect("first create_run");
        assert!(first.is_created());

        let second = creator
            .create_run(
                CreateRunOpts::new()
                    .idempotency_key("dedup-1")
                    .build("idem-test", None, None),
            )
            .await
            .expect("second create_run");
        assert!(!second.is_created());
        assert_eq!(first.into_run().id, second.into_run().id);
    }
}
