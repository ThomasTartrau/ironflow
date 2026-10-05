//! Canned step results and the closure-backed doubles that serve them.
//!
//! [`MockInterceptor`] answers shell and HTTP steps from closures; agent steps
//! are answered one seam lower, by [`MockAgentProvider`] or
//! [`MissingAgentProvider`]. Everything here mirrors the shape the real
//! executors persist, so a handler cannot tell the difference.
//!
//! The value types are defined locally rather than reusing
//! [`ShellOutput`](ironflow_core::operations::shell::ShellOutput) and
//! [`HttpOutput`](ironflow_core::operations::http::HttpOutput): those have
//! private fields and no public constructor.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use rust_decimal::Decimal;
use serde_json::{Value, json, to_string};

use ironflow_core::error::{AgentError, OperationError};
use ironflow_core::provider::{AgentConfig, AgentOutput, AgentProvider, InvokeFuture};

use crate::config::{ApprovalConfig, HttpConfig, HumanInputConfig, ShellConfig, StepConfig};
use crate::error::EngineError;
use crate::executor::{
    ApprovalOutcome, HumanInputOutcome, SignalOutcome, StepArtifacts, StepInterceptor, StepOutput,
};

/// Message carried by [`MissingAgentProvider`] failures.
const MISSING_AGENT_PROVIDER: &str = "TestEngine has no agent provider: call with_mock_agent(...), with_recorded_agent(...) or \
     with_agent_provider(...)";

/// Canned result of a mocked shell step.
///
/// # Examples
///
/// ```
/// use ironflow_engine::testing::MockShellOutput;
///
/// let ok = MockShellOutput::ok("built 3 crates");
/// assert_eq!(ok.exit_code, 0);
///
/// let ko = MockShellOutput::failed(2, "linker not found");
/// assert_eq!(ko.stderr, "linker not found");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MockShellOutput {
    /// Standard output the step reports.
    pub stdout: String,
    /// Standard error the step reports.
    pub stderr: String,
    /// Process exit code. Anything but `0` fails the step.
    pub exit_code: i32,
}

impl MockShellOutput {
    /// A successful command that printed `stdout`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::testing::MockShellOutput;
    ///
    /// let output = MockShellOutput::ok("ok\n");
    /// assert_eq!(output.stdout, "ok\n");
    /// assert!(output.stderr.is_empty());
    /// ```
    pub fn ok(stdout: &str) -> Self {
        Self {
            stdout: stdout.to_string(),
            ..Self::default()
        }
    }

    /// A command that exited with `exit_code` after printing `stderr`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::testing::MockShellOutput;
    ///
    /// let output = MockShellOutput::failed(127, "command not found");
    /// assert_eq!(output.exit_code, 127);
    /// ```
    pub fn failed(exit_code: i32, stderr: &str) -> Self {
        Self {
            stdout: String::new(),
            stderr: stderr.to_string(),
            exit_code,
        }
    }

    /// Convert to what the step lifecycle expects.
    ///
    /// Mirrors [`ShellExecutor`](crate::executor::ShellExecutor): a non-zero
    /// exit code is an error, not an output, unless the step set
    /// `exit_code_as_output`. `allow_failure`, step retry policies and run
    /// failure all key off that error.
    pub(crate) fn into_step_result(
        self,
        exit_code_as_output: bool,
    ) -> Result<StepOutput, EngineError> {
        if self.exit_code != 0 && !exit_code_as_output {
            return Err(EngineError::Operation(OperationError::Shell {
                exit_code: self.exit_code,
                stderr: self.stderr,
            }));
        }

        Ok(StepOutput {
            output: json!({
                "stdout": self.stdout,
                "stderr": self.stderr,
                "exit_code": self.exit_code,
            }),
            duration_ms: 0,
            cost_usd: Decimal::ZERO,
            input_tokens: None,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            output_tokens: None,
            model: None,
            debug_messages: None,
            artifacts: StepArtifacts::default(),
            account_id: None,
            environment_id: None,
        })
    }
}

/// Canned response of a mocked HTTP step.
///
/// A non-2xx status is *not* an error, exactly like the real
/// [`HttpExecutor`](crate::executor::HttpExecutor): the status lands in the
/// step output. A transport failure is expressed by returning
/// `Err(OperationError::Http { status: None, .. })` from the mock closure.
///
/// # Examples
///
/// ```
/// use ironflow_engine::testing::MockHttpResponse;
/// use serde_json::json;
///
/// let created = MockHttpResponse::json(201, &json!({"id": 7}))
///     .header("location", "/things/7");
/// assert_eq!(created.status, 201);
/// assert_eq!(created.headers, vec![("location".to_string(), "/things/7".to_string())]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockHttpResponse {
    /// HTTP status code the step reports.
    pub status: u16,
    /// Response headers, in insertion order.
    pub headers: Vec<(String, String)>,
    /// Raw response body.
    pub body: String,
}

impl Default for MockHttpResponse {
    fn default() -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: String::new(),
        }
    }
}

impl MockHttpResponse {
    /// A `200 OK` carrying `body` serialized as JSON.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::testing::MockHttpResponse;
    /// use serde_json::json;
    ///
    /// let response = MockHttpResponse::ok(&json!({"ok": true}));
    /// assert_eq!(response.status, 200);
    /// assert_eq!(response.body, r#"{"ok":true}"#);
    /// ```
    pub fn ok(body: &Value) -> Self {
        Self::json(200, body)
    }

    /// A response with the given status carrying `body` serialized as JSON.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::testing::MockHttpResponse;
    /// use serde_json::json;
    ///
    /// let response = MockHttpResponse::json(404, &json!({"error": "not found"}));
    /// assert_eq!(response.status, 404);
    /// ```
    pub fn json(status: u16, body: &Value) -> Self {
        Self {
            status,
            headers: Vec::new(),
            // `Value` always serializes; the fallback keeps the mock infallible.
            body: to_string(body).unwrap_or_else(|_| body.to_string()),
        }
    }

    /// A response with the given status carrying a raw text body.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::testing::MockHttpResponse;
    ///
    /// let response = MockHttpResponse::text(503, "upstream is down");
    /// assert_eq!(response.body, "upstream is down");
    /// ```
    pub fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.to_string(),
        }
    }

    /// Add a response header.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::testing::MockHttpResponse;
    ///
    /// let response = MockHttpResponse::text(200, "pong").header("x-trace", "abc");
    /// assert_eq!(response.headers.len(), 1);
    /// ```
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// Convert to the exact shape [`HttpExecutor`](crate::executor::HttpExecutor)
    /// persists.
    pub(crate) fn into_step_output(self) -> StepOutput {
        let headers: BTreeMap<String, String> = self.headers.into_iter().collect();
        StepOutput {
            output: json!({
                "status": self.status,
                "headers": headers,
                "body": self.body,
            }),
            duration_ms: 0,
            cost_usd: Decimal::ZERO,
            input_tokens: None,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            output_tokens: None,
            model: None,
            debug_messages: None,
            artifacts: StepArtifacts::default(),
            account_id: None,
            environment_id: None,
        }
    }
}

/// Closure answering a shell step from its config.
pub type ShellMock =
    Arc<dyn Fn(&ShellConfig) -> Result<MockShellOutput, OperationError> + Send + Sync>;

/// Closure answering an HTTP step from its config.
pub type HttpMock =
    Arc<dyn Fn(&HttpConfig) -> Result<MockHttpResponse, OperationError> + Send + Sync>;

/// Closure answering a human input step from its name and config.
pub type HumanInputMock = Arc<dyn Fn(&str, &HumanInputConfig) -> HumanInputOutcome + Send + Sync>;

/// Closure answering a signal step from its step name, signal name and key.
pub type SignalMock = Arc<dyn Fn(&str, &str, &str) -> SignalOutcome + Send + Sync>;

/// Closure answering an agent invocation from its config.
pub type AgentMock = Arc<dyn Fn(&AgentConfig) -> Result<AgentOutput, AgentError> + Send + Sync>;

/// A [`StepInterceptor`] built from closures.
///
/// Built by [`TestEngine`](crate::testing::TestEngine); a step kind with no
/// mock attached falls through to the real executor.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::{ShellConfig, StepConfig};
/// use ironflow_engine::executor::StepInterceptor;
/// use ironflow_engine::testing::{MockInterceptor, MockShellOutput};
///
/// let interceptor = MockInterceptor::new().shell(|_cfg| Ok(MockShellOutput::ok("mocked")));
/// let config = StepConfig::Shell(ShellConfig::new("./deploy.sh"));
///
/// let output = interceptor
///     .intercept(&config)
///     .expect("shell steps are mocked")
///     .expect("the mock succeeded");
/// assert_eq!(output.stdout(), "mocked");
/// ```
#[derive(Clone, Default)]
pub struct MockInterceptor {
    shell: Option<ShellMock>,
    http: Option<HttpMock>,
    approval: Option<ApprovalOutcome>,
    human_input: Option<HumanInputMock>,
    signal: Option<SignalMock>,
}

impl MockInterceptor {
    /// An interceptor that mocks nothing.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::{ShellConfig, StepConfig};
    /// use ironflow_engine::executor::StepInterceptor;
    /// use ironflow_engine::testing::MockInterceptor;
    ///
    /// let interceptor = MockInterceptor::new();
    /// let config = StepConfig::Shell(ShellConfig::new("echo hi"));
    /// assert!(interceptor.intercept(&config).is_none());
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Answer every shell step with `f`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::testing::{MockInterceptor, MockShellOutput};
    ///
    /// let interceptor = MockInterceptor::new()
    ///     .shell(|cfg| Ok(MockShellOutput::ok(&format!("ran {}", cfg.command))));
    /// # let _ = interceptor;
    /// ```
    pub fn shell(
        mut self,
        f: impl Fn(&ShellConfig) -> Result<MockShellOutput, OperationError> + Send + Sync + 'static,
    ) -> Self {
        self.shell = Some(Arc::new(f));
        self
    }

    /// Answer every HTTP step with `f`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::testing::{MockHttpResponse, MockInterceptor};
    /// use serde_json::json;
    ///
    /// let interceptor = MockInterceptor::new()
    ///     .http(|_cfg| Ok(MockHttpResponse::ok(&json!({"ok": true}))));
    /// # let _ = interceptor;
    /// ```
    pub fn http(
        mut self,
        f: impl Fn(&HttpConfig) -> Result<MockHttpResponse, OperationError> + Send + Sync + 'static,
    ) -> Self {
        self.http = Some(Arc::new(f));
        self
    }

    /// Resolve every approval gate with `outcome`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::testing::{ApprovalOutcome, MockInterceptor};
    ///
    /// let interceptor = MockInterceptor::new().approval(ApprovalOutcome::Approved);
    /// # let _ = interceptor;
    /// ```
    pub fn approval(mut self, outcome: ApprovalOutcome) -> Self {
        self.approval = Some(outcome);
        self
    }

    /// Answer every human input step with `f`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::testing::{HumanInputOutcome, MockInterceptor};
    /// use serde_json::json;
    ///
    /// let interceptor = MockInterceptor::new()
    ///     .human_input(|_name, _cfg| HumanInputOutcome::Provided(json!({"answers": ["yes"]})));
    /// # let _ = interceptor;
    /// ```
    pub fn human_input(
        mut self,
        f: impl Fn(&str, &HumanInputConfig) -> HumanInputOutcome + Send + Sync + 'static,
    ) -> Self {
        self.human_input = Some(Arc::new(f));
        self
    }

    /// Resolve every signal step with `f`.
    ///
    /// `f` receives the step name, the signal name and the key.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::testing::{MockInterceptor, SignalOutcome};
    /// use serde_json::json;
    ///
    /// let interceptor = MockInterceptor::new()
    ///     .signal(|_step, _name, _key| SignalOutcome::Received(json!({"status": "success"})));
    /// # let _ = interceptor;
    /// ```
    pub fn signal(
        mut self,
        f: impl Fn(&str, &str, &str) -> SignalOutcome + Send + Sync + 'static,
    ) -> Self {
        self.signal = Some(Arc::new(f));
        self
    }
}

impl fmt::Debug for MockInterceptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Closures are not `Debug`: report which seams are mocked instead.
        f.debug_struct("MockInterceptor")
            .field("shell", &self.shell.is_some())
            .field("http", &self.http.is_some())
            .field("approval", &self.approval)
            .field("human_input", &self.human_input.is_some())
            .field("signal", &self.signal.is_some())
            .finish()
    }
}

impl StepInterceptor for MockInterceptor {
    fn intercept(&self, config: &StepConfig) -> Option<Result<StepOutput, EngineError>> {
        match config {
            StepConfig::Shell(cfg) => {
                let mock = self.shell.as_ref()?;
                Some(match mock(cfg) {
                    Ok(out) => out.into_step_result(cfg.exit_code_as_output),
                    Err(err) => Err(EngineError::Operation(err)),
                })
            }
            StepConfig::Http(cfg) => {
                let mock = self.http.as_ref()?;
                Some(match mock(cfg) {
                    Ok(res) => Ok(res.into_step_output()),
                    Err(err) => Err(EngineError::Operation(err)),
                })
            }
            // Agent steps are mocked at the provider seam instead.
            _ => None,
        }
    }

    fn intercept_approval(&self, _name: &str, _config: &ApprovalConfig) -> Option<ApprovalOutcome> {
        self.approval.clone()
    }

    fn intercept_human_input(
        &self,
        name: &str,
        config: &HumanInputConfig,
        _schema: &Value,
    ) -> Option<HumanInputOutcome> {
        self.human_input.as_ref().map(|f| f(name, config))
    }

    fn intercept_signal(
        &self,
        name: &str,
        signal_name: &str,
        key: &str,
        _schema: &Value,
    ) -> Option<SignalOutcome> {
        self.signal.as_ref().map(|f| f(name, signal_name, key))
    }
}

/// An [`AgentProvider`] backed by a closure.
///
/// # Examples
///
/// ```
/// use ironflow_core::provider::AgentOutput;
/// use ironflow_engine::testing::MockAgentProvider;
/// use serde_json::json;
///
/// let provider = MockAgentProvider::new(|cfg| {
///     assert!(cfg.prompt.contains("review"));
///     Ok(AgentOutput::new(json!({"score": 9})))
/// });
/// # let _ = provider;
/// ```
pub struct MockAgentProvider {
    f: AgentMock,
}

impl MockAgentProvider {
    /// Answer every invocation with `f`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentOutput;
    /// use ironflow_engine::testing::MockAgentProvider;
    /// use serde_json::json;
    ///
    /// let provider = MockAgentProvider::new(|_cfg| Ok(AgentOutput::new(json!("done"))));
    /// # let _ = provider;
    /// ```
    pub fn new(
        f: impl Fn(&AgentConfig) -> Result<AgentOutput, AgentError> + Send + Sync + 'static,
    ) -> Self {
        Self { f: Arc::new(f) }
    }
}

impl fmt::Debug for MockAgentProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MockAgentProvider").finish_non_exhaustive()
    }
}

impl AgentProvider for MockAgentProvider {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        let result = (self.f)(config);
        Box::pin(async move { result })
    }
}

/// The provider a [`TestEngine`](crate::testing::TestEngine) uses when no agent
/// backend was configured.
///
/// Every invocation fails with an explanation instead of reaching the Claude
/// CLI, so a forgotten `with_mock_agent` is a loud test failure, not a silent
/// network call.
///
/// # Examples
///
/// ```
/// use ironflow_engine::testing::MissingAgentProvider;
///
/// // Usable anywhere an `AgentProvider` is expected, including as the inner
/// // provider of a `RecordReplayProvider` in replay mode.
/// let provider = MissingAgentProvider;
/// assert_eq!(format!("{provider:?}"), "MissingAgentProvider");
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct MissingAgentProvider;

impl AgentProvider for MissingAgentProvider {
    fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            Err(AgentError::ProcessFailed {
                exit_code: -1,
                stderr: MISSING_AGENT_PROVIDER.to_string(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::AgentStepConfig;

    #[test]
    fn shell_ok_maps_to_the_real_executor_output_shape() {
        let output = MockShellOutput::ok("hello\n")
            .into_step_result(false)
            .expect("exit code 0 succeeds");

        assert_eq!(output.output["stdout"], "hello\n");
        assert_eq!(output.output["stderr"], "");
        assert_eq!(output.output["exit_code"], 0);
        assert_eq!(output.cost_usd, Decimal::ZERO);
    }

    #[test]
    fn shell_default_is_an_empty_success() {
        let default = MockShellOutput::default();
        assert_eq!(default.exit_code, 0);
        assert!(default.stdout.is_empty());
        assert!(default.stderr.is_empty());
    }

    #[test]
    fn shell_non_zero_exit_is_an_operation_error() {
        let err = MockShellOutput::failed(2, "x")
            .into_step_result(false)
            .expect_err("a non-zero exit code fails the step");

        match err {
            EngineError::Operation(OperationError::Shell { exit_code, stderr }) => {
                assert_eq!(exit_code, 2);
                assert_eq!(stderr, "x");
            }
            other => panic!("expected a shell operation error, got {other}"),
        }
    }

    #[test]
    fn shell_non_zero_exit_with_option_is_an_output() {
        let output = MockShellOutput::failed(2, "x")
            .into_step_result(true)
            .expect("the option turns a non-zero exit into an output");

        assert_eq!(output.output["exit_code"], 2);
        assert_eq!(output.output["stderr"], "x");
        assert!(!output.is_success());
    }

    #[test]
    fn intercept_shell_non_zero_with_option_completes() {
        let interceptor = MockInterceptor::new().shell(|_| Ok(MockShellOutput::failed(2, "x")));
        let config = StepConfig::Shell(ShellConfig::new("x").exit_code_as_output());

        let output = interceptor
            .intercept(&config)
            .expect("the shell mock answers")
            .expect("the step completes");
        assert_eq!(output.exit_code(), Some(2));

        let plain = StepConfig::Shell(ShellConfig::new("x"));
        assert!(
            interceptor
                .intercept(&plain)
                .expect("the shell mock answers")
                .is_err()
        );
    }

    #[test]
    fn http_json_carries_status_body_and_headers() {
        let output = MockHttpResponse::json(201, &json!({"id": 7}))
            .header("location", "/things/7")
            .into_step_output();

        assert_eq!(output.output["status"], 201);
        assert_eq!(output.output["body"], r#"{"id":7}"#);
        assert_eq!(output.output["headers"]["location"], "/things/7");
    }

    #[test]
    fn http_default_is_an_empty_200() {
        let default = MockHttpResponse::default();
        assert_eq!(default.status, 200);
        assert!(default.body.is_empty());
        assert!(default.headers.is_empty());
    }

    #[test]
    fn http_non_2xx_is_still_an_output() {
        let output = MockHttpResponse::text(500, "boom").into_step_output();
        assert_eq!(output.status(), Some(500));
        assert_eq!(output.body(), "boom");
    }

    #[test]
    fn intercept_declines_agent_steps() {
        let interceptor = MockInterceptor::new().shell(|_| Ok(MockShellOutput::ok("x")));
        let config = StepConfig::Agent(AgentStepConfig::new("review this"));

        assert!(interceptor.intercept(&config).is_none());
    }

    #[test]
    fn intercept_declines_shell_steps_without_a_shell_mock() {
        let interceptor = MockInterceptor::new();
        let config = StepConfig::Shell(ShellConfig::new("echo hi"));

        assert!(interceptor.intercept(&config).is_none());
    }

    #[test]
    fn intercept_approval_returns_the_configured_outcome() {
        let interceptor = MockInterceptor::new().approval(ApprovalOutcome::reject("nope"));
        let config = ApprovalConfig::new("Approve?");

        assert_eq!(
            interceptor.intercept_approval("gate", &config),
            Some(ApprovalOutcome::reject("nope"))
        );
        assert_eq!(
            MockInterceptor::new().intercept_approval("gate", &config),
            None
        );
    }

    #[test]
    fn intercept_human_input_returns_the_mocked_answer() {
        let interceptor = MockInterceptor::new().human_input(|name, cfg| {
            HumanInputOutcome::Provided(json!({"step": name, "message": cfg.message()}))
        });
        let config = HumanInputConfig::new("Answer?");
        let answer = json!({"step": "clarify", "message": "Answer?"});

        assert_eq!(
            interceptor.intercept_human_input("clarify", &config, &json!({})),
            Some(HumanInputOutcome::Provided(answer))
        );
    }

    #[test]
    fn intercept_human_input_declines_without_a_mock() {
        let config = HumanInputConfig::new("Answer?");

        assert_eq!(
            MockInterceptor::new().intercept_human_input("clarify", &config, &json!({})),
            None
        );
    }

    #[test]
    fn intercept_signal_returns_the_mocked_outcome() {
        let interceptor = MockInterceptor::new().signal(|step, name, key| {
            SignalOutcome::Received(json!({"step": step, "name": name, "key": key}))
        });
        let expected = json!({"step": "wait-ci", "name": "ci.done", "key": "abc"});

        assert_eq!(
            interceptor.intercept_signal("wait-ci", "ci.done", "abc", &json!({})),
            Some(SignalOutcome::Received(expected))
        );
        assert_eq!(
            MockInterceptor::new().intercept_signal("wait-ci", "ci.done", "abc", &json!({})),
            None
        );
    }

    #[test]
    fn debug_reports_which_seams_are_mocked() {
        let interceptor = MockInterceptor::new().http(|_| Ok(MockHttpResponse::default()));
        let rendered = format!("{interceptor:?}");

        assert!(rendered.contains("shell: false"));
        assert!(rendered.contains("http: true"));
    }

    #[tokio::test]
    async fn missing_agent_provider_names_the_three_constructors() {
        let config = AgentConfig::new("anything");
        let err = MissingAgentProvider
            .invoke(&config)
            .await
            .expect_err("no agent backend is configured");

        let message = err.to_string();
        assert!(message.contains("with_mock_agent"));
        assert!(message.contains("with_recorded_agent"));
        assert!(message.contains("with_agent_provider"));
    }

    #[tokio::test]
    async fn mock_agent_provider_runs_the_closure() {
        let provider = MockAgentProvider::new(|cfg| {
            let echoed = json!({"echoed": cfg.prompt.clone()});
            Ok(AgentOutput::new(echoed))
        });
        let config = AgentConfig::new("say hi");

        let output = provider.invoke(&config).await.expect("the mock succeeded");

        assert_eq!(output.value["echoed"], "say hi");
    }
}
