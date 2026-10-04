#![cfg(feature = "store-postgres")]

//! Integration tests for the run and step state machines on PostgreSQL.
//!
//! The SQL state machines (`run_lifecycle`, `step_lifecycle`) are written by
//! migrations, while [`RunStatus`] and the in-memory store encode the same
//! machines in Rust. Nothing ties the two together at compile time, and the
//! in-memory store accepts whatever Rust allows, so a transition missing from
//! SQL only shows up on a real database. These tests are that database.
//!
//! Run them with:
//!
//! ```sh
//! scripts/test-postgres.sh -p ironflow-store --features store-postgres \
//!     --test postgres_fsm -- --ignored
//! ```

use std::collections::HashMap;
use std::env::var;
use std::fmt::{Debug, Display, Formatter, Result as FmtResult};
use std::time::Duration;

use ironflow_store::entities::{
    NewRun, NewStep, RunStatus, StepKind, StepStatus, StepUpdate, TriggerKind, step_trace_id,
};
use ironflow_store::error::StoreError;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::postgres::{PoolConfig, PostgresStore};
use ironflow_store::store::RunStore;
use serde_json::json;
use sqlx::{Executor, PgPool, migrate, query_scalar};
use strum::IntoEnumIterator;
use tokio::time::timeout;
use uuid::Uuid;

const TEST_TIMEOUT: Duration = Duration::from_secs(60);

fn database_url() -> String {
    var("DATABASE_URL").expect("DATABASE_URL must be set")
}

/// Connect and migrate with a pool that waits as long as the test may run.
///
/// The default 5s acquire timeout is tighter than [`TEST_TIMEOUT`]: on a loaded
/// CI runner, the first connection to a freshly created database can take
/// longer than that and the test failed with "pool timed out" well within its
/// budget.
async fn connect(url: &str) -> Result<PostgresStore, StoreError> {
    let config = PoolConfig {
        acquire_timeout: TEST_TIMEOUT,
        ..PoolConfig::default()
    };
    PostgresStore::with_config(url, config).await
}

async fn postgres_store() -> PostgresStore {
    connect(&database_url())
        .await
        .expect("failed to connect to PostgreSQL")
}

fn new_run() -> NewRun {
    NewRun {
        workflow_name: "fsm-parity".to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        created_by: None,
        idempotency_key: None,
        concurrency_key: None,
        max_cost_usd: None,
    }
}

/// Statuses a fresh run (created `Pending`) goes through to reach `status`.
///
/// Exhaustive on purpose: a new [`RunStatus`] variant stops this file from
/// compiling until it says how the variant is reached. From then on, every
/// transition into and out of it is compared across both stores.
fn run_path(status: RunStatus) -> &'static [RunStatus] {
    match status {
        RunStatus::Pending => &[],
        RunStatus::Running => &[RunStatus::Running],
        RunStatus::Completed => &[RunStatus::Running, RunStatus::Completed],
        RunStatus::Failed => &[RunStatus::Running, RunStatus::Failed],
        RunStatus::Retrying => &[RunStatus::Running, RunStatus::Retrying],
        RunStatus::Cancelled => &[RunStatus::Cancelled],
        RunStatus::AwaitingApproval => &[RunStatus::Running, RunStatus::AwaitingApproval],
        RunStatus::Warning => &[RunStatus::Running, RunStatus::Warning],
        RunStatus::Sleeping => &[RunStatus::Running, RunStatus::Sleeping],
    }
}

/// How a store answered one `from -> to` transition attempt.
enum Outcome<S> {
    /// The walk to `from` failed on the transition into `stuck_at`.
    Unreachable { stuck_at: S, error: StoreError },
    /// The transition went through; the status read back afterwards.
    Accepted(S),
    /// The transition was refused.
    Refused(StoreError),
}

impl<S: Debug> Display for Outcome<S> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Unreachable { stuck_at, error } => {
                write!(f, "cannot walk into {stuck_at:?}: {error}")
            }
            Self::Accepted(status) => write!(f, "accepted, now {status:?}"),
            Self::Refused(error) => write!(f, "refused: {error}"),
        }
    }
}

impl<S: PartialEq> Outcome<S> {
    /// Same verdict, whatever the error text (each store words it its own way).
    fn same_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Accepted(a), Self::Accepted(b)) => a == b,
            (Self::Refused(_), Self::Refused(_)) => true,
            _ => false,
        }
    }
}

async fn attempt_run_transition(
    store: &impl RunStore,
    from: RunStatus,
    to: RunStatus,
) -> Outcome<RunStatus> {
    let run = store
        .create_run(new_run())
        .await
        .expect("create run")
        .into_run();
    for &status in run_path(from) {
        if let Err(error) = store.update_run_status(run.id, status).await {
            return Outcome::Unreachable {
                stuck_at: status,
                error,
            };
        }
    }
    match store.update_run_status(run.id, to).await {
        Ok(()) => {
            let run = store
                .get_run(run.id)
                .await
                .expect("get run")
                .expect("run exists");
            Outcome::Accepted(run.status.state)
        }
        Err(error) => Outcome::Refused(error),
    }
}

/// Statuses a fresh step (created `Pending`) goes through to reach `status`.
///
/// Exhaustive for the same reason as [`run_path`].
fn step_path(status: StepStatus) -> &'static [StepStatus] {
    match status {
        StepStatus::Pending => &[],
        StepStatus::Running => &[StepStatus::Running],
        StepStatus::Completed => &[StepStatus::Running, StepStatus::Completed],
        StepStatus::Failed => &[StepStatus::Running, StepStatus::Failed],
        StepStatus::Skipped => &[StepStatus::Skipped],
        StepStatus::AwaitingApproval => &[StepStatus::Running, StepStatus::AwaitingApproval],
        StepStatus::Rejected => &[
            StepStatus::Running,
            StepStatus::AwaitingApproval,
            StepStatus::Rejected,
        ],
    }
}

fn to_status(status: StepStatus) -> StepUpdate {
    StepUpdate {
        status: Some(status),
        ..StepUpdate::default()
    }
}

async fn attempt_step_transition(
    store: &impl RunStore,
    from: StepStatus,
    to: StepStatus,
) -> Outcome<StepStatus> {
    let run = store
        .create_run(new_run())
        .await
        .expect("create run")
        .into_run();
    let name = "fsm-parity".to_string();
    let step = store
        .create_step(NewStep {
            run_id: run.id,
            trace_id: step_trace_id(run.id, &name, 0),
            name,
            kind: StepKind::Shell,
            position: 0,
            input: None,
            is_error_handler: false,
        })
        .await
        .expect("create step");
    for &status in step_path(from) {
        if let Err(error) = store.update_step(step.id, to_status(status)).await {
            return Outcome::Unreachable {
                stuck_at: status,
                error,
            };
        }
    }
    match store.update_step(step.id, to_status(to)).await {
        Ok(()) => {
            let step = store
                .get_step(step.id)
                .await
                .expect("get step")
                .expect("step exists");
            Outcome::Accepted(step.status.state)
        }
        Err(error) => Outcome::Refused(error),
    }
}

/// Fail with the full list of divergences, not only the first one.
fn assert_no_divergence(machine: &str, mismatches: &[String]) {
    assert!(
        mismatches.is_empty(),
        "the PostgreSQL {machine} FSM diverges from the in-memory store:\n{}",
        mismatches.join("\n")
    );
}

/// `None` when Postgres gave the same verdict as the in-memory store, the
/// reference. Panics when the reference itself cannot walk to `from`: the
/// path table would then hide the pair instead of testing it.
fn divergence<S: Debug + PartialEq>(
    from: S,
    to: S,
    expected: &Outcome<S>,
    actual: &Outcome<S>,
) -> Option<String> {
    assert!(
        !matches!(expected, Outcome::Unreachable { .. }),
        "the path to {from:?} is not valid on the in-memory store: {expected}"
    );
    if matches!(actual, Outcome::Unreachable { .. }) {
        return Some(format!("cannot reach {from:?} on postgres: {actual}"));
    }
    (!actual.same_as(expected))
        .then(|| format!("{from:?} -> {to:?}: in-memory {expected}, postgres {actual}"))
}

/// `url` with its database replaced by `name`, query string kept.
fn with_database(url: &str, name: &str) -> String {
    let authority = url.find("://").expect("DATABASE_URL has no scheme") + 3;
    let path = url[authority..]
        .find('/')
        .map_or(url.len(), |offset| authority + offset);
    let query = url[path..]
        .find('?')
        .map_or("", |offset| &url[path + offset..]);
    format!("{}/{name}{query}", &url[..path])
}

/// A database created empty for one test, so migrations run from scratch
/// whatever state the shared `DATABASE_URL` database is in.
struct BlankDatabase {
    admin: PgPool,
    name: String,
    url: String,
}

impl BlankDatabase {
    async fn create() -> Self {
        let admin_url = database_url();
        let admin = PgPool::connect(&admin_url)
            .await
            .expect("failed to connect to PostgreSQL");
        let name = format!("ironflow_blank_{}", Uuid::now_v7().simple());
        admin
            .execute(format!(r#"CREATE DATABASE "{name}""#).as_str())
            .await
            .expect("failed to create the blank database");
        let url = with_database(&admin_url, &name);
        Self { admin, name, url }
    }

    async fn drop(self) {
        self.admin
            .execute(format!(r#"DROP DATABASE "{}" WITH (FORCE)"#, self.name).as_str())
            .await
            .expect("failed to drop the blank database");
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn migrations_apply_on_a_blank_database() {
    timeout(TEST_TIMEOUT, async {
        let db = BlankDatabase::create().await;

        let migrated = connect(&db.url).await.map(|_| ());
        db.drop().await;

        migrated.expect("migrations must apply on a blank database");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn every_run_transition_matches_the_in_memory_store() {
    timeout(TEST_TIMEOUT, async {
        let memory = InMemoryStore::new();
        let postgres = postgres_store().await;

        let mut mismatches = Vec::new();
        for from in RunStatus::iter() {
            for to in RunStatus::iter() {
                let expected = attempt_run_transition(&memory, from, to).await;
                let actual = attempt_run_transition(&postgres, from, to).await;
                mismatches.extend(divergence(from, to, &expected, &actual));
                if matches!(actual, Outcome::Unreachable { .. }) {
                    // Every other target would report the same failed walk.
                    break;
                }
            }
        }

        assert_no_divergence("run_lifecycle", &mismatches);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn every_step_transition_matches_the_in_memory_store() {
    timeout(TEST_TIMEOUT, async {
        let memory = InMemoryStore::new();
        let postgres = postgres_store().await;

        let mut mismatches = Vec::new();
        for from in StepStatus::iter() {
            for to in StepStatus::iter() {
                let expected = attempt_step_transition(&memory, from, to).await;
                let actual = attempt_step_transition(&postgres, from, to).await;
                mismatches.extend(divergence(from, to, &expected, &actual));
                if matches!(actual, Outcome::Unreachable { .. }) {
                    // Every other target would report the same failed walk.
                    break;
                }
            }
        }

        assert_no_divergence("step_lifecycle", &mismatches);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn a_transition_refused_by_run_status_leaves_the_run_in_place() {
    timeout(TEST_TIMEOUT, async {
        let store = postgres_store().await;
        let run = store
            .create_run(new_run())
            .await
            .expect("create run")
            .into_run();
        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .expect("to running");
        store
            .update_run_status(run.id, RunStatus::AwaitingApproval)
            .await
            .expect("to awaiting approval");

        let refused = store.update_run_status(run.id, RunStatus::Completed).await;

        assert!(
            matches!(
                refused,
                Err(StoreError::InvalidTransition {
                    from: RunStatus::AwaitingApproval,
                    to: RunStatus::Completed,
                })
            ),
            "expected InvalidTransition, got {refused:?}"
        );
        let run = store
            .get_run(run.id)
            .await
            .expect("get run")
            .expect("run exists");
        assert_eq!(run.status.state, RunStatus::AwaitingApproval);
    })
    .await
    .expect("test timed out");
}

/// Last migration before the run_lifecycle FSM caught up with [`RunStatus`].
const BEFORE_RUN_FSM_STATES: i64 = 20260925120000;

async fn run_lifecycle_states_named(pool: &PgPool, state: &str) -> i64 {
    query_scalar!(
        r#"SELECT count(*) AS "count!" FROM lib_fsm.abstract_state s
         JOIN lib_fsm.abstract_state_machine m USING (abstract_machine__id)
         WHERE m.name = 'run_lifecycle' AND s.name = $1"#,
        state,
    )
    .fetch_one(pool)
    .await
    .expect("count states")
}

async fn run_lifecycle_transitions_touching(pool: &PgPool, state: &str) -> i64 {
    query_scalar!(
        r#"SELECT count(*) AS "count!" FROM lib_fsm.abstract_transition t
         JOIN lib_fsm.abstract_state f ON f.abstract_state__id = t.from_abstract_state__id
         JOIN lib_fsm.abstract_state g ON g.abstract_state__id = t.to_abstract_state__id
         JOIN lib_fsm.abstract_state_machine m ON m.abstract_machine__id = f.abstract_machine__id
         WHERE m.name = 'run_lifecycle' AND $1 IN (f.name, g.name)"#,
        state,
    )
    .fetch_one(pool)
    .await
    .expect("count transitions")
}

/// Create a run and walk it to `status`.
async fn run_in(store: &PostgresStore, status: RunStatus) -> Uuid {
    let run = store
        .create_run(new_run())
        .await
        .expect("create run")
        .into_run();
    for &step in run_path(status) {
        store
            .update_run_status(run.id, step)
            .await
            .expect("walk the run");
    }
    run.id
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn run_fsm_state_migrations_revert_and_reapply() {
    timeout(TEST_TIMEOUT, async {
        let db = BlankDatabase::create().await;
        let store = connect(&db.url)
            .await
            .expect("migrations must apply on a blank database");
        let pool = PgPool::connect(&db.url)
            .await
            .expect("failed to connect to the blank database");
        let migrator = migrate!("./migrations");

        assert_eq!(
            run_lifecycle_transitions_touching(&pool, "awaiting_approval").await,
            5
        );
        assert_eq!(
            run_lifecycle_transitions_touching(&pool, "sleeping").await,
            4
        );
        let awaiting = run_in(&store, RunStatus::AwaitingApproval).await;
        let sleeping = run_in(&store, RunStatus::Sleeping).await;

        migrator
            .undo(&pool, BEFORE_RUN_FSM_STATES)
            .await
            .expect("down migrations must apply");

        for state in ["awaiting_approval", "sleeping"] {
            assert_eq!(run_lifecycle_states_named(&pool, state).await, 0, "{state}");
            assert_eq!(
                run_lifecycle_transitions_touching(&pool, state).await,
                0,
                "{state}"
            );
        }
        for id in [awaiting, sleeping] {
            let run = store
                .get_run(id)
                .await
                .expect("get run")
                .expect("run exists");
            assert_eq!(run.status.state, RunStatus::Failed);
            assert!(run.error.is_some(), "a rolled-back run says why it failed");
            assert!(run.completed_at.is_some(), "a failed run is completed");
        }

        migrator
            .run(&pool)
            .await
            .expect("up migrations must reapply");

        assert_eq!(
            run_lifecycle_transitions_touching(&pool, "awaiting_approval").await,
            5
        );
        assert_eq!(
            run_lifecycle_transitions_touching(&pool, "sleeping").await,
            4
        );

        pool.close().await;
        drop(store);
        db.drop().await;
    })
    .await
    .expect("test timed out");
}

#[test]
fn with_database_replaces_only_the_database() {
    assert_eq!(
        with_database("postgres://u:p@host:5432/ironflow", "blank"),
        "postgres://u:p@host:5432/blank"
    );
    assert_eq!(
        with_database("postgres://u:p@host/ironflow?sslmode=disable", "blank"),
        "postgres://u:p@host/blank?sslmode=disable"
    );
    assert_eq!(
        with_database("postgres://u:p@host:5432", "blank"),
        "postgres://u:p@host:5432/blank"
    );
}
