use ironflow_ops_common::log::{condense_log, drop_noise, is_failure_line};

const MARKER: &str = "<<cut>>";

fn failure_log() -> String {
    let mut lines: Vec<String> = (0..200).map(|i| format!("setup step {i}")).collect();
    lines.push("error[E0599]: no method named `foo`".to_string());
    lines.push(" --> src/main.rs:3:5".to_string());
    lines.push("  |".to_string());
    lines.push("  = help: try bar".to_string());
    lines.push(String::new());
    lines.extend((0..200).map(|i| format!("filler {i}")));
    lines.push("tail-end line".to_string());
    lines.join("\n")
}

#[test]
fn drop_noise_removes_cargo_pnpm_and_script_echo_lines() {
    let log = [
        "   Compiling serde v1.0.0",
        "Downloaded serde v1.0.0",
        "    Finished `dev` profile",
        "test foo::bar ... ok",
        "Progress: resolved 10, reused 5",
        "Packages: +12",
        "++++++-----",
        "> pkg@1.0.0 build /app",
        "> @scope/pkg@2.1.0 build /app",
        "> tsc -b",
        "kept line",
        "",
        "",
        "",
        "end",
    ]
    .join("\n");
    assert_eq!(drop_noise(&log), "kept line\n\nend");
}

#[test]
fn drop_noise_keeps_unrelated_quoted_lines() {
    let log = "> some quoted message\n> node server.js\n> pkg@x build\nWARN deprecated foo";
    assert_eq!(drop_noise(log), log);
}

#[test]
fn drop_noise_keeps_failure_lines_matching_noise_prefix() {
    let log = "Compiling app\nCompiling failed: error: boom\ntest a::b ... FAILED\nFinished with ERROR: x";
    assert_eq!(
        drop_noise(log),
        "Compiling failed: error: boom\ntest a::b ... FAILED\nFinished with ERROR: x"
    );
}

#[test]
fn is_failure_line_table() {
    let failures = [
        "error: could not compile",
        "error[E0308]: mismatched types",
        "src/a.ts(1,1): error TS2322: bad",
        "ERROR: Job failed",
        " ERR_PNPM_OUTDATED_LOCKFILE x",
        "npm ERR! code 1",
        "test a::b ... FAILED",
        "thread 'x' panicked at src/lib.rs:1:1",
        "Job failed: exit code 1",
        "test result: FAILED. 1 passed",
        "---- tests::it_works stdout ----",
    ];
    for line in failures {
        assert!(is_failure_line(line), "{line}");
    }
    let others = [
        "",
        "   Compiling app v0.1.0",
        "test a::b ... ok",
        "---- not a header",
        "WARN deprecated foo",
    ];
    for line in others {
        assert!(!is_failure_line(line), "{line}");
    }
}

#[test]
fn condense_log_keeps_failure_tail_and_lead_in() {
    let log = failure_log();
    let out = condense_log(&log, 1200, MARKER);
    assert!(out.chars().count() <= 1200);
    assert!(out.contains("error[E0599]: no method named `foo`"));
    assert!(out.contains("  = help: try bar"));
    assert!(out.contains("setup step 199"));
    assert!(out.contains("setup step 170"));
    assert!(!out.contains("setup step 169"));
    assert!(out.contains("tail-end line"));
    assert!(!out.contains("filler 3"));
    assert_eq!(out.matches(MARKER).count(), 2);
}

#[test]
fn condense_log_cargo_test_block_stops_at_failures_summary() {
    let mut lines: Vec<String> = (0..200).map(|i| format!("setup step {i}")).collect();
    lines.push("---- t::a stdout ----".to_string());
    lines.push("assertion left: 1".to_string());
    lines.push("failures:".to_string());
    lines.push("    t::a_summary_entry".to_string());
    lines.extend((0..200).map(|i| format!("filler {i}")));
    lines.push("tail-end line".to_string());
    let out = condense_log(&lines.join("\n"), 1200, MARKER);
    assert!(out.chars().count() <= 1200);
    assert!(out.contains("---- t::a stdout ----"));
    assert!(out.contains("assertion left: 1"));
    assert!(!out.contains("failures:"));
    assert!(!out.contains("t::a_summary_entry"));
    assert!(out.contains("tail-end line"));
}

#[test]
fn condense_log_failure_block_is_capped_at_forty_lines() {
    let mut lines: Vec<String> = (0..200).map(|i| format!("setup step {i}")).collect();
    lines.push("error: boom".to_string());
    lines.extend((0..100).map(|i| format!("trace frame {i}")));
    lines.extend((0..200).map(|i| format!("filler {i}")));
    lines.push("tail-end line".to_string());
    let out = condense_log(&lines.join("\n"), 2000, MARKER);
    assert!(out.chars().count() <= 2000);
    // The block is the error line plus 39 lines: frames 0 to 38.
    assert!(out.lines().any(|l| l == "trace frame 38"));
    assert!(!out.lines().any(|l| l == "trace frame 39"));
    assert!(out.contains("tail-end line"));
}

#[test]
fn condense_log_never_exceeds_budget() {
    let mut inputs = vec![
        failure_log(),
        "x".repeat(5000),
        "ligne é€\n".repeat(300),
        "error: é\n".repeat(300),
    ];
    inputs.push(format!("{}\nerror: boom", "y".repeat(3000)));
    for input in &inputs {
        for budget in [1, 2, 3, 5, 7, 8, 10, 50, 100, 500, 2000] {
            for marker in [MARKER, "", "[... trace tronquée, début omis ...]"] {
                let out = condense_log(input, budget, marker);
                assert!(
                    out.chars().count() <= budget,
                    "budget {budget}: got {}",
                    out.chars().count()
                );
            }
        }
    }
}

#[test]
fn condense_log_single_giant_line_keeps_marker_and_tail() {
    let line = format!("{}END", "x".repeat(5000));
    let out = condense_log(&line, 500, "<<cut>>");
    assert_eq!(out.chars().count(), 500);
    assert!(out.starts_with("<<cut>>\n"));
    assert!(out.ends_with("xxxEND"));

    let failing = format!("error: {}END", "x".repeat(5000));
    let out = condense_log(&failing, 500, "<<cut>>");
    assert_eq!(out.chars().count(), 500);
    assert!(out.ends_with("xxxEND"));
}

#[test]
fn condense_log_under_budget_is_unchanged_without_marker() {
    let text = "line 1\nerror: boom\nline 3";
    assert_eq!(condense_log(text, text.chars().count(), MARKER), text);
    let noisy = "   Compiling a v1\nerror: boom";
    assert_eq!(condense_log(noisy, 12, MARKER), "error: boom");
}

#[test]
fn condense_log_without_failure_returns_marker_and_tail() {
    let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let out = condense_log(&text, 100, "[cut]");
    assert!(out.starts_with("[cut]\n"));
    assert!(out.chars().count() <= 100);
    assert!(out.contains("line 98"));
    assert!(!out.contains("line 1\n"));
}

#[test]
fn condense_log_preserves_unicode_marker() {
    let marker = "[... trace tronquée, début omis ...]";
    let text: String = (0..100).map(|i| format!("ligne é€ {i}\n")).collect();
    let out = condense_log(&text, 200, marker);
    assert!(out.starts_with(marker));
    assert!(out.chars().count() <= 200);
    assert!(out.contains("ligne é€ 99"));
}

#[test]
fn condense_log_empty_input() {
    assert_eq!(condense_log("", 10, MARKER), "");
}

#[test]
#[should_panic(expected = "budget")]
fn condense_log_zero_budget_panics() {
    condense_log("text", 0, MARKER);
}
