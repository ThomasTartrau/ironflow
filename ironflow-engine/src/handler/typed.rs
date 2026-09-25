//! [`TypedWorkflow`] -- a handler whose input payload has a declared type.

use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{WorkflowHandler, input_schema_for};

/// A [`WorkflowHandler`] whose input payload has a declared type.
///
/// Implement it on a handler that is called as a sub-workflow:
/// [`WorkflowContext::workflow`](crate::context::WorkflowContext::workflow)
/// then only accepts a [`Self::Input`] for it, so a parent cannot pass a
/// misspelled or incomplete payload. Handlers that are never called as a
/// sub-workflow do not need it.
///
/// [`typed_input_schema`](Self::typed_input_schema) derives the JSON Schema of
/// the input from the same type, for [`WorkflowHandler::input_schema`].
///
/// # Examples
///
/// ```no_run
/// use ironflow_engine::config::ShellConfig;
/// use ironflow_engine::context::WorkflowContext;
/// use ironflow_engine::error::EngineError;
/// use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
/// use schemars::JsonSchema;
/// use serde::{Deserialize, Serialize};
/// use serde_json::Value;
///
/// #[derive(Serialize, Deserialize, JsonSchema)]
/// struct CollectInput {
///     host: String,
/// }
///
/// struct Collect;
///
/// impl WorkflowHandler for Collect {
///     fn name(&self) -> &str { "collect" }
///     fn input_schema(&self) -> Option<Value> { Self::typed_input_schema() }
///     fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
///         Box::pin(async move {
///             let input: CollectInput = ctx.input().await?;
///             ctx.shell("uptime", ShellConfig::new("ssh \"$HOST\" uptime").env("HOST", &input.host))
///                 .await?;
///             Ok(())
///         })
///     }
/// }
///
/// impl TypedWorkflow for Collect {
///     type Input = CollectInput;
/// }
///
/// # async fn parent(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
/// let child = ctx.workflow(&Collect, CollectInput { host: "db-1".to_string() }).await?;
/// println!("collected by run {}", child.run_id());
/// # Ok(())
/// # }
/// ```
pub trait TypedWorkflow: WorkflowHandler {
    /// The payload a run of this workflow is started with.
    type Input: Serialize + DeserializeOwned + Send;

    /// JSON Schema of [`Self::Input`], ready to return from
    /// [`WorkflowHandler::input_schema`].
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
    /// use schemars::JsonSchema;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Serialize, Deserialize, JsonSchema)]
    /// struct DeployInput {
    ///     environment: String,
    /// }
    ///
    /// struct Deploy;
    ///
    /// impl WorkflowHandler for Deploy {
    ///     fn name(&self) -> &str { "deploy" }
    ///     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    ///         Box::pin(async move { Ok(()) })
    ///     }
    /// }
    ///
    /// impl TypedWorkflow for Deploy {
    ///     type Input = DeployInput;
    /// }
    ///
    /// let schema = Deploy::typed_input_schema().expect("a schema");
    /// assert!(schema["properties"]["environment"].is_object());
    /// ```
    fn typed_input_schema() -> Option<Value>
    where
        Self: Sized,
        Self::Input: JsonSchema,
    {
        Some(input_schema_for::<Self::Input>())
    }
}

/// Names of the given handlers, for [`WorkflowHandler::sub_workflows`].
///
/// Pass the handlers themselves rather than their names: a misspelled handler
/// does not compile, a misspelled string does.
///
/// # Examples
///
/// ```
/// use ironflow_engine::context::WorkflowContext;
/// use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, sub_workflow_names};
///
/// struct Collect;
///
/// impl WorkflowHandler for Collect {
///     fn name(&self) -> &str { "collect" }
///     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
///         Box::pin(async move { Ok(()) })
///     }
/// }
///
/// struct Enrich;
///
/// impl WorkflowHandler for Enrich {
///     fn name(&self) -> &str { "enrich" }
///     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
///         Box::pin(async move { Ok(()) })
///     }
/// }
///
/// assert_eq!(sub_workflow_names(&[&Collect, &Enrich]), vec!["collect", "enrich"]);
/// assert!(sub_workflow_names(&[]).is_empty());
/// ```
pub fn sub_workflow_names(handlers: &[&dyn WorkflowHandler]) -> Vec<String> {
    handlers.iter().map(|h| h.name().to_string()).collect()
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;
    use crate::context::WorkflowContext;
    use crate::handler::HandlerFuture;

    #[derive(Serialize, Deserialize, JsonSchema)]
    struct ProbeInput {
        count: u32,
        label: Option<String>,
    }

    struct Probe;

    impl WorkflowHandler for Probe {
        fn name(&self) -> &str {
            "probe"
        }

        fn input_schema(&self) -> Option<Value> {
            Self::typed_input_schema()
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    impl TypedWorkflow for Probe {
        type Input = ProbeInput;
    }

    struct Other;

    impl WorkflowHandler for Other {
        fn name(&self) -> &str {
            "other-été"
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    #[test]
    fn typed_input_schema_describes_the_input_type() {
        let schema = Probe::typed_input_schema().expect("a schema");
        assert_eq!(schema, input_schema_for::<ProbeInput>());
        assert!(schema["properties"]["count"].is_object());
        assert!(schema["properties"]["label"].is_object());
    }

    #[test]
    fn input_schema_can_return_the_typed_schema() {
        assert_eq!(Probe.input_schema(), Probe::typed_input_schema());
        assert_eq!(Probe.describe().input_schema, Probe::typed_input_schema());
    }

    #[test]
    fn sub_workflow_names_keeps_the_order_of_the_handlers() {
        assert_eq!(
            sub_workflow_names(&[&Other, &Probe]),
            vec!["other-été".to_string(), "probe".to_string()]
        );
    }

    #[test]
    fn sub_workflow_names_of_nothing_is_empty() {
        assert!(sub_workflow_names(&[]).is_empty());
    }
}
