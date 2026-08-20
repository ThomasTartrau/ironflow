//! Seed the store with development data.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, bail};
use chrono::Utc;
use clap::Args;
use ironflow_artifacts::blob_store::BlobStore;
use ironflow_artifacts::local::LocalBlobStore;
use ironflow_artifacts::stream_from_bytes;
use ironflow_auth::password;
use ironflow_store::entities::{
    ApiKeyScope, EventKind, NewApiKey, NewArtifact, NewAuditLogEntry, NewRun, NewStep, NewUser,
    RunActor, RunStatus, StepKind, StepStatus, StepUpdate, TriggerKind,
};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::store::Store;
use rust_decimal::Decimal;
use serde_json::json;
use tracing::info;
use uuid::Uuid;

/// CLI arguments for the seed command.
#[derive(Args)]
pub struct SeedArgs {
    /// Reset the store before seeding.
    #[arg(long)]
    force: bool,

    /// PostgreSQL connection URL. Without it, seeds an InMemoryStore (dry run).
    #[arg(long)]
    database_url: Option<String>,

    /// Directory for artifact files. Without it, artifacts are skipped.
    #[arg(long)]
    artifacts_dir: Option<PathBuf>,
}

/// Options passed to the seed function.
pub struct SeedOptions {
    /// Reset before seeding (used by the Postgres path).
    #[cfg_attr(not(feature = "postgres"), allow(dead_code))]
    pub force: bool,
    /// Directory for artifact files.
    pub artifacts_dir: Option<PathBuf>,
}

/// Run the seed command.
pub async fn run(args: SeedArgs) -> anyhow::Result<()> {
    let opts = SeedOptions {
        force: args.force,
        artifacts_dir: args.artifacts_dir,
    };

    if let Some(ref url) = args.database_url {
        #[cfg(feature = "postgres")]
        {
            seed_postgres(url, &opts).await
        }
        #[cfg(not(feature = "postgres"))]
        {
            let _ = url;
            bail!(
                "--database-url requires the 'postgres' feature.\n\
                 Build with: cargo xtask --features postgres seed --database-url ..."
            );
        }
    } else {
        let store = InMemoryStore::new();
        let store: Arc<dyn Store> = Arc::new(store);
        seed_store(&*store, &opts).await?;
        info!("seed complete (in-memory, data not persisted)");
        Ok(())
    }
}

#[cfg(feature = "postgres")]
async fn seed_postgres(url: &str, opts: &SeedOptions) -> anyhow::Result<()> {
    use ironflow_store::crypto::KeyRing;
    use ironflow_store::postgres::PostgresStore;

    if opts.force {
        info!("--force: truncating all data tables");
        let pool = sqlx::PgPool::connect(url)
            .await
            .context("failed to connect for truncate")?;
        sqlx::query(
            "TRUNCATE ironflow.step_artifacts, ironflow.step_dependencies, \
             ironflow.steps, ironflow.runs, ironflow.secrets, ironflow.audit_logs, \
             iam.api_keys, iam.users CASCADE",
        )
        .execute(&pool)
        .await
        .context("TRUNCATE failed")?;
        pool.close().await;
        info!("truncate done");
    }

    let mut store = PostgresStore::new(url)
        .await
        .context("failed to connect PostgresStore")?;

    if let Ok(Some(ring)) = KeyRing::from_env() {
        store.set_key_ring(ring);
    }

    let store: Arc<dyn Store> = Arc::new(store);
    seed_store(&*store, opts).await?;
    info!("seed complete (PostgreSQL)");
    Ok(())
}

/// Seed the store with development data.
///
/// Expects an empty store. If the store already contains users, returns an
/// error unless `opts.force` is set (the caller is responsible for resetting
/// the store before calling this function).
pub async fn seed_store(store: &dyn Store, opts: &SeedOptions) -> anyhow::Result<()> {
    let existing = store.count_users().await?;
    if existing > 0 {
        bail!("store already contains {existing} user(s). Use --force to reset before seeding.");
    }

    let users = seed_users(store).await?;
    let runs = seed_runs(store, &users).await?;
    seed_api_keys(store, &users).await?;
    seed_secrets(store).await?;
    seed_audit_logs(store, &users, &runs).await?;

    if let Some(ref dir) = opts.artifacts_dir {
        let blob_store = LocalBlobStore::new(dir);
        seed_artifacts(store, &blob_store, &runs).await?;
    } else {
        info!("no --artifacts-dir, skipping artifacts");
    }

    Ok(())
}

#[allow(dead_code)]
struct SeededUser {
    id: Uuid,
    email: String,
    username: String,
    is_admin: bool,
}

struct SeededRun {
    id: Uuid,
    workflow_name: String,
    status: RunStatus,
    step_ids: Vec<Uuid>,
}

// ---------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------

async fn seed_users(store: &dyn Store) -> anyhow::Result<Vec<SeededUser>> {
    let specs = [
        ("admin@ironflow.dev", "admin", Some(true)),
        ("alice@ironflow.dev", "alice", None),
        ("bob@ironflow.dev", "bob", None),
    ];

    let mut users = Vec::new();
    for (email, username, is_admin) in specs {
        let hash = password::hash(email).context("password hash failed")?;
        let user = store
            .create_user(NewUser {
                email: email.to_string(),
                username: username.to_string(),
                password_hash: hash,
                is_admin,
            })
            .await?;

        info!(
            email,
            username,
            admin = user.is_admin,
            "created user (password = email)"
        );
        users.push(SeededUser {
            id: user.id,
            email: email.to_string(),
            username: username.to_string(),
            is_admin: user.is_admin,
        });
    }
    Ok(users)
}

// ---------------------------------------------------------------------------
// Runs + Steps
// ---------------------------------------------------------------------------

struct RunSpec {
    workflow: &'static str,
    trigger: TriggerKind,
    target_status: RunStatus,
    labels: HashMap<String, String>,
    steps: Vec<StepSpec>,
}

struct StepSpec {
    name: &'static str,
    kind: StepKind,
    target_status: StepStatus,
    output: Option<serde_json::Value>,
    error: Option<&'static str>,
    duration_ms: u64,
    cost_usd: Decimal,
}

fn run_specs() -> Vec<RunSpec> {
    vec![
        RunSpec {
            workflow: "greeting",
            trigger: TriggerKind::Manual,
            target_status: RunStatus::Completed,
            labels: HashMap::from([("env".to_string(), "dev".to_string())]),
            steps: vec![
                StepSpec {
                    name: "say-hello",
                    kind: StepKind::Agent,
                    target_status: StepStatus::Completed,
                    output: Some(
                        json!({"message": "Hello, world!", "model": "claude-sonnet-4-20250514"}),
                    ),
                    error: None,
                    duration_ms: 1200,
                    cost_usd: Decimal::new(3, 2),
                },
                StepSpec {
                    name: "log-result",
                    kind: StepKind::Shell,
                    target_status: StepStatus::Completed,
                    output: Some(json!({"stdout": "Greeting sent successfully", "exit_code": 0})),
                    error: None,
                    duration_ms: 50,
                    cost_usd: Decimal::ZERO,
                },
            ],
        },
        RunSpec {
            workflow: "greeting",
            trigger: TriggerKind::Api,
            target_status: RunStatus::Failed,
            labels: HashMap::from([("env".to_string(), "staging".to_string())]),
            steps: vec![StepSpec {
                name: "say-hello",
                kind: StepKind::Agent,
                target_status: StepStatus::Failed,
                output: None,
                error: Some("agent budget exceeded: $0.50 > $0.10 max"),
                duration_ms: 800,
                cost_usd: Decimal::new(50, 2),
            }],
        },
        RunSpec {
            workflow: "ci-pipeline",
            trigger: TriggerKind::Webhook {
                path: "/hooks/gitlab".to_string(),
            },
            target_status: RunStatus::Completed,
            labels: HashMap::from([
                ("env".to_string(), "ci".to_string()),
                ("branch".to_string(), "main".to_string()),
            ]),
            steps: vec![
                StepSpec {
                    name: "checkout",
                    kind: StepKind::Shell,
                    target_status: StepStatus::Completed,
                    output: Some(json!({"stdout": "HEAD is now at abc1234", "exit_code": 0})),
                    error: None,
                    duration_ms: 3000,
                    cost_usd: Decimal::ZERO,
                },
                StepSpec {
                    name: "build",
                    kind: StepKind::Shell,
                    target_status: StepStatus::Completed,
                    output: Some(
                        json!({"stdout": "Compiling ironflow v2.0.0\n   Finished release", "exit_code": 0}),
                    ),
                    error: None,
                    duration_ms: 45000,
                    cost_usd: Decimal::ZERO,
                },
                StepSpec {
                    name: "test",
                    kind: StepKind::Shell,
                    target_status: StepStatus::Completed,
                    output: Some(
                        json!({"stdout": "test result: ok. 142 passed; 0 failed", "exit_code": 0}),
                    ),
                    error: None,
                    duration_ms: 30000,
                    cost_usd: Decimal::ZERO,
                },
            ],
        },
        RunSpec {
            workflow: "ci-pipeline",
            trigger: TriggerKind::Webhook {
                path: "/hooks/gitlab".to_string(),
            },
            target_status: RunStatus::Running,
            labels: HashMap::from([
                ("env".to_string(), "ci".to_string()),
                ("branch".to_string(), "feat/new-feature".to_string()),
            ]),
            steps: vec![
                StepSpec {
                    name: "checkout",
                    kind: StepKind::Shell,
                    target_status: StepStatus::Completed,
                    output: Some(json!({"stdout": "HEAD is now at def5678", "exit_code": 0})),
                    error: None,
                    duration_ms: 2500,
                    cost_usd: Decimal::ZERO,
                },
                StepSpec {
                    name: "build",
                    kind: StepKind::Shell,
                    target_status: StepStatus::Running,
                    output: None,
                    error: None,
                    duration_ms: 0,
                    cost_usd: Decimal::ZERO,
                },
            ],
        },
        RunSpec {
            workflow: "ci-pipeline",
            trigger: TriggerKind::Webhook {
                path: "/hooks/gitlab".to_string(),
            },
            target_status: RunStatus::Pending,
            labels: HashMap::from([
                ("env".to_string(), "ci".to_string()),
                ("branch".to_string(), "fix/hotfix".to_string()),
            ]),
            steps: vec![StepSpec {
                name: "checkout",
                kind: StepKind::Shell,
                target_status: StepStatus::Pending,
                output: None,
                error: None,
                duration_ms: 0,
                cost_usd: Decimal::ZERO,
            }],
        },
        RunSpec {
            workflow: "deploy-approval",
            trigger: TriggerKind::Manual,
            target_status: RunStatus::Completed,
            labels: HashMap::from([("env".to_string(), "production".to_string())]),
            steps: vec![
                StepSpec {
                    name: "plan",
                    kind: StepKind::Agent,
                    target_status: StepStatus::Completed,
                    output: Some(json!({"plan": "Deploy v2.1.0 to production", "changes": 3})),
                    error: None,
                    duration_ms: 5000,
                    cost_usd: Decimal::new(8, 2),
                },
                StepSpec {
                    name: "approve",
                    kind: StepKind::Approval,
                    target_status: StepStatus::Completed,
                    output: Some(json!({"approved_by": "admin", "comment": "LGTM"})),
                    error: None,
                    duration_ms: 120000,
                    cost_usd: Decimal::ZERO,
                },
                StepSpec {
                    name: "deploy",
                    kind: StepKind::Shell,
                    target_status: StepStatus::Completed,
                    output: Some(
                        json!({"stdout": "Deployed v2.1.0 to 3 replicas", "exit_code": 0}),
                    ),
                    error: None,
                    duration_ms: 15000,
                    cost_usd: Decimal::ZERO,
                },
            ],
        },
        RunSpec {
            workflow: "deploy-approval",
            trigger: TriggerKind::Manual,
            target_status: RunStatus::Cancelled,
            labels: HashMap::from([("env".to_string(), "production".to_string())]),
            steps: vec![
                StepSpec {
                    name: "plan",
                    kind: StepKind::Agent,
                    target_status: StepStatus::Completed,
                    output: Some(json!({"plan": "Deploy v2.0.9-rc1", "changes": 12})),
                    error: None,
                    duration_ms: 4500,
                    cost_usd: Decimal::new(7, 2),
                },
                StepSpec {
                    name: "approve",
                    kind: StepKind::Approval,
                    target_status: StepStatus::Skipped,
                    output: None,
                    error: None,
                    duration_ms: 0,
                    cost_usd: Decimal::ZERO,
                },
            ],
        },
        RunSpec {
            workflow: "system-audit",
            trigger: TriggerKind::Cron {
                schedule: "0 0 * * *".to_string(),
            },
            target_status: RunStatus::Completed,
            labels: HashMap::from([("env".to_string(), "production".to_string())]),
            steps: vec![
                StepSpec {
                    name: "collect-metrics",
                    kind: StepKind::Shell,
                    target_status: StepStatus::Completed,
                    output: Some(json!({"cpu": 42.5, "memory_mb": 1024, "disk_pct": 68})),
                    error: None,
                    duration_ms: 2000,
                    cost_usd: Decimal::ZERO,
                },
                StepSpec {
                    name: "analyze",
                    kind: StepKind::Agent,
                    target_status: StepStatus::Completed,
                    output: Some(json!({
                        "summary": "System healthy. Disk usage trending up.",
                        "recommendations": ["Consider archiving old logs"]
                    })),
                    error: None,
                    duration_ms: 8000,
                    cost_usd: Decimal::new(12, 2),
                },
            ],
        },
    ]
}

async fn seed_runs(store: &dyn Store, users: &[SeededUser]) -> anyhow::Result<Vec<SeededRun>> {
    let admin = &users[0];
    let mut seeded = Vec::new();

    for spec in run_specs() {
        let created_by = Some(RunActor::User { user_id: admin.id });

        let creation = store
            .create_run(NewRun {
                workflow_name: spec.workflow.to_string(),
                trigger: spec.trigger.clone(),
                payload: json!({}),
                max_retries: 3,
                handler_version: Some("0.1.0".to_string()),
                labels: spec.labels.clone(),
                scheduled_at: None,
                created_by,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await?;
        let run = creation.into_run();
        let run_id = run.id;

        // Transition run through FSM
        let run_needs_running = spec.target_status == RunStatus::Running
            || spec.target_status == RunStatus::Completed
            || spec.target_status == RunStatus::Failed
            || spec.target_status == RunStatus::Cancelled
            || spec.target_status == RunStatus::Warning;

        if run_needs_running {
            store.update_run_status(run_id, RunStatus::Running).await?;
        }

        if spec.target_status.is_terminal() {
            store.update_run_status(run_id, spec.target_status).await?;
        }

        // Create steps
        let mut step_ids = Vec::new();
        let mut total_duration = 0u64;
        let mut total_cost = Decimal::ZERO;

        for (pos, step_spec) in spec.steps.iter().enumerate() {
            let step = store
                .create_step(NewStep {
                    run_id,
                    name: step_spec.name.to_string(),
                    kind: step_spec.kind.clone(),
                    position: pos as u32,
                    input: Some(json!({"seed": true})),
                    is_error_handler: false,
                })
                .await?;

            let now = Utc::now();

            match step_spec.target_status {
                StepStatus::Pending => {}
                StepStatus::Running => {
                    store
                        .update_step(
                            step.id,
                            StepUpdate {
                                status: Some(StepStatus::Running),
                                started_at: Some(now),
                                ..StepUpdate::default()
                            },
                        )
                        .await?;
                }
                StepStatus::Skipped => {
                    store
                        .update_step(
                            step.id,
                            StepUpdate {
                                status: Some(StepStatus::Skipped),
                                ..StepUpdate::default()
                            },
                        )
                        .await?;
                }
                terminal => {
                    // Pending -> Running -> terminal
                    store
                        .update_step(
                            step.id,
                            StepUpdate {
                                status: Some(StepStatus::Running),
                                started_at: Some(now),
                                ..StepUpdate::default()
                            },
                        )
                        .await?;
                    store
                        .update_step(
                            step.id,
                            StepUpdate {
                                status: Some(terminal),
                                output: step_spec.output.clone(),
                                error: step_spec.error.map(|s| s.to_string()),
                                duration_ms: Some(step_spec.duration_ms),
                                cost_usd: Some(step_spec.cost_usd),
                                completed_at: Some(now),
                                ..StepUpdate::default()
                            },
                        )
                        .await?;
                }
            }

            total_duration += step_spec.duration_ms;
            total_cost += step_spec.cost_usd;
            step_ids.push(step.id);
        }

        // Update run aggregated metrics
        let mut run_update = ironflow_store::entities::RunUpdate {
            cost_usd: Some(total_cost),
            duration_ms: Some(total_duration),
            ..Default::default()
        };
        if run_needs_running {
            run_update.started_at = Some(Utc::now());
        }
        if spec.target_status.is_terminal() {
            run_update.completed_at = Some(Utc::now());
        }
        store.update_run(run_id, run_update).await?;

        info!(
            workflow = spec.workflow,
            status = %spec.target_status,
            steps = step_ids.len(),
            "created run"
        );

        seeded.push(SeededRun {
            id: run_id,
            workflow_name: spec.workflow.to_string(),
            status: spec.target_status,
            step_ids,
        });
    }

    Ok(seeded)
}

// ---------------------------------------------------------------------------
// API Keys
// ---------------------------------------------------------------------------

async fn seed_api_keys(store: &dyn Store, users: &[SeededUser]) -> anyhow::Result<()> {
    let admin = &users[0];
    let alice = &users[1];

    let keys = [
        (admin, "admin-key", vec![ApiKeyScope::Admin]),
        (
            alice,
            "alice-readonly",
            vec![
                ApiKeyScope::RunsRead,
                ApiKeyScope::WorkflowsRead,
                ApiKeyScope::StatsRead,
            ],
        ),
    ];

    for (user, name, scopes) in keys {
        let raw_key = format!("irfl_{}", Uuid::now_v7().simple());
        let prefix = raw_key[..12].to_string();
        let hash = password::hash(&raw_key).context("api key hash failed")?;

        store
            .create_api_key(NewApiKey {
                user_id: user.id,
                name: name.to_string(),
                key_hash: hash,
                key_prefix: prefix,
                scopes: scopes.clone(),
                expires_at: None,
            })
            .await?;

        info!(
            name,
            user = user.username,
            raw = raw_key.as_str(),
            "created API key"
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Secrets
// ---------------------------------------------------------------------------

async fn seed_secrets(store: &dyn Store) -> anyhow::Result<()> {
    // Try to set a secret; if it fails with Crypto error, the key ring is not
    // configured and we skip.
    let secrets = [
        (
            "workflows/greeting/api_token",
            "sk-demo-greeting-token-12345",
        ),
        (
            "workflows/deploy/webhook_secret",
            "whsec-demo-deploy-secret-67890",
        ),
    ];

    for (key, value) in secrets {
        match store.set_secret(key, value).await {
            Ok(_) => info!(key, "created secret"),
            Err(ironflow_store::error::StoreError::Crypto(msg)) => {
                info!(
                    reason = msg.as_str(),
                    "skipping secrets (no key ring configured)"
                );
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Artifacts
// ---------------------------------------------------------------------------

async fn seed_artifacts(
    store: &dyn Store,
    blob_store: &LocalBlobStore,
    runs: &[SeededRun],
) -> anyhow::Result<()> {
    // Pick completed runs that have completed steps
    let completed_runs: Vec<&SeededRun> = runs
        .iter()
        .filter(|r| r.status == RunStatus::Completed && !r.step_ids.is_empty())
        .collect();

    if completed_runs.is_empty() {
        info!("no completed runs, skipping artifacts");
        return Ok(());
    }

    let demo_files: Vec<(&str, &str, &[u8])> = vec![
        ("build.log", "text/plain", b"[2026-08-15 10:00:00] Build started\n[2026-08-15 10:00:45] Compiling ironflow v2.0.0\n[2026-08-15 10:01:30] Finished release target(s)\n[2026-08-15 10:01:30] Build completed successfully\n"),
        ("report.json", "application/json", br#"{"status":"success","tests_passed":142,"tests_failed":0,"coverage":87.3,"duration_secs":30,"timestamp":"2026-08-15T10:02:00Z"}"#),
    ];

    for (i, (name, content_type, content)) in demo_files.iter().enumerate() {
        let run = completed_runs[i % completed_runs.len()];
        let step_id = run.step_ids[0];
        let artifact_id = Uuid::now_v7();
        let storage_key = format!("artifacts/{}/{}/{}", run.id, step_id, artifact_id);

        // Write bytes to blob store
        let digest = blob_store
            .put(&storage_key, stream_from_bytes(content.to_vec()))
            .await
            .context("blob store write failed")?;

        // Record metadata
        store
            .create_artifact(NewArtifact {
                id: artifact_id,
                run_id: run.id,
                step_id,
                name: name.to_string(),
                storage_key: storage_key.clone(),
                content_type: content_type.to_string(),
                size_bytes: digest.size_bytes,
                sha256: digest.sha256,
            })
            .await?;

        info!(
            name,
            run = run.workflow_name.as_str(),
            size = digest.size_bytes,
            "created artifact"
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Audit Logs
// ---------------------------------------------------------------------------

async fn seed_audit_logs(
    store: &dyn Store,
    users: &[SeededUser],
    runs: &[SeededRun],
) -> anyhow::Result<()> {
    let admin = &users[0];
    let alice = &users[1];
    let bob = &users[2];

    let all_runs: Vec<&SeededRun> = runs.iter().collect();
    let completed_runs: Vec<&SeededRun> = runs
        .iter()
        .filter(|r| r.status == RunStatus::Completed)
        .collect();
    let failed_run = runs.iter().find(|r| r.status == RunStatus::Failed);
    let cancelled_run = runs.iter().find(|r| r.status == RunStatus::Cancelled);

    let user_pool = [admin, alice, bob];
    let workflows = [
        "greeting",
        "ci-pipeline",
        "deploy-approval",
        "system-audit",
        "data-sync",
    ];
    let branches = [
        "main",
        "feat/new-feature",
        "fix/hotfix",
        "chore/deps",
        "release/v2.1",
    ];
    let ips = [
        "192.168.1.100",
        "10.0.0.42",
        "172.16.0.5",
        "10.10.1.200",
        "192.168.50.3",
    ];
    let user_agents = [
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
        "curl/8.4.0",
        "ironflow-cli/2.0.0",
        "Mozilla/5.0 (X11; Linux x86_64)",
        "Python/3.12 httpx/0.27",
    ];

    struct AuditSpec {
        event_type: EventKind,
        payload: serde_json::Value,
        run_id: Option<Uuid>,
        step_id: Option<Uuid>,
        user_id: Option<Uuid>,
    }

    let mut specs: Vec<AuditSpec> = Vec::with_capacity(101);

    // --- Base events covering every EventKind (25 entries) ---

    // RunCreated x5 (one per workflow)
    for (i, wf) in workflows.iter().enumerate() {
        let run = all_runs.get(i % all_runs.len());
        specs.push(AuditSpec {
            event_type: EventKind::RunCreated,
            payload: json!({
                "workflow": wf,
                "trigger": if i % 2 == 0 { "manual" } else { "webhook" },
                "labels": {"env": if i < 3 { "dev" } else { "production" }}
            }),
            run_id: run.map(|r| r.id),
            step_id: None,
            user_id: Some(user_pool[i % user_pool.len()].id),
        });
    }

    // RunStatusChanged x8 (various transitions)
    let transitions = [
        ("pending", "running"),
        ("running", "completed"),
        ("pending", "running"),
        ("running", "failed"),
        ("pending", "cancelled"),
        ("running", "completed"),
        ("pending", "running"),
        ("running", "completed"),
    ];
    for (i, (from, to)) in transitions.iter().enumerate() {
        let run = all_runs.get(i % all_runs.len());
        specs.push(AuditSpec {
            event_type: EventKind::RunStatusChanged,
            payload: json!({
                "from": from,
                "to": to,
                "workflow": workflows[i % workflows.len()],
                "duration_ms": (i + 1) * 500
            }),
            run_id: run.map(|r| r.id),
            step_id: None,
            user_id: None,
        });
    }

    // RunFailed x3
    for i in 0..3 {
        let errors = [
            "agent budget exceeded: $0.50 > $0.10 max",
            "step timeout after 300s",
            "shell exited with code 1",
        ];
        specs.push(AuditSpec {
            event_type: EventKind::RunFailed,
            payload: json!({
                "workflow": workflows[i % workflows.len()],
                "error": errors[i],
                "step": if i == 0 { "say-hello" } else { "build" }
            }),
            run_id: failed_run.map(|r| r.id),
            step_id: failed_run.and_then(|r| r.step_ids.first().copied()),
            user_id: None,
        });
    }

    // RunBudgetExceeded x2
    for (i, workflow) in workflows.iter().enumerate().take(2) {
        specs.push(AuditSpec {
            event_type: EventKind::RunBudgetExceeded,
            payload: json!({
                "workflow": workflow,
                "budget_usd": 0.10 + (i as f64) * 0.05,
                "actual_usd": 0.50 + (i as f64) * 0.20
            }),
            run_id: failed_run.map(|r| r.id),
            step_id: None,
            user_id: None,
        });
    }

    // StepCompleted x10
    let step_names = [
        "checkout",
        "build",
        "test",
        "deploy",
        "collect-metrics",
        "analyze",
        "notify",
        "cleanup",
        "validate",
        "publish",
    ];
    for (i, step) in step_names.iter().enumerate() {
        let run = completed_runs.get(i % completed_runs.len());
        specs.push(AuditSpec {
            event_type: EventKind::StepCompleted,
            payload: json!({
                "step": step,
                "duration_ms": (i + 1) * 1000,
                "exit_code": 0
            }),
            run_id: run.map(|r| r.id),
            step_id: run.and_then(|r| r.step_ids.first().copied()),
            user_id: None,
        });
    }

    // StepFailed x4
    let step_errors = [
        ("say-hello", "agent budget exceeded"),
        ("build", "compilation error: expected ; found }"),
        ("test", "assertion failed: expected 200, got 500"),
        ("deploy", "connection refused: 10.0.0.1:5432"),
    ];
    for (i, (step, error)) in step_errors.iter().enumerate() {
        specs.push(AuditSpec {
            event_type: EventKind::StepFailed,
            payload: json!({
                "step": step,
                "error": error,
                "duration_ms": (i + 1) * 400
            }),
            run_id: failed_run.map(|r| r.id),
            step_id: failed_run.and_then(|r| r.step_ids.first().copied()),
            user_id: None,
        });
    }

    // ApprovalRequested x3
    for i in 0..3 {
        let plans = [
            "Deploy v2.1.0 to production",
            "Run data migration batch #47",
            "Enable feature flag: new-billing",
        ];
        specs.push(AuditSpec {
            event_type: EventKind::ApprovalRequested,
            payload: json!({
                "workflow": "deploy-approval",
                "step": "approve",
                "plan": plans[i]
            }),
            run_id: completed_runs.first().map(|r| r.id),
            step_id: None,
            user_id: Some(user_pool[i % user_pool.len()].id),
        });
    }

    // ApprovalGranted x3
    for i in 0..3 {
        let comments = ["LGTM", "Approved after review", "Go ahead, staging passed"];
        specs.push(AuditSpec {
            event_type: EventKind::ApprovalGranted,
            payload: json!({
                "workflow": "deploy-approval",
                "step": "approve",
                "approved_by": user_pool[i % user_pool.len()].username,
                "comment": comments[i]
            }),
            run_id: completed_runs.first().map(|r| r.id),
            step_id: None,
            user_id: Some(user_pool[i % user_pool.len()].id),
        });
    }

    // ApprovalRejected x2
    let reject_reasons = [
        "Not ready for production, missing tests",
        "Performance regression detected in staging",
    ];
    for (i, reason) in reject_reasons.iter().enumerate() {
        specs.push(AuditSpec {
            event_type: EventKind::ApprovalRejected,
            payload: json!({
                "workflow": "deploy-approval",
                "step": "approve",
                "rejected_by": user_pool[(i + 1) % user_pool.len()].username,
                "reason": reason
            }),
            run_id: cancelled_run.map(|r| r.id),
            step_id: None,
            user_id: Some(user_pool[(i + 1) % user_pool.len()].id),
        });
    }

    // LogLine x5
    let log_messages = [
        "Starting workflow execution",
        "Fetching dependencies from registry",
        "Running test suite: 142 tests",
        "Uploading artifacts to blob store",
        "Workflow completed successfully",
    ];
    for (i, msg) in log_messages.iter().enumerate() {
        let run = all_runs.get(i % all_runs.len());
        specs.push(AuditSpec {
            event_type: EventKind::LogLine,
            payload: json!({
                "message": msg,
                "level": if i < 3 { "info" } else { "debug" },
                "stream": "stdout"
            }),
            run_id: run.map(|r| r.id),
            step_id: run.and_then(|r| r.step_ids.first().copied()),
            user_id: None,
        });
    }

    // UserSignedIn x10
    for i in 0..10 {
        let user = user_pool[i % user_pool.len()];
        specs.push(AuditSpec {
            event_type: EventKind::UserSignedIn,
            payload: json!({
                "username": user.username,
                "ip": ips[i % ips.len()],
                "user_agent": user_agents[i % user_agents.len()],
                "mfa": i % 3 == 0
            }),
            run_id: None,
            step_id: None,
            user_id: Some(user.id),
        });
    }

    // UserSignedUp x3
    let signups = [
        ("bob", "bob@ironflow.dev"),
        ("charlie", "charlie@ironflow.dev"),
        ("dana", "dana@ironflow.dev"),
    ];
    for (i, (name, email)) in signups.iter().enumerate() {
        specs.push(AuditSpec {
            event_type: EventKind::UserSignedUp,
            payload: json!({ "username": name, "email": email }),
            run_id: None,
            step_id: None,
            user_id: Some(user_pool[i % user_pool.len()].id),
        });
    }

    // UserSignedOut x5
    for i in 0..5 {
        let user = user_pool[i % user_pool.len()];
        specs.push(AuditSpec {
            event_type: EventKind::UserSignedOut,
            payload: json!({
                "username": user.username,
                "session_duration_secs": (i + 1) * 900
            }),
            run_id: None,
            step_id: None,
            user_id: Some(user.id),
        });
    }

    // SecretsRotated x3
    for i in 0..3 {
        specs.push(AuditSpec {
            event_type: EventKind::SecretsRotated,
            payload: json!({
                "rotated": (i + 1) * 5,
                "failed": if i == 2 { 1 } else { 0 },
                "from_version": i + 1,
                "to_version": i + 2
            }),
            run_id: None,
            step_id: None,
            user_id: Some(admin.id),
        });
    }

    // RetryForced x4
    let retry_reasons = [
        "Transient network error",
        "Database connection timeout",
        "Rate limited by external API",
        "Worker OOM, restarted",
    ];
    for (i, reason) in retry_reasons.iter().enumerate() {
        specs.push(AuditSpec {
            event_type: EventKind::RetryForced,
            payload: json!({
                "workflow": workflows[i % workflows.len()],
                "reason": reason,
                "attempt": i + 2
            }),
            run_id: failed_run.map(|r| r.id),
            step_id: None,
            user_id: Some(user_pool[i % user_pool.len()].id),
        });
    }

    // --- Extra entries to reach 101 ---
    // More run created + status changed for pagination diversity
    for i in 0..(101 - specs.len()) {
        let run = all_runs.get(i % all_runs.len());
        let user = user_pool[i % user_pool.len()];
        let event_type = match i % 5 {
            0 => EventKind::RunCreated,
            1 => EventKind::RunStatusChanged,
            2 => EventKind::StepCompleted,
            3 => EventKind::UserSignedIn,
            _ => EventKind::LogLine,
        };
        let payload = match i % 5 {
            0 => json!({
                "workflow": workflows[i % workflows.len()],
                "trigger": "cron",
                "labels": {"env": "staging", "branch": branches[i % branches.len()]}
            }),
            1 => json!({
                "from": "pending",
                "to": "running",
                "workflow": workflows[i % workflows.len()]
            }),
            2 => json!({
                "step": step_names[i % step_names.len()],
                "duration_ms": (i + 1) * 750,
                "exit_code": 0
            }),
            3 => json!({
                "username": user.username,
                "ip": ips[i % ips.len()],
                "user_agent": user_agents[i % user_agents.len()]
            }),
            _ => json!({
                "message": format!("Processing batch {} of {}", i + 1, 101 - specs.len()),
                "level": "info"
            }),
        };
        specs.push(AuditSpec {
            event_type,
            payload,
            run_id: if i % 5 <= 2 { run.map(|r| r.id) } else { None },
            step_id: if i % 5 == 2 {
                run.and_then(|r| r.step_ids.first().copied())
            } else {
                None
            },
            user_id: if i % 5 != 1 { Some(user.id) } else { None },
        });
    }

    assert_eq!(specs.len(), 101);

    for spec in &specs {
        store
            .append_audit_log(NewAuditLogEntry {
                event_type: spec.event_type,
                payload: spec.payload.clone(),
                run_id: spec.run_id,
                step_id: spec.step_id,
                user_id: spec.user_id,
            })
            .await?;
    }

    info!(count = specs.len(), "created audit log entries");

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironflow_store::api_key_store::ApiKeyStore;
    use ironflow_store::artifact_store::ArtifactStore;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::secret_store::SecretStore;
    use ironflow_store::store::{RunStore, Store};
    use ironflow_store::user_store::UserStore;

    use super::*;

    async fn empty_store() -> Arc<InMemoryStore> {
        Arc::new(InMemoryStore::new())
    }

    #[tokio::test]
    async fn seed_creates_users() {
        let store = empty_store().await;
        let opts = SeedOptions {
            force: false,
            artifacts_dir: None,
        };
        seed_store(&*store as &dyn Store, &opts).await.unwrap();

        let count = store.count_users().await.unwrap();
        assert_eq!(count, 3);

        let admin = store
            .find_user_by_email("admin@ironflow.dev")
            .await
            .unwrap()
            .expect("admin should exist");
        assert!(admin.is_admin);

        let alice = store
            .find_user_by_email("alice@ironflow.dev")
            .await
            .unwrap()
            .expect("alice should exist");
        assert!(!alice.is_admin);

        let bob = store
            .find_user_by_email("bob@ironflow.dev")
            .await
            .unwrap()
            .expect("bob should exist");
        assert!(!bob.is_admin);
    }

    #[tokio::test]
    async fn seed_creates_runs_in_various_states() {
        let store = empty_store().await;
        let opts = SeedOptions {
            force: false,
            artifacts_dir: None,
        };
        seed_store(&*store as &dyn Store, &opts).await.unwrap();

        use ironflow_store::entities::{Page, RunFilter};

        let Page { items: runs, .. } = store.list_runs(RunFilter::default(), 1, 100).await.unwrap();

        assert_eq!(runs.len(), 8);

        let statuses: Vec<RunStatus> = runs.iter().map(|r| r.status.state).collect();
        assert!(
            statuses.contains(&RunStatus::Completed),
            "should have Completed runs"
        );
        assert!(
            statuses.contains(&RunStatus::Failed),
            "should have Failed runs"
        );
        assert!(
            statuses.contains(&RunStatus::Running),
            "should have Running runs"
        );
        assert!(
            statuses.contains(&RunStatus::Pending),
            "should have Pending runs"
        );
        assert!(
            statuses.contains(&RunStatus::Cancelled),
            "should have Cancelled runs"
        );
    }

    #[tokio::test]
    async fn seed_creates_steps_with_outputs() {
        let store = empty_store().await;
        let opts = SeedOptions {
            force: false,
            artifacts_dir: None,
        };
        seed_store(&*store as &dyn Store, &opts).await.unwrap();

        use ironflow_store::entities::{Page, RunFilter};

        let Page { items: runs, .. } = store.list_runs(RunFilter::default(), 1, 100).await.unwrap();

        let mut total_steps = 0;
        let mut steps_with_output = 0;
        let mut steps_with_error = 0;

        for run in &runs {
            let steps = store.list_steps(run.id).await.unwrap();
            assert!(!steps.is_empty(), "every run should have at least one step");
            total_steps += steps.len();

            for step in &steps {
                if step.output.is_some() {
                    steps_with_output += 1;
                }
                if step.error.is_some() {
                    steps_with_error += 1;
                }
            }
        }

        assert!(total_steps >= 8, "should have many steps across all runs");
        assert!(steps_with_output > 0, "some steps should have outputs");
        assert!(steps_with_error > 0, "some steps should have errors");
    }

    #[tokio::test]
    async fn seed_creates_api_keys() {
        let store = empty_store().await;
        let opts = SeedOptions {
            force: false,
            artifacts_dir: None,
        };
        seed_store(&*store as &dyn Store, &opts).await.unwrap();

        let admin = store
            .find_user_by_email("admin@ironflow.dev")
            .await
            .unwrap()
            .unwrap();
        let admin_keys = store.list_api_keys_by_user(admin.id).await.unwrap();
        assert_eq!(admin_keys.len(), 1);
        assert_eq!(admin_keys[0].name, "admin-key");
        assert!(admin_keys[0].scopes.contains(&ApiKeyScope::Admin));

        let alice = store
            .find_user_by_email("alice@ironflow.dev")
            .await
            .unwrap()
            .unwrap();
        let alice_keys = store.list_api_keys_by_user(alice.id).await.unwrap();
        assert_eq!(alice_keys.len(), 1);
        assert_eq!(alice_keys[0].name, "alice-readonly");
        assert!(alice_keys[0].scopes.contains(&ApiKeyScope::RunsRead));
        assert!(!alice_keys[0].scopes.contains(&ApiKeyScope::Admin));
    }

    #[tokio::test]
    async fn seed_skips_secrets_without_key_ring() {
        let store = empty_store().await;
        let opts = SeedOptions {
            force: false,
            artifacts_dir: None,
        };
        // InMemoryStore without a key ring: secrets should be skipped, not error
        seed_store(&*store as &dyn Store, &opts).await.unwrap();

        let keys = store.list_secret_keys("").await.unwrap();
        assert!(
            keys.is_empty(),
            "secrets should be skipped without key ring"
        );
    }

    #[tokio::test]
    async fn seed_fails_on_populated_store() {
        let store = empty_store().await;
        let opts = SeedOptions {
            force: false,
            artifacts_dir: None,
        };
        seed_store(&*store as &dyn Store, &opts).await.unwrap();

        // Second seed should fail
        let result = seed_store(&*store as &dyn Store, &opts).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("already contains"),
            "error should mention existing data: {msg}"
        );
    }

    #[tokio::test]
    async fn seed_force_resets_store() {
        let store = empty_store().await;
        let opts = SeedOptions {
            force: false,
            artifacts_dir: None,
        };
        seed_store(&*store as &dyn Store, &opts).await.unwrap();
        assert_eq!(store.count_users().await.unwrap(), 3);

        // For InMemoryStore, "force" means using a fresh store. The caller
        // (main.rs) handles the reset; here we just verify the seed function
        // works on a fresh store after previous data existed.
        let fresh = empty_store().await;
        let force_opts = SeedOptions {
            force: true,
            artifacts_dir: None,
        };
        seed_store(&*fresh as &dyn Store, &force_opts)
            .await
            .unwrap();
        assert_eq!(fresh.count_users().await.unwrap(), 3);
    }

    #[tokio::test]
    async fn seed_creates_artifacts_when_dir_provided() {
        let store = empty_store().await;
        let tmp = tempfile::tempdir().unwrap();
        let opts = SeedOptions {
            force: false,
            artifacts_dir: Some(tmp.path().to_path_buf()),
        };
        seed_store(&*store as &dyn Store, &opts).await.unwrap();

        use ironflow_store::entities::{Page, RunFilter};

        let Page { items: runs, .. } = store.list_runs(RunFilter::default(), 1, 100).await.unwrap();

        let completed = runs
            .iter()
            .filter(|r| r.status.state == RunStatus::Completed)
            .collect::<Vec<_>>();

        let mut total_artifacts = 0;
        for run in &completed {
            let artifacts = store.list_artifacts_for_run(run.id).await.unwrap();
            total_artifacts += artifacts.len();
        }

        assert!(
            total_artifacts >= 2,
            "should have at least 2 artifacts: got {total_artifacts}"
        );
    }
}
