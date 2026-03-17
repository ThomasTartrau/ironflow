//! Tests for Shell-centric workflows: basic, parallel, failure, timeout.

use ironflow_core::prelude::*;

use crate::helpers::*;

#[tokio::test]
async fn shell_then_agent_with_tracker() {
    let (fixtures_dir, _guard) = temp_fixtures_dir("shell-agent");

    let shell_output = Shell::new("echo 'file1.rs file2.rs file3.rs'")
        .await
        .unwrap();
    assert_eq!(shell_output.stdout(), "file1.rs file2.rs file3.rs");

    let prompt = format!("Summarize these files:\n\n{}", shell_output.stdout());
    let config = make_config(&prompt, Some("You are a concise assistant."), None);
    let agent_output = make_output(
        serde_json::json!(
            "The directory contains 3 Rust source files: file1.rs, file2.rs, and file3.rs."
        ),
        0.005,
        1000,
        50,
    );
    write_fixture(&fixtures_dir, &config, &agent_output);

    let provider = ironflow_core::providers::record_replay::RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        &fixtures_dir,
    );

    let result = Agent::new()
        .system_prompt("You are a concise assistant.")
        .prompt(&prompt)
        .model(Model::Haiku)
        .max_turns(1)
        .max_budget_usd(0.10)
        .run(&provider)
        .await
        .unwrap();

    assert!(result.text().contains("3 Rust source files"));
    assert_eq!(result.cost_usd(), Some(0.005));

    let mut tracker = WorkflowTracker::new("file-summary");
    tracker.record_shell("ls", &shell_output);
    tracker.record_agent("summarize", &result);

    assert_eq!(tracker.step_count(), 2);
    assert_eq!(tracker.total_cost_usd(), 0.005);
    assert_eq!(tracker.total_input_tokens(), 1000);
    assert_eq!(tracker.total_output_tokens(), 50);
    tracker.summary();
}

#[tokio::test]
async fn multi_shell_then_agent() {
    let (fixtures_dir, _guard) = temp_fixtures_dir("multi-shell");

    let uname = Shell::new("echo 'Linux test-host 6.1'").await.unwrap();
    let disk = Shell::new("echo 'Filesystem Size Used Avail Use%'")
        .await
        .unwrap();

    let prompt = format!(
        "System info:\nOS: {}\nDisk: {}\nSummarize in one line.",
        uname.stdout(),
        disk.stdout()
    );
    let config = make_config(&prompt, None, None);
    let agent_output = make_output(
        serde_json::json!("Linux test-host 6.1 with disk usage information available."),
        0.003,
        800,
        30,
    );
    write_fixture(&fixtures_dir, &config, &agent_output);

    let provider = ironflow_core::providers::record_replay::RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        &fixtures_dir,
    );

    let result = Agent::new()
        .prompt(&prompt)
        .model(Model::Haiku)
        .max_turns(1)
        .max_budget_usd(0.10)
        .run(&provider)
        .await
        .unwrap();

    assert!(result.text().contains("Linux"));

    let mut tracker = WorkflowTracker::new("system-report");
    tracker.record_shell("uname", &uname);
    tracker.record_shell("disk", &disk);
    tracker.record_agent("summarize", &result);

    assert_eq!(tracker.step_count(), 3);
    assert_eq!(tracker.total_cost_usd(), 0.003);
}

#[tokio::test]
async fn parallel_shells_then_agent() {
    let (fixtures_dir, _guard) = temp_fixtures_dir("parallel");

    let (cpu, mem) = tokio::join!(Shell::new("echo '4 cores'"), Shell::new("echo '16GB RAM'"),);
    let cpu = cpu.unwrap();
    let mem = mem.unwrap();

    assert_eq!(cpu.stdout(), "4 cores");
    assert_eq!(mem.stdout(), "16GB RAM");

    let prompt = format!(
        "CPU: {}\nMemory: {}\nSummarize.",
        cpu.stdout(),
        mem.stdout()
    );
    let config = make_config(&prompt, None, None);
    let agent_output = make_output(
        serde_json::json!("System has 4 CPU cores and 16GB of RAM."),
        0.004,
        900,
        35,
    );
    write_fixture(&fixtures_dir, &config, &agent_output);

    let provider = ironflow_core::providers::record_replay::RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        &fixtures_dir,
    );

    let result = Agent::new()
        .prompt(&prompt)
        .model(Model::Haiku)
        .max_turns(1)
        .max_budget_usd(0.10)
        .run(&provider)
        .await
        .unwrap();

    assert!(result.text().contains("4 CPU cores"));
    assert!(result.text().contains("16GB"));
}
