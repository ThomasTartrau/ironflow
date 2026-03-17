use ironflow_core::prelude::*;
use serde_json::json;

use crate::helpers::{make_config, make_output, temp_fixtures_dir, write_fixture};

#[tokio::test]
async fn shell_dry_run_returns_synthetic_output() {
    let output = Shell::new("echo hello").dry_run(true).run().await.unwrap();
    assert_eq!(output.stdout(), "");
    assert_eq!(output.stderr(), "");
    assert_eq!(output.exit_code(), 0);
    assert_eq!(output.duration_ms(), 0);
}

#[tokio::test]
async fn shell_dry_run_false_runs_normally() {
    let output = Shell::new("echo hi").dry_run(false).run().await.unwrap();
    assert!(output.stdout().contains("hi"));
}

#[tokio::test]
async fn http_dry_run_returns_synthetic_200() {
    let output = Http::get("http://localhost:99999/does-not-exist")
        .dry_run(true)
        .run()
        .await
        .unwrap();
    assert_eq!(output.status(), 200);
    assert_eq!(output.body(), "");
    assert!(output.is_success());
    assert_eq!(output.duration_ms(), 0);
}

#[tokio::test]
async fn agent_dry_run_returns_synthetic_result() {
    let (dir, _guard) = temp_fixtures_dir("dry-run-agent");
    let config = make_config("dry-run prompt", None, None);
    let output = make_output(json!("ok"), 0.0, 0, 0);
    write_fixture(&dir, &config, &output);

    let provider = RecordReplayProvider::replay(ClaudeCodeProvider::new(), &dir);
    let result = Agent::new()
        .prompt("dry-run prompt")
        .dry_run(true)
        .run(&provider)
        .await
        .unwrap();

    assert_eq!(result.text(), "[dry-run] agent call skipped");
    assert_eq!(result.cost_usd(), Some(0.0));
    assert_eq!(result.input_tokens(), Some(0));
    assert_eq!(result.output_tokens(), Some(0));
    assert_eq!(result.duration_ms(), 0);
}

// NOTE: Global dry-run tests (set_dry_run) live in dry_run.rs unit tests only,
// because the global AtomicBool leaks across parallel test threads.
