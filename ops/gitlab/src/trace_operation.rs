//! [`GitLabTraceOp`] -- reads a job trace as a tracked text [`Operation`].

use async_trait::async_trait;
use gitlab::AsyncGitlab;
use gitlab::api::{AsyncQuery, Endpoint, raw};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use ironflow_ops_common::log::condense_log;
use serde_json::{Map, Value};

use crate::trace::decode_trace;

/// How a job trace is returned by a [`GitLabTraceOp`].
///
/// # Examples
///
/// ```
/// use ironflow_ops_gitlab::TraceView;
///
/// let view = TraceView::Condensed {
///     budget: 8000,
///     marker: "[... trace truncated ...]".to_string(),
/// };
/// assert_ne!(view, TraceView::Raw);
/// assert_ne!(TraceView::Decoded, TraceView::Raw);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceView {
    /// The trace as GitLab serves it.
    Raw,
    /// The trace without ANSI sequences, runner timestamps and section markers.
    Decoded,
    /// The decoded trace without progress noise, cut under `budget` chars.
    Condensed {
        /// Maximum number of chars of the result, marker lines included.
        budget: usize,
        /// Text of the line that replaces every gap of skipped lines.
        marker: String,
    },
}

/// A job trace wrapped as an Ironflow [`Operation`] that returns text.
///
/// Created via [`GitLab::trace_op`](crate::GitLab::trace_op). The output of
/// the step is a JSON string, readable with `StepOutput::text()`. An empty
/// trace is an empty string. The view defaults to [`TraceView::Raw`]; pick
/// another one with [`decoded`](GitLabTraceOp::decoded) or
/// [`condensed`](GitLabTraceOp::condensed).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::GitLab;
/// use ironflow_ops_gitlab::endpoints::jobs::JobTrace;
/// use gitlab::api::common::NameOrId;
/// use ironflow_core::operation::Operation;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
/// let endpoint = JobTrace { project: NameOrId::from(42), job: 7 };
/// let op = gitlab.trace_op(endpoint).condensed(8000, "[... trace truncated ...]");
///
/// assert_eq!(op.kind(), "gitlab");
/// # Ok(())
/// # }
/// ```
pub struct GitLabTraceOp<E> {
    client: AsyncGitlab,
    endpoint: E,
    view: TraceView,
}

impl<E> GitLabTraceOp<E> {
    pub(crate) fn new(client: AsyncGitlab, endpoint: E) -> Self {
        Self {
            client,
            endpoint,
            view: TraceView::Raw,
        }
    }

    /// Return the trace without ANSI sequences, runner timestamps and section markers.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_gitlab::GitLab;
    /// use ironflow_ops_gitlab::endpoints::jobs::JobTrace;
    /// use gitlab::api::common::NameOrId;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
    /// let op = gitlab
    ///     .trace_op(JobTrace { project: NameOrId::from(42), job: 7 })
    ///     .decoded();
    /// # Ok(())
    /// # }
    /// ```
    pub fn decoded(mut self) -> Self {
        self.view = TraceView::Decoded;
        self
    }

    /// Return the decoded trace without progress noise, cut under `budget` chars.
    ///
    /// Failure blocks, the end of the log and the lines leading to the first
    /// failure are kept; every gap is replaced by one line holding `marker`
    /// (see [`ironflow_ops_common::log::condense_log`]).
    ///
    /// # Panics
    ///
    /// Panics if `budget` is zero.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_gitlab::GitLab;
    /// use ironflow_ops_gitlab::endpoints::jobs::JobTrace;
    /// use gitlab::api::common::NameOrId;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
    /// let op = gitlab
    ///     .trace_op(JobTrace { project: NameOrId::from(42), job: 7 })
    ///     .condensed(8000, "[... trace truncated ...]");
    /// # Ok(())
    /// # }
    /// ```
    pub fn condensed(mut self, budget: usize, marker: impl Into<String>) -> Self {
        assert!(budget > 0, "budget must be greater than zero");
        self.view = TraceView::Condensed {
            budget,
            marker: marker.into(),
        };
        self
    }
}

#[async_trait]
impl<E> Operation for GitLabTraceOp<E>
where
    E: Endpoint + Sync + Send,
{
    fn kind(&self) -> &str {
        "gitlab"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let body: Vec<u8> = raw(&self.endpoint)
            .query_async(&self.client)
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: e.to_string(),
            })?;
        // GitLab can cut a trace in the middle of a multi-byte character: a
        // lossy conversion keeps the step alive instead of failing on UTF-8.
        let trace = String::from_utf8_lossy(&body).into_owned();
        let text = match &self.view {
            TraceView::Raw => trace,
            TraceView::Decoded => decode_trace(&trace),
            TraceView::Condensed { budget, marker } => {
                condense_log(&decode_trace(&trace), *budget, marker)
            }
        };
        Ok(Value::String(text))
    }

    fn input(&self) -> Option<Value> {
        let mut input = Map::from_iter([(
            "endpoint".to_string(),
            Value::String(self.endpoint.endpoint().into_owned()),
        )]);
        let view = match &self.view {
            TraceView::Raw => "raw",
            TraceView::Decoded => "decoded",
            TraceView::Condensed { budget, .. } => {
                input.insert("budget".to_string(), Value::from(*budget));
                "condensed"
            }
        };
        input.insert("view".to_string(), Value::String(view.to_string()));
        Some(Value::Object(input))
    }
}
