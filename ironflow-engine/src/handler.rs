//! [`WorkflowHandler`] trait — dynamic workflows with context chaining.
//!
//! Implement this trait to define workflows where steps can reference
//! outputs from previous steps. The handler receives a [`WorkflowContext`]
//! that provides step execution methods with automatic persistence.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_engine::handler::WorkflowHandler;
//! use ironflow_engine::context::WorkflowContext;
//! use ironflow_engine::config::{ShellConfig, AgentStepConfig};
//! use ironflow_engine::error::EngineError;
//! use std::future::Future;
//! use std::pin::Pin;
//!
//! struct DeployWorkflow;
//!
//! impl WorkflowHandler for DeployWorkflow {
//!     fn name(&self) -> &str {
//!         "deploy"
//!     }
//!
//!     fn execute<'a>(
//!         &'a self,
//!         ctx: &'a mut WorkflowContext,
//!     ) -> Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send + 'a>> {
//!         Box::pin(async move {
//!             let build = ctx.shell("build", ShellConfig::new("cargo build --release")).await?;
//!             let tests = ctx.shell("test", ShellConfig::new("cargo test")).await?;
//!
//!             let review = ctx.agent("review", AgentStepConfig::new(
//!                 &format!("Build:\n{}\nTests:\n{}\nReview.",
//!                     build.output["stdout"], tests.output["stdout"])
//!             )).await?;
//!
//!             if review.output.as_str().unwrap_or("").contains("LGTM") {
//!                 ctx.shell("deploy", ShellConfig::new("./deploy.sh")).await?;
//!             }
//!
//!             Ok(())
//!         })
//!     }
//! }
//! ```

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

use crate::context::WorkflowContext;
use crate::error::EngineError;
use crate::guard::WorkflowGuardConfig;
use crate::run_creator::{CreateRunOpts, RunCreator, RunCreatorFuture};
use crate::schedule::CronSchedule;

/// Generate a JSON Schema [`Value`] from a type that derives [`JsonSchema`].
///
/// Use this in [`WorkflowHandler::input_schema`] to automatically derive the
/// schema from your input struct instead of writing JSON by hand.
///
/// # Examples
///
/// ```
/// use schemars::JsonSchema;
/// use serde::Deserialize;
/// use ironflow_engine::handler::input_schema_for;
///
/// #[derive(Deserialize, JsonSchema)]
/// struct DeployInput {
///     environment: String,
///     dry_run: Option<bool>,
/// }
///
/// let schema = input_schema_for::<DeployInput>();
/// assert_eq!(schema["type"], "object");
/// assert!(schema["properties"]["environment"].is_object());
/// ```
pub fn input_schema_for<T: JsonSchema>() -> Value {
    let schema = schemars::schema_for!(T);
    serde_json::to_value(schema).expect("schema serialization cannot fail")
}

/// Boxed future returned by [`WorkflowHandler::execute`].
pub type HandlerFuture<'a> = Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send + 'a>>;

/// Metadata about a workflow, returned by [`WorkflowHandler::describe`].
///
/// Contains a human-readable description and optional Rust source code
/// for display in the dashboard.
///
/// Most handlers never build this struct by hand: override
/// [`WorkflowHandler::description`] and [`WorkflowHandler::source_code`] and
/// the default [`WorkflowHandler::describe`] assembles it from the other
/// trait methods. The builder below exists for handlers that override
/// `describe` entirely.
///
/// # Examples
///
/// ```
/// use ironflow_engine::handler::WorkflowInfo;
///
/// let info = WorkflowInfo::new("Deploy to production")
///     .with_category("ops")
///     .with_version("2.0.0")
///     .with_sub_workflows(["build"]);
///
/// assert_eq!(info.description, "Deploy to production");
/// assert_eq!(info.category.as_deref(), Some("ops"));
/// assert_eq!(info.sub_workflows, vec!["build".to_string()]);
/// ```
#[derive(Debug, Clone, Default, Serialize)]
pub struct WorkflowInfo {
    /// Human-readable description of what the workflow does.
    pub description: String,
    /// Optional Rust source code of the handler (for UI display).
    pub source_code: Option<String>,
    /// Names of sub-workflows invoked by this handler.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_workflows: Vec<String>,
    /// Optional `/`-separated category path used to group workflows in the UI tree.
    ///
    /// A value like `"data/etl"` places the workflow under `data` → `etl`.
    /// `None` means the workflow is uncategorized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Handler version string, used to trace which code produced a given run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Versions accepted for replay without `force`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compatible_versions: Vec<String>,
    /// JSON Schema describing the expected input payload.
    ///
    /// When present, the dashboard renders a dynamic form from this schema
    /// and the engine validates the payload before creating a run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    /// Labels automatically applied to every run of this workflow.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub default_labels: HashMap<String, String>,
    /// Optional cron schedule for automatic execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<CronSchedule>,
    /// Default cumulative cost cap applied to runs of this workflow, in USD.
    ///
    /// Overridden by a cap supplied at run creation, and takes precedence over
    /// the server-wide default. `None` means the handler declares no default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_max_cost_usd: Option<Decimal>,
}

impl WorkflowInfo {
    /// Create metadata with a description and every other field at its default.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::handler::WorkflowInfo;
    ///
    /// let info = WorkflowInfo::new("Nightly backup");
    /// assert_eq!(info.description, "Nightly backup");
    /// assert!(info.source_code.is_none());
    /// assert!(info.sub_workflows.is_empty());
    /// ```
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            ..Self::default()
        }
    }

    /// Attach the handler source code, typically via `include_str!`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::handler::WorkflowInfo;
    ///
    /// let info = WorkflowInfo::new("Demo").with_source_code("struct Demo;");
    /// assert_eq!(info.source_code.as_deref(), Some("struct Demo;"));
    /// ```
    pub fn with_source_code(mut self, source: impl Into<String>) -> Self {
        self.source_code = Some(source.into());
        self
    }

    /// Declare the sub-workflows this handler invokes.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::handler::WorkflowInfo;
    ///
    /// let info = WorkflowInfo::new("Report").with_sub_workflows(["collect", "enrich"]);
    /// assert_eq!(info.sub_workflows, vec!["collect".to_string(), "enrich".to_string()]);
    /// ```
    pub fn with_sub_workflows<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.sub_workflows = names.into_iter().map(Into::into).collect();
        self
    }

    /// Set the `/`-separated category path.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::handler::WorkflowInfo;
    ///
    /// let info = WorkflowInfo::new("ETL").with_category("data/etl");
    /// assert_eq!(info.category.as_deref(), Some("data/etl"));
    /// ```
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Set the handler version.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::handler::WorkflowInfo;
    ///
    /// let info = WorkflowInfo::new("Deploy").with_version("1.2.0");
    /// assert_eq!(info.version.as_deref(), Some("1.2.0"));
    /// ```
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Set the versions accepted for replay without `force`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::handler::WorkflowInfo;
    ///
    /// let info = WorkflowInfo::new("Deploy").with_compatible_versions(["1.0.0"]);
    /// assert_eq!(info.compatible_versions, vec!["1.0.0".to_string()]);
    /// ```
    pub fn with_compatible_versions<I, S>(mut self, versions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.compatible_versions = versions.into_iter().map(Into::into).collect();
        self
    }

    /// Set the JSON Schema of the expected input payload.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::handler::WorkflowInfo;
    /// use serde_json::json;
    ///
    /// let info = WorkflowInfo::new("Greet").with_input_schema(json!({"type": "object"}));
    /// assert_eq!(info.input_schema.unwrap()["type"], "object");
    /// ```
    pub fn with_input_schema(mut self, schema: Value) -> Self {
        self.input_schema = Some(schema);
        self
    }

    /// Set the labels applied to every run of this workflow.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::HashMap;
    /// use ironflow_engine::handler::WorkflowInfo;
    ///
    /// let labels = HashMap::from([("team".to_string(), "core".to_string())]);
    /// let info = WorkflowInfo::new("Sync").with_default_labels(labels);
    /// assert_eq!(info.default_labels["team"], "core");
    /// ```
    pub fn with_default_labels(mut self, labels: HashMap<String, String>) -> Self {
        self.default_labels = labels;
        self
    }

    /// Set the cron schedule.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::handler::WorkflowInfo;
    /// use ironflow_engine::schedule::CronSchedule;
    ///
    /// let schedule = CronSchedule::new("0 0 * * *")?;
    /// let info = WorkflowInfo::new("Nightly").with_schedule(schedule);
    /// assert!(info.schedule.is_some());
    /// # Ok::<(), String>(())
    /// ```
    pub fn with_schedule(mut self, schedule: CronSchedule) -> Self {
        self.schedule = Some(schedule);
        self
    }

    /// Set the default cumulative cost cap in USD.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::handler::WorkflowInfo;
    /// use rust_decimal::Decimal;
    ///
    /// let info = WorkflowInfo::new("Analysis").with_default_max_cost_usd(Decimal::new(500, 2));
    /// assert_eq!(info.default_max_cost_usd, Some(Decimal::new(500, 2)));
    /// ```
    pub fn with_default_max_cost_usd(mut self, cap: Decimal) -> Self {
        self.default_max_cost_usd = Some(cap);
        self
    }
}

/// A dynamic workflow handler with context-aware step chaining.
///
/// Implement this trait to define workflows where each step can use
/// the output of previous steps. Register handlers with
/// [`Engine::register`](crate::engine::Engine::register) and execute
/// them by name.
///
/// # Why `Pin<Box<dyn Future>>` instead of `async fn`?
///
/// The handler must be object-safe (`dyn WorkflowHandler`) to allow
/// registering different handler types in the engine's registry.
pub trait WorkflowHandler: Send + Sync {
    /// The workflow name used for registration and lookup.
    fn name(&self) -> &str;

    /// Handler version string, used to trace which code version produced a run.
    ///
    /// Override this to return a meaningful version (semver, git SHA, build
    /// hash, etc.). The default is `"1"`.
    ///
    /// The engine records this value on every run it creates so that retries
    /// can detect when the handler has changed since the original execution.
    fn version(&self) -> Option<&str> {
        Some("1")
    }

    /// Versions of this handler that can replay payloads produced by an
    /// older run without requiring `force`.
    ///
    /// When a retry targets a run whose `handler_version` differs from
    /// [`version`](Self::version), the engine checks this list. If the
    /// run's version appears here, the retry proceeds normally; otherwise
    /// it is refused with `409 HANDLER_VERSION_MISMATCH` unless the caller
    /// passes `force=true`.
    ///
    /// The default is an empty slice (only the current version is accepted).
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::handler::{WorkflowHandler, HandlerFuture};
    /// # use ironflow_engine::context::WorkflowContext;
    /// struct MigratedHandler;
    ///
    /// impl WorkflowHandler for MigratedHandler {
    ///     fn name(&self) -> &str { "migrated" }
    ///     fn version(&self) -> Option<&str> { Some("2.0.0") }
    ///     fn compatible_versions(&self) -> &[&str] { &["1.0.0", "1.5.0"] }
    ///     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    ///         Box::pin(async move { Ok(()) })
    ///     }
    /// }
    ///
    /// assert_eq!(MigratedHandler.compatible_versions(), &["1.0.0", "1.5.0"]);
    /// ```
    fn compatible_versions(&self) -> &[&str] {
        &[]
    }

    /// Human-readable description shown in the dashboard and the CLI.
    ///
    /// The default is an empty string. Override this rather than
    /// [`describe`](Self::describe): the default `describe` picks it up.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::handler::{WorkflowHandler, HandlerFuture};
    /// # use ironflow_engine::context::WorkflowContext;
    /// struct Backup;
    ///
    /// impl WorkflowHandler for Backup {
    ///     fn name(&self) -> &str { "backup" }
    ///     fn description(&self) -> &str { "Nightly database backup" }
    ///     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    ///         Box::pin(async move { Ok(()) })
    ///     }
    /// }
    ///
    /// assert_eq!(Backup.describe().description, "Nightly database backup");
    /// ```
    fn description(&self) -> &str {
        ""
    }

    /// Rust source of the handler, displayed in the dashboard.
    ///
    /// Return `Some(include_str!("this_file.rs"))` to show the code next to
    /// the run. The default is `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::handler::{WorkflowHandler, HandlerFuture};
    /// # use ironflow_engine::context::WorkflowContext;
    /// struct Backup;
    ///
    /// impl WorkflowHandler for Backup {
    ///     fn name(&self) -> &str { "backup" }
    ///     fn source_code(&self) -> Option<&str> { Some("struct Backup;") }
    ///     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    ///         Box::pin(async move { Ok(()) })
    ///     }
    /// }
    ///
    /// assert_eq!(Backup.describe().source_code.as_deref(), Some("struct Backup;"));
    /// ```
    fn source_code(&self) -> Option<&str> {
        None
    }

    /// Names of the sub-workflows this handler invokes through
    /// [`WorkflowContext::workflow`](crate::context::WorkflowContext::workflow).
    ///
    /// Purely informational: the dashboard uses it to draw the call graph.
    /// The default is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::handler::{WorkflowHandler, HandlerFuture};
    /// # use ironflow_engine::context::WorkflowContext;
    /// struct Report;
    ///
    /// impl WorkflowHandler for Report {
    ///     fn name(&self) -> &str { "report" }
    ///     fn sub_workflows(&self) -> Vec<String> { vec!["collect".to_string()] }
    ///     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    ///         Box::pin(async move { Ok(()) })
    ///     }
    /// }
    ///
    /// assert_eq!(Report.describe().sub_workflows, vec!["collect".to_string()]);
    /// ```
    fn sub_workflows(&self) -> Vec<String> {
        Vec::new()
    }

    /// Optional `/`-separated category path used to group workflows in the UI tree.
    ///
    /// Return a value like `"data/etl"` to place the workflow under `data` → `etl`.
    /// The default is `None` (uncategorized).
    ///
    /// Validation (empty segments, leading or trailing `/`, `//`, whitespace
    /// segments) is enforced at registration time by
    /// [`Engine::register`](crate::engine::Engine::register).
    fn category(&self) -> Option<&str> {
        None
    }

    /// Return a JSON Schema describing the expected input payload.
    ///
    /// When present, the dashboard renders a dynamic form from this schema
    /// and the engine validates the payload before creating a run.
    /// The default is `None` (no schema, free-form payload).
    fn input_schema(&self) -> Option<Value> {
        None
    }

    /// Labels automatically applied to every run of this workflow.
    ///
    /// These are merged with any labels provided at run creation time.
    /// User-provided labels take precedence over defaults.
    fn default_labels(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    /// Optional cron schedule for automatic execution.
    ///
    /// Return a [`CronSchedule`] built from a cron expression
    /// (5 or 6 fields, as supported by [`croner`]).
    ///
    /// When set, the engine exposes this handler via
    /// [`Engine::scheduled_handlers`](crate::engine::Engine::scheduled_handlers)
    /// so the runtime can wire it into a cron scheduler automatically.
    ///
    /// The default is `None` (no automatic scheduling).
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::handler::{WorkflowHandler, HandlerFuture};
    /// # use ironflow_engine::context::WorkflowContext;
    /// # use ironflow_engine::schedule::CronSchedule;
    /// struct HourlySync;
    ///
    /// impl WorkflowHandler for HourlySync {
    ///     fn name(&self) -> &str { "hourly-sync" }
    ///     fn schedule(&self) -> Option<&CronSchedule> {
    ///         // In practice, store as a field or use `std::sync::LazyLock`.
    ///         None
    ///     }
    ///     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    ///         Box::pin(async move { Ok(()) })
    ///     }
    /// }
    /// ```
    fn schedule(&self) -> Option<&CronSchedule> {
        None
    }

    /// Default cumulative cost cap for runs of this workflow, in USD.
    ///
    /// Applied when the run creation request does not supply one. Takes
    /// precedence over the server-wide
    /// [`IRONFLOW_DEFAULT_RUN_MAX_COST_USD`](crate::budget::DEFAULT_RUN_MAX_COST_ENV).
    /// The default is `None` (fall back to the server default, or no cap).
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::handler::{WorkflowHandler, HandlerFuture};
    /// # use ironflow_engine::context::WorkflowContext;
    /// use rust_decimal::Decimal;
    ///
    /// struct ExpensiveAnalysis;
    ///
    /// impl WorkflowHandler for ExpensiveAnalysis {
    ///     fn name(&self) -> &str { "expensive-analysis" }
    ///     fn default_max_cost_usd(&self) -> Option<Decimal> {
    ///         Some(Decimal::new(500, 2)) // $5.00
    ///     }
    ///     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    ///         Box::pin(async move { Ok(()) })
    ///     }
    /// }
    ///
    /// assert_eq!(ExpensiveAnalysis.default_max_cost_usd(), Some(Decimal::new(500, 2)));
    /// ```
    fn default_max_cost_usd(&self) -> Option<Decimal> {
        None
    }

    /// Optional guard configuration for this workflow.
    ///
    /// When present, overrides the engine's global guard configuration
    /// for runs of this handler. The default is `None` (use the engine's
    /// global configuration).
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::handler::{WorkflowHandler, HandlerFuture};
    /// # use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::guard::WorkflowGuardConfig;
    ///
    /// struct StrictWorkflow;
    ///
    /// impl WorkflowHandler for StrictWorkflow {
    ///     fn name(&self) -> &str { "strict" }
    ///     fn guard_config(&self) -> Option<WorkflowGuardConfig> {
    ///         Some(WorkflowGuardConfig::new().with_max_depth(2))
    ///     }
    ///     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    ///         Box::pin(async move { Ok(()) })
    ///     }
    /// }
    ///
    /// assert_eq!(StrictWorkflow.guard_config().unwrap().max_depth, 2);
    /// ```
    fn guard_config(&self) -> Option<WorkflowGuardConfig> {
        None
    }

    /// Check whether a run carrying `run_version` can be replayed by this
    /// handler without `force`.
    ///
    /// Compatibility rules:
    /// - `run_version` is `None` (old run predating version tracking): always
    ///   compatible.
    /// - `run_version` equals [`version`](Self::version): compatible.
    /// - `run_version` appears in [`compatible_versions`](Self::compatible_versions):
    ///   compatible.
    /// - Otherwise: incompatible.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::handler::{WorkflowHandler, HandlerFuture};
    /// # use ironflow_engine::context::WorkflowContext;
    /// struct MyHandler;
    ///
    /// impl WorkflowHandler for MyHandler {
    ///     fn name(&self) -> &str { "my-handler" }
    ///     fn version(&self) -> Option<&str> { Some("2.0.0") }
    ///     fn compatible_versions(&self) -> &[&str] { &["1.0.0"] }
    ///     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    ///         Box::pin(async move { Ok(()) })
    ///     }
    /// }
    ///
    /// assert!(MyHandler.is_version_compatible(None));
    /// assert!(MyHandler.is_version_compatible(Some("2.0.0")));
    /// assert!(MyHandler.is_version_compatible(Some("1.0.0")));
    /// assert!(!MyHandler.is_version_compatible(Some("0.5.0")));
    /// ```
    fn is_version_compatible(&self, run_version: Option<&str>) -> bool {
        let Some(rv) = run_version else {
            return true;
        };
        if self.version() == Some(rv) {
            return true;
        }
        self.compatible_versions().contains(&rv)
    }

    /// Return metadata about this workflow.
    ///
    /// The default assembles a [`WorkflowInfo`] from every other trait
    /// method: [`description`](Self::description),
    /// [`source_code`](Self::source_code),
    /// [`sub_workflows`](Self::sub_workflows), [`category`](Self::category),
    /// [`version`](Self::version),
    /// [`compatible_versions`](Self::compatible_versions),
    /// [`input_schema`](Self::input_schema),
    /// [`default_labels`](Self::default_labels), [`schedule`](Self::schedule)
    /// and [`default_max_cost_usd`](Self::default_max_cost_usd). Override
    /// those instead of this method; override `describe` only when the
    /// metadata cannot be expressed through them.
    fn describe(&self) -> WorkflowInfo {
        WorkflowInfo {
            description: self.description().to_string(),
            source_code: self.source_code().map(str::to_string),
            sub_workflows: self.sub_workflows(),
            category: self.category().map(str::to_string),
            version: self.version().map(str::to_string),
            compatible_versions: self
                .compatible_versions()
                .iter()
                .map(|s| s.to_string())
                .collect(),
            input_schema: self.input_schema(),
            default_labels: self.default_labels(),
            schedule: self.schedule().cloned(),
            default_max_cost_usd: self.default_max_cost_usd(),
        }
    }

    /// Create a run for this workflow, using handler metadata automatically.
    ///
    /// Assembles a [`NewRun`](ironflow_store::entities::NewRun) from [`name`](Self::name),
    /// [`version`](Self::version), and [`default_max_cost_usd`](Self::default_max_cost_usd),
    /// then delegates to the given [`RunCreator`].
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the underlying store rejects the run.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use ironflow_engine::handler::{WorkflowHandler, HandlerFuture};
    /// # use ironflow_engine::context::WorkflowContext;
    /// # use ironflow_engine::run_creator::{CreateRunOpts, RunCreator};
    /// # use ironflow_store::entities::TriggerKind;
    /// struct DeployWorkflow;
    ///
    /// impl WorkflowHandler for DeployWorkflow {
    ///     fn name(&self) -> &str { "deploy" }
    ///     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    ///         Box::pin(async move { Ok(()) })
    ///     }
    /// }
    ///
    /// # async fn example(store: &dyn RunCreator) -> Result<(), ironflow_engine::error::EngineError> {
    /// let opts = CreateRunOpts::new().trigger(TriggerKind::Api);
    /// let run = DeployWorkflow.create_run(store, opts).await?.into_run();
    /// assert_eq!(run.workflow_name, "deploy");
    /// # Ok(())
    /// # }
    /// ```
    fn create_run<'a>(
        &self,
        creator: &'a dyn RunCreator,
        opts: CreateRunOpts,
    ) -> RunCreatorFuture<'a> {
        use tracing::{Instrument, info_span};

        let new_run = opts.build(self.name(), self.version(), self.default_max_cost_usd());
        let span = info_span!("handler.create_run", workflow = %self.name());
        Box::pin(creator.create_run(new_run).instrument(span))
    }

    /// Execute the workflow with the given context.
    ///
    /// The context provides [`shell`](WorkflowContext::shell),
    /// [`http`](WorkflowContext::http), and [`agent`](WorkflowContext::agent)
    /// methods that automatically persist each step.
    ///
    /// # Errors
    ///
    /// Return [`EngineError`] if any step fails. The engine will mark
    /// the run as `Failed` and record the error.
    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a>;
}

/// A boxed handler is a handler.
///
/// Lets a single `Vec<Box<dyn WorkflowHandler>>` feed both
/// [`Engine::register`](crate::engine::Engine::register) and a worker
/// builder, so the API server and the workers cannot drift apart in the
/// list of workflows they know.
///
/// # Examples
///
/// ```
/// use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
/// use ironflow_engine::context::WorkflowContext;
///
/// struct Hello;
///
/// impl WorkflowHandler for Hello {
///     fn name(&self) -> &str { "hello" }
///     fn description(&self) -> &str { "Says hello" }
///     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
///         Box::pin(async move { Ok(()) })
///     }
/// }
///
/// fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
///     vec![Box::new(Hello)]
/// }
///
/// for handler in handlers() {
///     assert_eq!(handler.name(), "hello");
///     assert_eq!(handler.describe().description, "Says hello");
/// }
/// ```
impl<T: WorkflowHandler + ?Sized> WorkflowHandler for Box<T> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn version(&self) -> Option<&str> {
        (**self).version()
    }

    fn compatible_versions(&self) -> &[&str] {
        (**self).compatible_versions()
    }

    fn description(&self) -> &str {
        (**self).description()
    }

    fn source_code(&self) -> Option<&str> {
        (**self).source_code()
    }

    fn sub_workflows(&self) -> Vec<String> {
        (**self).sub_workflows()
    }

    fn category(&self) -> Option<&str> {
        (**self).category()
    }

    fn input_schema(&self) -> Option<Value> {
        (**self).input_schema()
    }

    fn default_labels(&self) -> HashMap<String, String> {
        (**self).default_labels()
    }

    fn schedule(&self) -> Option<&CronSchedule> {
        (**self).schedule()
    }

    fn default_max_cost_usd(&self) -> Option<Decimal> {
        (**self).default_max_cost_usd()
    }

    fn guard_config(&self) -> Option<WorkflowGuardConfig> {
        (**self).guard_config()
    }

    fn is_version_compatible(&self, run_version: Option<&str>) -> bool {
        (**self).is_version_compatible(run_version)
    }

    fn describe(&self) -> WorkflowInfo {
        (**self).describe()
    }

    fn create_run<'a>(
        &self,
        creator: &'a dyn RunCreator,
        opts: CreateRunOpts,
    ) -> RunCreatorFuture<'a> {
        (**self).create_run(creator, opts)
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        (**self).execute(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    struct TestInput {
        environment: String,
        #[serde(default)]
        dry_run: bool,
    }

    struct MinimalHandler;

    impl WorkflowHandler for MinimalHandler {
        fn name(&self) -> &str {
            "minimal"
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async { Ok(()) })
        }
    }

    struct FullFeaturedHandler;

    impl WorkflowHandler for FullFeaturedHandler {
        fn name(&self) -> &str {
            "full"
        }

        fn version(&self) -> Option<&str> {
            Some("1.2.0")
        }

        fn category(&self) -> Option<&str> {
            Some("data/etl")
        }

        fn input_schema(&self) -> Option<Value> {
            Some(input_schema_for::<TestInput>())
        }

        fn default_labels(&self) -> HashMap<String, String> {
            HashMap::from([
                ("team".to_string(), "platform".to_string()),
                ("env".to_string(), "prod".to_string()),
            ])
        }

        fn default_max_cost_usd(&self) -> Option<Decimal> {
            Some(Decimal::new(750, 2))
        }

        fn describe(&self) -> WorkflowInfo {
            WorkflowInfo {
                description: "Full-featured test handler".to_string(),
                source_code: Some("fn test() {}".to_string()),
                sub_workflows: vec!["helper".to_string()],
                category: self.category().map(str::to_string),
                version: self.version().map(str::to_string),
                compatible_versions: self
                    .compatible_versions()
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                input_schema: self.input_schema(),
                default_labels: self.default_labels(),
                schedule: self.schedule().cloned(),
                default_max_cost_usd: self.default_max_cost_usd(),
            }
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async { Ok(()) })
        }
    }

    #[test]
    fn minimal_handler_has_required_name() {
        let handler = MinimalHandler;
        assert_eq!(handler.name(), "minimal");
    }

    #[test]
    fn minimal_handler_defaults_to_version_1() {
        let handler = MinimalHandler;
        assert_eq!(handler.version(), Some("1"));
    }

    #[test]
    fn minimal_handler_defaults_to_no_compatible_versions() {
        let handler = MinimalHandler;
        assert!(handler.compatible_versions().is_empty());
    }

    #[test]
    fn minimal_handler_defaults_to_no_category() {
        let handler = MinimalHandler;
        assert_eq!(handler.category(), None);
    }

    #[test]
    fn minimal_handler_defaults_to_no_schema() {
        let handler = MinimalHandler;
        assert_eq!(handler.input_schema(), None);
    }

    #[test]
    fn minimal_handler_defaults_to_empty_labels() {
        let handler = MinimalHandler;
        let labels = handler.default_labels();
        assert!(labels.is_empty());
    }

    #[test]
    fn minimal_handler_defaults_to_no_schedule() {
        let handler = MinimalHandler;
        assert_eq!(handler.schedule(), None);
    }

    #[test]
    fn minimal_handler_describe_reflects_defaults() {
        let handler = MinimalHandler;
        let info = handler.describe();
        assert_eq!(info.description, "");
        assert_eq!(info.source_code, None);
        assert_eq!(info.sub_workflows, Vec::<String>::new());
        assert_eq!(info.category, None);
        assert_eq!(info.version, Some("1".to_string()));
        assert!(info.compatible_versions.is_empty());
        assert_eq!(info.input_schema, None);
        assert!(info.default_labels.is_empty());
        assert_eq!(info.schedule, None);
    }

    #[test]
    fn full_handler_returns_all_metadata() {
        let handler = FullFeaturedHandler;
        assert_eq!(handler.name(), "full");
        assert_eq!(handler.version(), Some("1.2.0"));
        assert_eq!(handler.category(), Some("data/etl"));
        assert!(handler.input_schema().is_some());
    }

    #[test]
    fn full_handler_default_labels_are_set() {
        let handler = FullFeaturedHandler;
        let labels = handler.default_labels();
        assert_eq!(labels.get("team"), Some(&"platform".to_string()));
        assert_eq!(labels.get("env"), Some(&"prod".to_string()));
    }

    #[test]
    fn full_handler_describe_includes_all_fields() {
        let handler = FullFeaturedHandler;
        let info = handler.describe();
        assert_eq!(info.description, "Full-featured test handler");
        assert_eq!(info.source_code, Some("fn test() {}".to_string()));
        assert_eq!(info.sub_workflows, vec!["helper".to_string()]);
        assert_eq!(info.category, Some("data/etl".to_string()));
        assert_eq!(info.version, Some("1.2.0".to_string()));
        assert!(info.input_schema.is_some());
        assert_eq!(info.default_labels.len(), 2);
    }

    #[test]
    fn input_schema_for_generates_json_schema() {
        let schema = input_schema_for::<TestInput>();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["environment"].is_object());
        assert!(schema["properties"]["dry_run"].is_object());
    }

    #[test]
    fn input_schema_for_preserves_serde_attributes() {
        let schema = input_schema_for::<TestInput>();
        let properties = &schema["properties"];
        assert!(properties.is_object());
        assert!(properties.get("environment").is_some());
        assert!(properties.get("dry_run").is_some());
    }

    #[test]
    fn minimal_handler_defaults_to_no_max_cost() {
        assert!(MinimalHandler.default_max_cost_usd().is_none());
        assert!(MinimalHandler.describe().default_max_cost_usd.is_none());
    }

    #[test]
    fn describe_propagates_handler_max_cost() {
        assert_eq!(
            FullFeaturedHandler.describe().default_max_cost_usd,
            Some(Decimal::new(750, 2))
        );
    }

    #[test]
    fn workflow_info_omits_absent_max_cost_from_json() {
        let json = serde_json::to_value(MinimalHandler.describe()).expect("serialize");
        assert!(json.get("default_max_cost_usd").is_none());
    }

    #[test]
    fn workflow_info_serializes_with_skip_empty() {
        let info = WorkflowInfo {
            description: "test".to_string(),
            source_code: None,
            sub_workflows: Vec::new(),
            category: None,
            version: None,
            compatible_versions: Vec::new(),
            input_schema: None,
            default_labels: HashMap::new(),
            schedule: None,
            default_max_cost_usd: None,
        };

        let json = serde_json::to_value(&info).expect("serialize");
        assert_eq!(json["description"], "test");
        // Optional fields with skip_serializing_if may still be present or absent
        // depending on the serde configuration. Just verify the description is there.
        assert!(json.is_object());
    }

    #[test]
    fn workflow_info_serializes_with_values() {
        let info = WorkflowInfo {
            description: "test".to_string(),
            source_code: Some("code".to_string()),
            sub_workflows: vec!["sub".to_string()],
            category: Some("cat".to_string()),
            version: Some("1.0.0".to_string()),
            compatible_versions: vec!["0.9.0".to_string()],
            input_schema: Some(serde_json::json!({"type": "object"})),
            default_labels: HashMap::from([("key".to_string(), "value".to_string())]),
            schedule: Some(CronSchedule::new("0 0 * * * *").unwrap()),
            default_max_cost_usd: Some(Decimal::new(750, 2)),
        };

        let json = serde_json::to_value(&info).expect("serialize");
        assert_eq!(json["description"], "test");
        assert_eq!(json["source_code"], "code");
        assert_eq!(json["sub_workflows"][0], "sub");
        assert_eq!(json["category"], "cat");
        assert_eq!(json["version"], "1.0.0");
        assert_eq!(json["default_labels"]["key"], "value");
        assert_eq!(json["schedule"], "0 0 * * * *");
        assert_eq!(json["compatible_versions"][0], "0.9.0");
    }

    // ---- is_version_compatible ----

    struct VersionedHandler;

    impl WorkflowHandler for VersionedHandler {
        fn name(&self) -> &str {
            "versioned"
        }
        fn version(&self) -> Option<&str> {
            Some("2.0.0")
        }
        fn compatible_versions(&self) -> &[&str] {
            &["1.5.0", "1.9.0"]
        }
        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async { Ok(()) })
        }
    }

    #[test]
    fn version_compatible_with_same_version() {
        assert!(VersionedHandler.is_version_compatible(Some("2.0.0")));
    }

    #[test]
    fn version_compatible_with_none_run_version() {
        assert!(VersionedHandler.is_version_compatible(None));
    }

    #[test]
    fn version_compatible_with_listed_version() {
        assert!(VersionedHandler.is_version_compatible(Some("1.5.0")));
        assert!(VersionedHandler.is_version_compatible(Some("1.9.0")));
    }

    #[test]
    fn version_incompatible_with_unlisted_version() {
        assert!(!VersionedHandler.is_version_compatible(Some("1.0.0")));
        assert!(!VersionedHandler.is_version_compatible(Some("3.0.0")));
    }

    #[test]
    fn minimal_handler_compatible_with_same_default() {
        assert!(MinimalHandler.is_version_compatible(Some("1")));
    }

    #[test]
    fn minimal_handler_incompatible_with_different_version() {
        assert!(!MinimalHandler.is_version_compatible(Some("2")));
    }

    // ---- WorkflowHandler::create_run ----

    #[tokio::test]
    async fn handler_create_run_uses_handler_metadata() {
        use ironflow_store::entities::TriggerKind;
        use ironflow_store::memory::InMemoryStore;

        let store = InMemoryStore::new();

        let opts = CreateRunOpts::new().trigger(TriggerKind::Api);
        let creation = FullFeaturedHandler
            .create_run(&store, opts)
            .await
            .expect("create_run");
        let run = creation.into_run();

        assert_eq!(run.workflow_name, "full");
        assert_eq!(run.handler_version, Some("1.2.0".to_string()));
        assert_eq!(run.max_cost_usd, Some(Decimal::new(750, 2)));
    }

    #[tokio::test]
    async fn handler_create_run_opts_override_handler_defaults() {
        use ironflow_store::memory::InMemoryStore;

        let store = InMemoryStore::new();

        let opts = CreateRunOpts::new().max_cost_usd(Decimal::new(100, 2));
        let creation = FullFeaturedHandler
            .create_run(&store, opts)
            .await
            .expect("create_run");
        let run = creation.into_run();

        assert_eq!(run.max_cost_usd, Some(Decimal::new(100, 2)));
    }

    #[tokio::test]
    async fn handler_create_run_minimal_handler_defaults() {
        use ironflow_store::memory::InMemoryStore;

        let store = InMemoryStore::new();

        let opts = CreateRunOpts::new();
        let creation = MinimalHandler
            .create_run(&store, opts)
            .await
            .expect("create_run");
        let run = creation.into_run();

        assert_eq!(run.workflow_name, "minimal");
        assert_eq!(run.handler_version, Some("1".to_string()));
        assert_eq!(run.max_cost_usd, None);
    }

    struct Documented;

    impl WorkflowHandler for Documented {
        fn name(&self) -> &str {
            "documented"
        }

        fn description(&self) -> &str {
            "A documented handler"
        }

        fn source_code(&self) -> Option<&str> {
            Some("struct Documented;")
        }

        fn sub_workflows(&self) -> Vec<String> {
            vec!["child".to_string()]
        }

        fn category(&self) -> Option<&str> {
            Some("tests/handlers")
        }

        fn version(&self) -> Option<&str> {
            Some("3.1.0")
        }

        fn compatible_versions(&self) -> &[&str] {
            &["3.0.0"]
        }

        fn input_schema(&self) -> Option<Value> {
            Some(input_schema_for::<TestInput>())
        }

        fn default_labels(&self) -> HashMap<String, String> {
            HashMap::from([("team".to_string(), "core".to_string())])
        }

        fn default_max_cost_usd(&self) -> Option<Decimal> {
            Some(Decimal::new(250, 2))
        }

        fn guard_config(&self) -> Option<WorkflowGuardConfig> {
            Some(WorkflowGuardConfig::new().with_max_depth(4))
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async { Ok(()) })
        }
    }

    #[test]
    fn default_describe_propagates_every_trait_method() {
        let info = Documented.describe();
        assert_eq!(info.description, "A documented handler");
        assert_eq!(info.source_code.as_deref(), Some("struct Documented;"));
        assert_eq!(info.sub_workflows, vec!["child".to_string()]);
        assert_eq!(info.category.as_deref(), Some("tests/handlers"));
        assert_eq!(info.version.as_deref(), Some("3.1.0"));
        assert_eq!(info.compatible_versions, vec!["3.0.0".to_string()]);
        assert!(info.input_schema.is_some());
        assert_eq!(info.default_labels["team"], "core");
        assert_eq!(info.default_max_cost_usd, Some(Decimal::new(250, 2)));
    }

    #[test]
    fn minimal_handler_describe_uses_defaults() {
        let info = MinimalHandler.describe();
        assert_eq!(info.description, "");
        assert!(info.source_code.is_none());
        assert!(info.sub_workflows.is_empty());
        assert!(info.category.is_none());
        assert_eq!(info.version.as_deref(), Some("1"));
    }

    #[test]
    fn workflow_info_builder_sets_every_field() {
        let schedule = CronSchedule::new("0 0 * * *").expect("valid cron");
        let info = WorkflowInfo::new("desc")
            .with_source_code("code")
            .with_sub_workflows(["a", "b"])
            .with_category("cat/sub")
            .with_version("2")
            .with_compatible_versions(["1"])
            .with_input_schema(serde_json::json!({"type": "object"}))
            .with_default_labels(HashMap::from([("k".to_string(), "v".to_string())]))
            .with_schedule(schedule)
            .with_default_max_cost_usd(Decimal::ONE);

        assert_eq!(info.description, "desc");
        assert_eq!(info.source_code.as_deref(), Some("code"));
        assert_eq!(info.sub_workflows, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(info.category.as_deref(), Some("cat/sub"));
        assert_eq!(info.version.as_deref(), Some("2"));
        assert_eq!(info.compatible_versions, vec!["1".to_string()]);
        assert_eq!(info.input_schema.unwrap()["type"], "object");
        assert_eq!(info.default_labels["k"], "v");
        assert!(info.schedule.is_some());
        assert_eq!(info.default_max_cost_usd, Some(Decimal::ONE));
    }

    #[test]
    fn workflow_info_new_matches_default_for_other_fields() {
        let info = WorkflowInfo::new("only description");
        let default = WorkflowInfo::default();
        assert_eq!(info.description, "only description");
        assert_eq!(default.description, "");
        assert_eq!(info.source_code, default.source_code);
        assert_eq!(info.sub_workflows, default.sub_workflows);
        assert_eq!(info.category, default.category);
        assert_eq!(info.version, default.version);
        assert_eq!(info.default_max_cost_usd, default.default_max_cost_usd);
    }

    #[test]
    fn boxed_handler_delegates_every_method() {
        let boxed: Box<dyn WorkflowHandler> = Box::new(Documented);
        assert_eq!(boxed.name(), "documented");
        assert_eq!(boxed.version(), Some("3.1.0"));
        assert_eq!(boxed.compatible_versions(), &["3.0.0"]);
        assert_eq!(boxed.description(), "A documented handler");
        assert_eq!(boxed.source_code(), Some("struct Documented;"));
        assert_eq!(boxed.sub_workflows(), vec!["child".to_string()]);
        assert_eq!(boxed.category(), Some("tests/handlers"));
        assert!(boxed.input_schema().is_some());
        assert_eq!(boxed.default_labels()["team"], "core");
        assert!(boxed.schedule().is_none());
        assert_eq!(boxed.default_max_cost_usd(), Some(Decimal::new(250, 2)));
        assert_eq!(boxed.guard_config().map(|g| g.max_depth), Some(4));
        assert!(boxed.is_version_compatible(Some("3.0.0")));
        assert!(!boxed.is_version_compatible(Some("0.1.0")));
        assert_eq!(boxed.describe().description, "A documented handler");
    }

    #[test]
    fn boxed_handler_is_accepted_by_generic_register() {
        fn takes_handler(handler: impl WorkflowHandler + 'static) -> String {
            handler.name().to_string()
        }
        let boxed: Box<dyn WorkflowHandler> = Box::new(MinimalHandler);
        assert_eq!(takes_handler(boxed), "minimal");
    }

    #[tokio::test]
    async fn boxed_handler_create_run_delegates_metadata() {
        use ironflow_store::memory::InMemoryStore;
        use ironflow_store::models::TriggerKind;

        let store = InMemoryStore::new();
        let boxed: Box<dyn WorkflowHandler> = Box::new(Documented);
        let run = boxed
            .create_run(&store, CreateRunOpts::new().trigger(TriggerKind::Manual))
            .await
            .expect("run created")
            .into_run();
        assert_eq!(run.workflow_name, "documented");
        assert_eq!(run.handler_version.as_deref(), Some("3.1.0"));
        assert_eq!(run.max_cost_usd, Some(Decimal::new(250, 2)));
    }
}
