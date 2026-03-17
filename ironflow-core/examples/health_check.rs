//! Parallel project health check.
//!
//! Runs compilation, linting, tests, and formatting checks concurrently,
//! then aggregates the results. Demonstrates both static parallelism
//! (`tokio::try_join!`) and dynamic, concurrency-limited parallelism
//! (`try_join_all_limited`).
//!
//! ```bash
//! cargo run --example health_check
//! ```

use std::time::Duration;

use ironflow_core::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json()
        .init();

    let mut tracker = WorkflowTracker::new("health-check");

    // --- Phase 1: core checks in parallel ---
    eprintln!("Running core checks...");

    let (build, clippy, fmt) = tokio::try_join!(
        Shell::new("cargo check --message-format=short 2>&1")
            .timeout(Duration::from_secs(120))
            .run(),
        Shell::new("cargo clippy --message-format=short -- -D warnings 2>&1 || true")
            .timeout(Duration::from_secs(120))
            .run(),
        Shell::new("cargo fmt --check 2>&1 || true")
            .timeout(Duration::from_secs(30))
            .run(),
    )?;
    tracker.record_shell("build", &build);
    tracker.record_shell("clippy", &clippy);
    tracker.record_shell("fmt", &fmt);

    print_check("build", build.exit_code() == 0, &build);
    print_check("clippy", !clippy.stdout().contains("warning"), &clippy);
    print_check("fmt", fmt.exit_code() == 0, &fmt);

    // --- Phase 2: per-crate test runs with concurrency limit ---
    eprintln!("\nRunning per-crate tests (concurrency=2)...");

    let crates = ["ironflow-core", "ironflow-runtime"];
    let test_futures: Vec<_> = crates
        .iter()
        .map(|name| {
            Shell::new(&format!("cargo test -p {name} --no-fail-fast 2>&1 || true"))
                .timeout(Duration::from_secs(180))
                .run()
        })
        .collect();

    let test_results = try_join_all_limited(test_futures, 2).await?;

    for (name, result) in crates.iter().zip(&test_results) {
        tracker.record_shell(&format!("test-{name}"), result);
        let passed = !result.stdout().contains("FAILED");
        print_check(&format!("test ({name})"), passed, result);
    }

    // --- Summary ---
    eprintln!("\n--- Health check complete ---");
    eprintln!("Steps: {}", tracker.step_count());
    eprintln!("Wall time: {}ms", tracker.total_duration_ms());
    tracker.summary();

    Ok(())
}

fn print_check(name: &str, passed: bool, output: &ShellOutput) {
    let icon = if passed { "PASS" } else { "FAIL" };
    eprintln!("[{icon}] {name} ({}ms)", output.duration_ms());
    if !passed {
        for line in output.stdout().lines().take(10) {
            eprintln!("       {line}");
        }
    }
}
