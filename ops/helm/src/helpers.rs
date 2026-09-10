//! Internal helpers for spawning the Helm CLI and mapping errors.

use std::process::{Output, Stdio};

use ironflow_core::error::OperationError;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::client::HelmClient;

pub(crate) fn helm_error(message: impl Into<String>) -> OperationError {
    OperationError::External {
        origin: "helm".to_string(),
        message: message.into(),
    }
}

pub(crate) fn to_value<T: Serialize>(v: &T) -> Result<Value, OperationError> {
    serde_json::to_value(v).map_err(|e| helm_error(e.to_string()))
}

fn build_command(client: &HelmClient, args: &[&str]) -> Command {
    let mut cmd = Command::new(client.binary());
    if let Some(kubeconfig) = client.kubeconfig() {
        cmd.arg("--kubeconfig").arg(kubeconfig);
    }
    if let Some(namespace) = client.namespace() {
        cmd.arg("--namespace").arg(namespace);
    }
    cmd.args(args);
    cmd
}

fn check_output(output: &Output) -> Result<String, OperationError> {
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        let code = output.status.code().unwrap_or(-1);
        Err(OperationError::Shell {
            exit_code: code,
            stderr,
        })
    }
}

/// Run a Helm command and return the raw stdout as a string.
pub(crate) async fn run_helm(client: &HelmClient, args: &[&str]) -> Result<String, OperationError> {
    let output = build_command(client, args)
        .output()
        .await
        .map_err(|e| helm_error(format!("failed to spawn helm: {e}")))?;
    check_output(&output)
}

/// Run a Helm command with `--output json` and deserialize the JSON output.
pub(crate) async fn run_helm_json<T: DeserializeOwned>(
    client: &HelmClient,
    args: &[&str],
) -> Result<T, OperationError> {
    let mut full_args: Vec<&str> = args.to_vec();
    full_args.push("--output");
    full_args.push("json");
    let stdout = run_helm(client, &full_args).await?;
    serde_json::from_str(&stdout)
        .map_err(|e| helm_error(format!("failed to parse helm JSON output: {e}")))
}

/// Build a command with extra arguments appended after the base args.
pub(crate) async fn run_helm_with_extra(
    client: &HelmClient,
    base_args: &[&str],
    extra_args: &[String],
) -> Result<String, OperationError> {
    let mut cmd = build_command(client, base_args);
    for arg in extra_args {
        cmd.arg(arg);
    }
    let output = cmd
        .output()
        .await
        .map_err(|e| helm_error(format!("failed to spawn helm: {e}")))?;
    check_output(&output)
}

/// Run a Helm command with data piped to stdin.
pub(crate) async fn run_helm_with_stdin(
    client: &HelmClient,
    args: &[&str],
    extra_args: &[String],
    stdin_data: &str,
) -> Result<String, OperationError> {
    let mut cmd = build_command(client, args);
    for arg in extra_args {
        cmd.arg(arg);
    }
    cmd.stdin(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| helm_error(format!("failed to spawn helm: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_data.as_bytes())
            .await
            .map_err(|e| helm_error(format!("failed to write to helm stdin: {e}")))?;
    }
    let output = child
        .wait_with_output()
        .await
        .map_err(|e| helm_error(format!("failed to wait for helm: {e}")))?;
    check_output(&output)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn fake_client() -> HelmClient {
        HelmClient::new("echo", None::<String>, None::<String>)
    }

    fn nonexistent_client() -> HelmClient {
        HelmClient::new(
            "/nonexistent/helm-binary-12345",
            None::<String>,
            None::<String>,
        )
    }

    #[tokio::test]
    async fn run_helm_returns_stdout() {
        let client = fake_client();
        let result = run_helm(&client, &["hello", "world"]).await.unwrap();
        assert!(result.contains("hello"));
        assert!(result.contains("world"));
    }

    #[tokio::test]
    async fn run_helm_returns_shell_error_on_failure() {
        let client = HelmClient::new("false", None::<String>, None::<String>);
        let err = run_helm(&client, &[]).await.unwrap_err();
        match err {
            OperationError::Shell { exit_code, .. } => {
                assert_ne!(exit_code, 0);
            }
            other => panic!("expected Shell error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_helm_returns_error_on_missing_binary() {
        let client = nonexistent_client();
        let err = run_helm(&client, &["version"]).await.unwrap_err();
        let msg = err.to_string();
        // "failed to spawn helm" when spawn() itself fails (macOS);
        // exit code 127 when fork succeeds but exec fails (Linux).
        assert!(
            msg.contains("failed to spawn helm") || msg.contains("127"),
            "expected spawn or command-not-found error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn run_helm_json_parses_output() {
        let client = HelmClient::new("echo", None::<String>, None::<String>);
        // echo will print the args including --output json, which is not valid JSON
        let err = run_helm_json::<Value>(&client, &["test"]).await;
        assert!(err.is_err(), "echo output is not valid JSON");
    }

    #[tokio::test]
    async fn to_value_serializes() {
        let val = to_value(&"hello").unwrap();
        assert_eq!(val, Value::String("hello".to_string()));
    }

    #[tokio::test]
    async fn helm_error_has_correct_origin() {
        let err = helm_error("test message");
        let msg = err.to_string();
        assert!(msg.contains("helm error"), "got: {msg}");
        assert!(msg.contains("test message"), "got: {msg}");
    }
}
