//! Integration tests for artifact production and consumption.
//!
//! Black-box: real shell processes writing real files into a real temporary
//! directory, stored through the real `LocalBlobStore`.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

use ironflow_artifacts::blob_store::BlobStore;
use ironflow_artifacts::local::LocalBlobStore;
use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::artifact::{ArtifactSink, DirectArtifactSink};
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{NewRun, StepStatus, TriggerKind};
use ironflow_store::store::Store;

/// A run context wired to a filesystem-backed artifact store.
struct Fixture {
    ctx: WorkflowContext,
    store: Arc<dyn Store>,
    run_id: Uuid,
    /// Where steps run and write their files.
    work_dir: TempDir,
    /// Where artifact blobs are persisted.
    _blob_dir: TempDir,
}

impl Fixture {
    async fn new() -> Self {
        Self::build(true).await
    }

    /// Same fixture, without an artifact backend attached.
    async fn without_artifact_storage() -> Self {
        Self::build(false).await
    }

    async fn build(with_artifacts: bool) -> Self {
        let work_dir = TempDir::new().expect("work dir");
        let blob_dir = TempDir::new().expect("blob dir");

        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let provider: Arc<dyn AgentProvider> = Arc::new(RecordReplayProvider::replay(
            ClaudeCodeProvider::new(),
            "/tmp/ironflow-fixtures",
        ));

        let run = store
            .create_run(NewRun {
                workflow_name: "artifacts".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                created_by: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .expect("create run")
            .into_run();

        let mut ctx = WorkflowContext::new(run.id, "test".to_string(), store.clone(), provider);
        if with_artifacts {
            let blob: Arc<dyn BlobStore> = Arc::new(LocalBlobStore::new(blob_dir.path()));
            let sink: Arc<dyn ArtifactSink> =
                Arc::new(DirectArtifactSink::new(blob, store.clone()));
            ctx.set_artifact_sink(sink);
        }

        Self {
            ctx,
            store,
            run_id: run.id,
            work_dir,
            _blob_dir: blob_dir,
        }
    }

    /// A shell config rooted in the fixture's working directory.
    fn shell(&self, command: &str) -> ShellConfig {
        ShellConfig::new(command).dir(self.work_dir.path().to_str().expect("utf-8 path"))
    }
}

#[tokio::test]
async fn a_declared_output_is_stored_with_its_size_type_and_hash() {
    let mut fixture = Fixture::new().await;

    fixture
        .ctx
        .shell(
            "build",
            fixture
                .shell("printf 'hello' > report.html")
                .output("report.html"),
        )
        .await
        .expect("step");

    let artifacts = fixture
        .store
        .list_artifacts_for_run(fixture.run_id)
        .await
        .expect("list");

    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].name, "report.html");
    assert_eq!(artifacts[0].content_type, "text/html");
    assert_eq!(artifacts[0].size_bytes, 5);
    assert_eq!(
        artifacts[0].sha256,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
}

#[tokio::test]
async fn a_later_step_reads_an_artifact_produced_earlier() {
    let mut fixture = Fixture::new().await;

    fixture
        .ctx
        .shell(
            "build",
            fixture
                .shell("printf 'payload' > report.txt")
                .output("report.txt"),
        )
        .await
        .expect("producer");

    // The consumer runs in a different directory: the only way it can see the
    // file is through the declared input.
    let consumer_dir = TempDir::new().expect("consumer dir");
    let consumer = ShellConfig::new("cat report.txt")
        .dir(consumer_dir.path().to_str().expect("utf-8 path"))
        .input("build", "report.txt");

    let output = fixture
        .ctx
        .shell("publish", consumer)
        .await
        .expect("consumer");

    assert_eq!(output.output["stdout"], "payload");
}

#[tokio::test]
async fn an_input_can_be_written_to_another_path() {
    let mut fixture = Fixture::new().await;

    fixture
        .ctx
        .shell(
            "build",
            fixture
                .shell("printf 'x' > report.txt")
                .output("report.txt"),
        )
        .await
        .expect("producer");

    let consumer_dir = TempDir::new().expect("consumer dir");
    let consumer = ShellConfig::new("cat nested/renamed.txt")
        .dir(consumer_dir.path().to_str().expect("utf-8 path"))
        .input_at("build", "report.txt", "nested/renamed.txt");

    let output = fixture
        .ctx
        .shell("publish", consumer)
        .await
        .expect("consumer");

    assert_eq!(output.output["stdout"], "x");
}

#[tokio::test]
async fn a_glob_stores_every_match() {
    let mut fixture = Fixture::new().await;

    fixture
        .ctx
        .shell(
            "build",
            fixture.shell("touch a.log b.log notes.txt").output("*.log"),
        )
        .await
        .expect("step");

    let names: Vec<String> = fixture
        .store
        .list_artifacts_for_run(fixture.run_id)
        .await
        .expect("list")
        .into_iter()
        .map(|artifact| artifact.name)
        .collect();

    assert_eq!(names.len(), 2);
    assert!(names.contains(&"a.log".to_string()));
    assert!(names.contains(&"b.log".to_string()));
}

#[tokio::test]
async fn an_output_that_matches_nothing_fails_a_successful_step() {
    let mut fixture = Fixture::new().await;

    let err = fixture
        .ctx
        .shell("build", fixture.shell("true").output("target/report.html"))
        .await
        .expect_err("declared output never appeared");

    assert!(matches!(
        err,
        EngineError::MissingArtifact { ref step, ref pattern }
            if step == "build" && pattern == "target/report.html"
    ));

    let steps = fixture
        .store
        .list_steps(fixture.run_id)
        .await
        .expect("steps");
    assert_eq!(steps[0].status.state, StepStatus::Failed);
}

#[tokio::test]
async fn a_failed_step_still_yields_the_files_it_did_produce() {
    let mut fixture = Fixture::new().await;

    let err = fixture
        .ctx
        .shell(
            "build",
            fixture
                .shell("printf 'partial' > build.log; exit 1")
                .output("build.log"),
        )
        .await
        .expect_err("command failed");

    // The command failure is what surfaces, not an artifact error.
    assert!(matches!(err, EngineError::Operation(_)));

    let artifacts = fixture
        .store
        .list_artifacts_for_run(fixture.run_id)
        .await
        .expect("list");
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].name, "build.log");
    assert_eq!(artifacts[0].size_bytes, 7);
}

#[tokio::test]
async fn a_failed_step_tolerates_an_output_that_matched_nothing() {
    let mut fixture = Fixture::new().await;

    let err = fixture
        .ctx
        .shell("build", fixture.shell("exit 1").output("report.html"))
        .await
        .expect_err("command failed");

    assert!(matches!(err, EngineError::Operation(_)));
    assert!(
        fixture
            .store
            .list_artifacts_for_run(fixture.run_id)
            .await
            .expect("list")
            .is_empty()
    );
}

#[tokio::test]
async fn an_unresolvable_input_fails_the_step_before_the_command_runs() {
    let mut fixture = Fixture::new().await;

    let err = fixture
        .ctx
        .shell(
            "publish",
            fixture
                .shell("printf 'ran' > marker.txt")
                .input("build", "report.txt"),
        )
        .await
        .expect_err("input does not exist");

    assert!(matches!(
        err,
        EngineError::ArtifactNotFound { ref step, ref name }
            if step == "build" && name == "report.txt"
    ));
    assert!(
        !fixture.work_dir.path().join("marker.txt").exists(),
        "the command must not have run"
    );
}

#[tokio::test]
async fn an_input_produced_by_a_later_step_is_not_visible() {
    let mut fixture = Fixture::new().await;

    // Position 0 declares an input produced at position 1: nothing can satisfy
    // it, because resolution only looks backwards.
    let err = fixture
        .ctx
        .shell(
            "publish",
            fixture.shell("true").input("build", "report.txt"),
        )
        .await
        .expect_err("nothing produced yet");
    assert!(matches!(err, EngineError::ArtifactNotFound { .. }));

    fixture
        .ctx
        .shell(
            "build",
            fixture
                .shell("printf 'x' > report.txt")
                .output("report.txt"),
        )
        .await
        .expect("producer");
}

#[tokio::test]
async fn the_same_file_name_from_two_steps_is_kept_separately() {
    let mut fixture = Fixture::new().await;

    fixture
        .ctx
        .shell(
            "build",
            fixture.shell("printf 'one' > out.txt").output("out.txt"),
        )
        .await
        .expect("first");
    fixture
        .ctx
        .shell(
            "rebuild",
            fixture.shell("printf 'two' > out.txt").output("out.txt"),
        )
        .await
        .expect("second");

    let artifacts = fixture
        .store
        .list_artifacts_for_run(fixture.run_id)
        .await
        .expect("list");

    assert_eq!(artifacts.len(), 2);
    assert_ne!(artifacts[0].step_id, artifacts[1].step_id);
}

#[tokio::test]
async fn an_explicit_content_type_wins_over_the_guess() {
    let mut fixture = Fixture::new().await;

    fixture
        .ctx
        .shell(
            "build",
            fixture
                .shell("printf '{}' > data.txt")
                .output_typed("data.txt", "application/json"),
        )
        .await
        .expect("step");

    let artifacts = fixture
        .store
        .list_artifacts_for_run(fixture.run_id)
        .await
        .expect("list");

    assert_eq!(artifacts[0].content_type, "application/json");
}

#[tokio::test]
async fn a_file_name_outside_the_whitelist_is_refused() {
    let mut fixture = Fixture::new().await;

    let err = fixture
        .ctx
        .shell(
            "build",
            fixture
                .shell("printf 'x' > 'rapport été.html'")
                .output("rapport été.html"),
        )
        .await
        .expect_err("name is not in the whitelist");

    assert!(matches!(err, EngineError::Artifact(_)));
}

#[tokio::test]
async fn a_step_without_artifacts_runs_without_a_backend() {
    let mut fixture = Fixture::without_artifact_storage().await;

    let output = fixture
        .ctx
        .shell("greet", fixture.shell("echo hello"))
        .await
        .expect("step");

    assert_eq!(output.output["stdout"], "hello");
}

#[tokio::test]
async fn declaring_an_output_without_a_backend_fails_explicitly() {
    let mut fixture = Fixture::without_artifact_storage().await;

    let err = fixture
        .ctx
        .shell(
            "build",
            fixture
                .shell("printf 'x' > report.txt")
                .output("report.txt"),
        )
        .await
        .expect_err("no artifact storage");

    assert!(matches!(err, EngineError::ArtifactsUnavailable(_)));
}

#[tokio::test]
async fn put_and_get_artifact_roundtrip_for_custom_operations() {
    let mut fixture = Fixture::new().await;

    // A shell step gives us a persisted step to hang the artifact on.
    fixture
        .ctx
        .shell("generate", fixture.shell("true"))
        .await
        .expect("step");

    let step_id = fixture
        .store
        .list_steps(fixture.run_id)
        .await
        .expect("steps")[0]
        .id;

    let artifact = fixture
        .ctx
        .put_artifact(step_id, "summary.json", None, br#"{"ok":true}"#.to_vec())
        .await
        .expect("put");

    assert_eq!(artifact.content_type, "application/json");

    let bytes = fixture
        .ctx
        .get_artifact("generate", "summary.json")
        .await
        .expect("get");

    assert_eq!(bytes, br#"{"ok":true}"#);
}

#[tokio::test]
async fn get_artifact_on_an_unknown_name_reports_not_found() {
    let mut fixture = Fixture::new().await;

    fixture
        .ctx
        .shell("generate", fixture.shell("true"))
        .await
        .expect("step");

    let err = fixture
        .ctx
        .get_artifact("generate", "missing.json")
        .await
        .expect_err("unknown artifact");

    assert!(matches!(err, EngineError::ArtifactNotFound { .. }));
}

#[tokio::test]
async fn put_artifact_refuses_a_duplicate_name_on_the_same_step() {
    let mut fixture = Fixture::new().await;

    fixture
        .ctx
        .shell("generate", fixture.shell("true"))
        .await
        .expect("step");
    let step_id = fixture
        .store
        .list_steps(fixture.run_id)
        .await
        .expect("steps")[0]
        .id;

    fixture
        .ctx
        .put_artifact(step_id, "a.json", None, b"{}".to_vec())
        .await
        .expect("first");

    let err = fixture
        .ctx
        .put_artifact(step_id, "a.json", None, b"{}".to_vec())
        .await
        .expect_err("duplicate");

    assert!(matches!(err, EngineError::Store(_)));
}

#[tokio::test]
async fn put_artifact_without_a_backend_fails_explicitly() {
    let mut fixture = Fixture::without_artifact_storage().await;

    fixture
        .ctx
        .shell("generate", fixture.shell("true"))
        .await
        .expect("step");
    let step_id = fixture
        .store
        .list_steps(fixture.run_id)
        .await
        .expect("steps")[0]
        .id;

    let err = fixture
        .ctx
        .put_artifact(step_id, "a.json", None, b"{}".to_vec())
        .await
        .expect_err("no artifact storage");

    assert!(matches!(err, EngineError::ArtifactsUnavailable(_)));
}

#[tokio::test]
async fn an_empty_file_is_a_valid_artifact() {
    let mut fixture = Fixture::new().await;

    fixture
        .ctx
        .shell(
            "build",
            fixture.shell("touch empty.txt").output("empty.txt"),
        )
        .await
        .expect("step");

    let artifacts = fixture
        .store
        .list_artifacts_for_run(fixture.run_id)
        .await
        .expect("list");

    assert_eq!(artifacts[0].size_bytes, 0);
    assert_eq!(
        artifacts[0].sha256,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[tokio::test]
async fn a_directory_matching_a_pattern_is_ignored() {
    let mut fixture = Fixture::new().await;

    // `out.d` is a directory and `out.f` a file; only the file is an artifact.
    fixture
        .ctx
        .shell(
            "build",
            fixture
                .shell("mkdir -p out.d && printf 'x' > out.f")
                .output("out.*"),
        )
        .await
        .expect("step");

    let artifacts = fixture
        .store
        .list_artifacts_for_run(fixture.run_id)
        .await
        .expect("list");

    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].name, "out.f");
}
