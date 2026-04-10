//! In-memory [`RunStore`] implementation for development and testing.
//!
//! [`InMemoryStore`] uses `Arc<RwLock<..>>` internally, making it safe to share
//! across tasks. Data is lost when the process exits.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_store::prelude::*;
//! use serde_json::json;
//!
//! # async fn example() -> Result<(), ironflow_store::error::StoreError> {
//! let store = InMemoryStore::new();
//!
//! let run = store.create_run(NewRun {
//!     workflow_name: "test".to_string(),
//!     trigger: TriggerKind::Manual,
//!     payload: json!({}),
//!     max_retries: 3,
//! }).await?;
//!
//! assert_eq!(run.status.state, RunStatus::Pending);
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::RwLock;
use uuid::Uuid;

use rust_decimal::Decimal;

use crate::entities::{
    FsmState, NewRun, NewStep, NewStepDependency, NewUser, Page, Run, RunFilter, RunStats,
    RunStatus, RunUpdate, Step, StepDependency, StepStatus, StepUpdate, User,
};
use crate::error::StoreError;
use crate::store::{RunStore, StoreFuture};
use crate::user_store::UserStore;

#[derive(Debug, Default)]
struct State {
    runs: HashMap<Uuid, Run>,
    steps: HashMap<Uuid, Step>,
    step_dependencies: Vec<StepDependency>,
    users: HashMap<Uuid, User>,
}

/// In-memory store backed by `Arc<RwLock<..>>`.
///
/// Thread-safe and cheap to clone. All data is held in memory and lost on drop.
///
/// # Examples
///
/// ```
/// use ironflow_store::memory::InMemoryStore;
///
/// let store = InMemoryStore::new();
/// let store2 = store.clone(); // cheap Arc clone
/// ```
#[derive(Debug, Clone)]
pub struct InMemoryStore {
    state: Arc<RwLock<State>>,
}

impl InMemoryStore {
    /// Create a new empty in-memory store.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::memory::InMemoryStore;
    ///
    /// let store = InMemoryStore::new();
    /// ```
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(State::default())),
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl RunStore for InMemoryStore {
    fn create_run(&self, req: NewRun) -> StoreFuture<'_, Run> {
        Box::pin(async move {
            let now = Utc::now();
            let run = Run {
                id: Uuid::now_v7(),
                workflow_name: req.workflow_name,
                status: FsmState::new(RunStatus::Pending, Uuid::now_v7()),
                trigger: req.trigger,
                payload: req.payload,
                error: None,
                retry_count: 0,
                max_retries: req.max_retries,
                cost_usd: Decimal::ZERO,
                duration_ms: 0,
                created_at: now,
                updated_at: now,
                started_at: None,
                completed_at: None,
            };

            let mut state = self.state.write().await;
            state.runs.insert(run.id, run.clone());
            Ok(run)
        })
    }

    fn get_run(&self, id: Uuid) -> StoreFuture<'_, Option<Run>> {
        Box::pin(async move {
            let state = self.state.read().await;
            Ok(state.runs.get(&id).cloned())
        })
    }

    fn list_runs(&self, filter: RunFilter, page: u32, per_page: u32) -> StoreFuture<'_, Page<Run>> {
        Box::pin(async move {
            let state = self.state.read().await;

            let mut runs: Vec<&Run> = state
                .runs
                .values()
                .filter(|r| {
                    if let Some(ref wf) = filter.workflow_name
                        && !r.workflow_name.to_lowercase().contains(&wf.to_lowercase())
                    {
                        return false;
                    }
                    if let Some(ref status) = filter.status
                        && &r.status.state != status
                    {
                        return false;
                    }
                    if let Some(after) = filter.created_after
                        && r.created_at < after
                    {
                        return false;
                    }
                    if let Some(before) = filter.created_before
                        && r.created_at > before
                    {
                        return false;
                    }
                    true
                })
                .collect();

            // Sort newest first.
            runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));

            let total = runs.len() as u64;
            let page = page.max(1);
            let per_page = per_page.clamp(1, 100);
            let offset = ((page - 1) * per_page) as usize;
            let items: Vec<Run> = runs
                .into_iter()
                .skip(offset)
                .take(per_page as usize)
                .cloned()
                .collect();

            Ok(Page {
                items,
                total,
                page,
                per_page,
            })
        })
    }

    fn update_run_status(&self, id: Uuid, new_status: RunStatus) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            let run = state.runs.get_mut(&id).ok_or(StoreError::RunNotFound(id))?;

            if !run.status.state.can_transition_to(&new_status) {
                return Err(StoreError::InvalidTransition {
                    from: run.status.state,
                    to: new_status,
                });
            }

            let now = Utc::now();
            run.status.state = new_status;
            run.updated_at = now;

            if new_status == RunStatus::Running && run.started_at.is_none() {
                run.started_at = Some(now);
            }
            if new_status.is_terminal() {
                run.completed_at = Some(now);
            }

            Ok(())
        })
    }

    fn update_run(&self, id: Uuid, update: RunUpdate) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            let run = state.runs.get_mut(&id).ok_or(StoreError::RunNotFound(id))?;

            let now = Utc::now();

            if let Some(status) = update.status {
                if !run.status.state.can_transition_to(&status) {
                    return Err(StoreError::InvalidTransition {
                        from: run.status.state,
                        to: status,
                    });
                }
                run.status.state = status;
                if status == RunStatus::Running && run.started_at.is_none() {
                    run.started_at = Some(now);
                }
                if status.is_terminal() {
                    run.completed_at = Some(now);
                }
            }

            if let Some(error) = update.error {
                run.error = Some(error);
            }
            if update.increment_retry {
                run.retry_count += 1;
            }
            if let Some(cost) = update.cost_usd {
                run.cost_usd = cost;
            }
            if let Some(dur) = update.duration_ms {
                run.duration_ms = dur;
            }
            if let Some(started) = update.started_at {
                run.started_at = Some(started);
            }
            if let Some(completed) = update.completed_at {
                run.completed_at = Some(completed);
            }

            run.updated_at = now;
            Ok(())
        })
    }

    fn pick_next_pending(&self) -> StoreFuture<'_, Option<Run>> {
        Box::pin(async move {
            let mut state = self.state.write().await;

            // Find the oldest pending run.
            let oldest_id = state
                .runs
                .values()
                .filter(|r| r.status.state == RunStatus::Pending)
                .min_by_key(|r| r.created_at)
                .map(|r| r.id);

            let Some(id) = oldest_id else {
                return Ok(None);
            };

            // Transition to Running atomically.
            let run = state.runs.get_mut(&id).expect("run exists");
            let now = Utc::now();
            run.status.state = RunStatus::Running;
            run.started_at = Some(now);
            run.updated_at = now;

            Ok(Some(run.clone()))
        })
    }

    fn create_step(&self, req: NewStep) -> StoreFuture<'_, Step> {
        Box::pin(async move {
            let mut state = self.state.write().await;

            if !state.runs.contains_key(&req.run_id) {
                return Err(StoreError::RunNotFound(req.run_id));
            }

            let now = Utc::now();
            let step = Step {
                id: Uuid::now_v7(),
                run_id: req.run_id,
                name: req.name,
                kind: req.kind,
                position: req.position,
                status: FsmState::new(StepStatus::Pending, Uuid::now_v7()),
                input: req.input,
                output: None,
                error: None,
                duration_ms: 0,
                cost_usd: Decimal::ZERO,
                input_tokens: None,
                output_tokens: None,
                created_at: now,
                updated_at: now,
                started_at: None,
                completed_at: None,
            };

            state.steps.insert(step.id, step.clone());
            Ok(step)
        })
    }

    fn update_step(&self, id: Uuid, update: StepUpdate) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            let step = state
                .steps
                .get_mut(&id)
                .ok_or(StoreError::StepNotFound(id))?;

            let now = Utc::now();

            if let Some(status) = update.status {
                if !matches!(
                    (step.status.state, status),
                    (StepStatus::Pending, StepStatus::Running)
                        | (StepStatus::Pending, StepStatus::Skipped)
                        | (StepStatus::Running, StepStatus::Completed)
                        | (StepStatus::Running, StepStatus::Failed)
                        | (StepStatus::Running, StepStatus::AwaitingApproval)
                        | (StepStatus::AwaitingApproval, StepStatus::Running)
                        | (StepStatus::AwaitingApproval, StepStatus::Completed)
                        | (StepStatus::AwaitingApproval, StepStatus::Failed)
                        | (StepStatus::AwaitingApproval, StepStatus::Rejected)
                ) {
                    return Err(StoreError::Database(format!(
                        "invalid step status transition: {:?} -> {:?}",
                        step.status.state, status
                    )));
                }
                step.status.state = status;
            }
            if let Some(output) = update.output {
                step.output = Some(output);
            }
            if let Some(error) = update.error {
                step.error = Some(error);
            }
            if let Some(dur) = update.duration_ms {
                step.duration_ms = dur;
            }
            if let Some(cost) = update.cost_usd {
                step.cost_usd = cost;
            }
            if let Some(tokens) = update.input_tokens {
                step.input_tokens = Some(tokens);
            }
            if let Some(tokens) = update.output_tokens {
                step.output_tokens = Some(tokens);
            }
            if let Some(started) = update.started_at {
                step.started_at = Some(started);
            }
            if let Some(completed) = update.completed_at {
                step.completed_at = Some(completed);
            }

            step.updated_at = now;
            Ok(())
        })
    }

    fn list_steps(&self, run_id: Uuid) -> StoreFuture<'_, Vec<Step>> {
        Box::pin(async move {
            let state = self.state.read().await;
            let mut steps: Vec<Step> = state
                .steps
                .values()
                .filter(|s| s.run_id == run_id)
                .cloned()
                .collect();
            steps.sort_by_key(|s| s.position);
            Ok(steps)
        })
    }

    fn get_stats(&self) -> StoreFuture<'_, RunStats> {
        Box::pin(async move {
            let state = self.state.read().await;

            let mut total_cost_usd = Decimal::ZERO;
            let mut total_duration_ms = 0u64;
            let mut completed_runs = 0u64;
            let mut failed_runs = 0u64;
            let mut cancelled_runs = 0u64;
            let mut active_runs = 0u64;

            for run in state.runs.values() {
                total_cost_usd += run.cost_usd;
                total_duration_ms += run.duration_ms;

                match run.status.state {
                    RunStatus::Completed => completed_runs += 1,
                    RunStatus::Failed => failed_runs += 1,
                    RunStatus::Cancelled => cancelled_runs += 1,
                    RunStatus::Pending
                    | RunStatus::Running
                    | RunStatus::Retrying
                    | RunStatus::AwaitingApproval => {
                        active_runs += 1;
                    }
                }
            }

            let total_runs = state.runs.len() as u64;

            Ok(RunStats {
                total_runs,
                completed_runs,
                failed_runs,
                cancelled_runs,
                active_runs,
                total_cost_usd,
                total_duration_ms,
            })
        })
    }

    fn create_step_dependencies(&self, deps: Vec<NewStepDependency>) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut state = self.state.write().await;

            for dep in deps {
                if !state.steps.contains_key(&dep.step_id) {
                    return Err(StoreError::StepNotFound(dep.step_id));
                }
                if !state.steps.contains_key(&dep.depends_on) {
                    return Err(StoreError::StepNotFound(dep.depends_on));
                }

                let already_exists = state
                    .step_dependencies
                    .iter()
                    .any(|d| d.step_id == dep.step_id && d.depends_on == dep.depends_on);

                if !already_exists {
                    state.step_dependencies.push(StepDependency {
                        step_id: dep.step_id,
                        depends_on: dep.depends_on,
                        created_at: Utc::now(),
                    });
                }
            }

            Ok(())
        })
    }

    fn list_step_dependencies(&self, run_id: Uuid) -> StoreFuture<'_, Vec<StepDependency>> {
        Box::pin(async move {
            let state = self.state.read().await;

            let run_step_ids: std::collections::HashSet<Uuid> = state
                .steps
                .values()
                .filter(|s| s.run_id == run_id)
                .map(|s| s.id)
                .collect();

            let mut deps: Vec<StepDependency> = state
                .step_dependencies
                .iter()
                .filter(|d| run_step_ids.contains(&d.step_id))
                .cloned()
                .collect();

            deps.sort_by_key(|d| d.created_at);
            Ok(deps)
        })
    }
}

impl UserStore for InMemoryStore {
    fn create_user(&self, req: NewUser) -> StoreFuture<'_, User> {
        Box::pin(async move {
            let mut state = self.state.write().await;

            let email_exists = state.users.values().any(|u| u.email == req.email);
            if email_exists {
                return Err(StoreError::DuplicateEmail(req.email));
            }

            let username_exists = state.users.values().any(|u| u.username == req.username);
            if username_exists {
                return Err(StoreError::DuplicateUsername(req.username));
            }

            let now = Utc::now();
            let user = User {
                id: Uuid::now_v7(),
                email: req.email,
                username: req.username,
                password_hash: req.password_hash,
                is_admin: false,
                created_at: now,
                updated_at: now,
            };

            state.users.insert(user.id, user.clone());
            Ok(user)
        })
    }

    fn find_user_by_email(&self, email: &str) -> StoreFuture<'_, Option<User>> {
        let email = email.to_string();
        Box::pin(async move {
            let state = self.state.read().await;
            Ok(state.users.values().find(|u| u.email == email).cloned())
        })
    }

    fn find_user_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<User>> {
        Box::pin(async move {
            let state = self.state.read().await;
            Ok(state.users.get(&id).cloned())
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::spawn;

    use super::*;
    use crate::entities::TriggerKind;

    fn new_run_req(name: &str) -> NewRun {
        NewRun {
            workflow_name: name.to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 3,
        }
    }

    // ---- create_run ----

    #[tokio::test]
    async fn create_run_returns_pending_status() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();
        assert_eq!(run.status.state, RunStatus::Pending);
        assert_eq!(run.workflow_name, "test");
        assert_eq!(run.retry_count, 0);
        assert_eq!(run.max_retries, 3);
    }

    #[tokio::test]
    async fn create_run_generates_unique_ids() {
        let store = InMemoryStore::new();
        let r1 = store.create_run(new_run_req("a")).await.unwrap();
        let r2 = store.create_run(new_run_req("b")).await.unwrap();
        assert_ne!(r1.id, r2.id);
    }

    // ---- get_run ----

    #[tokio::test]
    async fn get_run_returns_created_run() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();
        let fetched = store.get_run(run.id).await.unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().id, run.id);
    }

    #[tokio::test]
    async fn get_run_returns_none_for_missing() {
        let store = InMemoryStore::new();
        let fetched = store.get_run(Uuid::nil()).await.unwrap();
        assert!(fetched.is_none());
    }

    // ---- update_run_status ----

    #[tokio::test]
    async fn update_run_status_valid_transition() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();

        let fetched = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(fetched.status.state, RunStatus::Running);
        assert!(fetched.started_at.is_some());
    }

    #[tokio::test]
    async fn update_run_status_invalid_transition_returns_error() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();

        let result = store.update_run_status(run.id, RunStatus::Completed).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(matches!(err, StoreError::InvalidTransition { .. }));
    }

    #[tokio::test]
    async fn update_run_status_not_found() {
        let store = InMemoryStore::new();
        let result = store
            .update_run_status(Uuid::nil(), RunStatus::Running)
            .await;
        assert!(matches!(result.unwrap_err(), StoreError::RunNotFound(_)));
    }

    #[tokio::test]
    async fn update_run_status_terminal_sets_completed_at() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::Completed)
            .await
            .unwrap();

        let fetched = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(fetched.status.state, RunStatus::Completed);
        assert!(fetched.completed_at.is_some());
    }

    // ---- list_runs ----

    #[tokio::test]
    async fn list_runs_empty_store() {
        let store = InMemoryStore::new();
        let page = store.list_runs(RunFilter::default(), 1, 20).await.unwrap();
        assert_eq!(page.total, 0);
        assert!(page.items.is_empty());
    }

    #[tokio::test]
    async fn list_runs_with_workflow_filter() {
        let store = InMemoryStore::new();
        store.create_run(new_run_req("deploy")).await.unwrap();
        store.create_run(new_run_req("test")).await.unwrap();
        store.create_run(new_run_req("deploy")).await.unwrap();

        let filter = RunFilter {
            workflow_name: Some("deploy".to_string()),
            ..RunFilter::default()
        };
        let page = store.list_runs(filter, 1, 20).await.unwrap();
        assert_eq!(page.total, 2);
        assert!(page.items.iter().all(|r| r.workflow_name == "deploy"));
    }

    #[tokio::test]
    async fn list_runs_with_status_filter() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("a")).await.unwrap();
        store.create_run(new_run_req("b")).await.unwrap();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();

        let filter = RunFilter {
            status: Some(RunStatus::Running),
            ..RunFilter::default()
        };
        let page = store.list_runs(filter, 1, 20).await.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].id, run.id);
    }

    #[tokio::test]
    async fn list_runs_pagination() {
        let store = InMemoryStore::new();
        for i in 0..5 {
            store
                .create_run(new_run_req(&format!("wf-{i}")))
                .await
                .unwrap();
        }

        let page1 = store.list_runs(RunFilter::default(), 1, 2).await.unwrap();
        assert_eq!(page1.total, 5);
        assert_eq!(page1.items.len(), 2);
        assert_eq!(page1.page, 1);
        assert_eq!(page1.per_page, 2);

        let page2 = store.list_runs(RunFilter::default(), 2, 2).await.unwrap();
        assert_eq!(page2.items.len(), 2);

        let page3 = store.list_runs(RunFilter::default(), 3, 2).await.unwrap();
        assert_eq!(page3.items.len(), 1);
    }

    // ---- pick_next_pending ----

    #[tokio::test]
    async fn pick_next_pending_empty_store() {
        let store = InMemoryStore::new();
        let result = store.pick_next_pending().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn pick_next_pending_returns_oldest_and_transitions_to_running() {
        let store = InMemoryStore::new();
        let r1 = store.create_run(new_run_req("first")).await.unwrap();
        let _r2 = store.create_run(new_run_req("second")).await.unwrap();

        let picked = store.pick_next_pending().await.unwrap().unwrap();
        assert_eq!(picked.id, r1.id);
        assert_eq!(picked.status.state, RunStatus::Running);
        assert!(picked.started_at.is_some());

        // Verify it's Running in the store too.
        let fetched = store.get_run(r1.id).await.unwrap().unwrap();
        assert_eq!(fetched.status.state, RunStatus::Running);
    }

    #[tokio::test]
    async fn pick_next_pending_skips_non_pending() {
        let store = InMemoryStore::new();
        let r1 = store.create_run(new_run_req("a")).await.unwrap();
        let r2 = store.create_run(new_run_req("b")).await.unwrap();

        // Transition r1 to Running.
        store
            .update_run_status(r1.id, RunStatus::Running)
            .await
            .unwrap();

        let picked = store.pick_next_pending().await.unwrap().unwrap();
        assert_eq!(picked.id, r2.id);
    }

    // ---- create_step ----

    #[tokio::test]
    async fn create_step_returns_pending() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();

        let step = store
            .create_step(NewStep {
                run_id: run.id,
                name: "build".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: Some(json!({"command": "cargo build"})),
            })
            .await
            .unwrap();

        assert_eq!(step.status.state, StepStatus::Pending);
        assert_eq!(step.name, "build");
        assert_eq!(step.run_id, run.id);
        assert_eq!(step.position, 0);
    }

    #[tokio::test]
    async fn create_step_for_missing_run_returns_error() {
        let store = InMemoryStore::new();
        let result = store
            .create_step(NewStep {
                run_id: Uuid::nil(),
                name: "build".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await;
        assert!(matches!(result.unwrap_err(), StoreError::RunNotFound(_)));
    }

    // ---- update_step ----

    #[tokio::test]
    async fn update_step_applies_partial_update() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();

        let step = store
            .create_step(NewStep {
                run_id: run.id,
                name: "build".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        // Transition Pending → Running first
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    ..StepUpdate::default()
                },
            )
            .await
            .unwrap();

        // Then Running → Completed
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Completed),
                    output: Some(json!({"stdout": "ok"})),
                    duration_ms: Some(150),
                    ..StepUpdate::default()
                },
            )
            .await
            .unwrap();

        let steps = store.list_steps(run.id).await.unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].status.state, StepStatus::Completed);
        assert_eq!(steps[0].duration_ms, 150);
        assert!(steps[0].output.is_some());
    }

    #[tokio::test]
    async fn update_step_not_found() {
        let store = InMemoryStore::new();
        let result = store.update_step(Uuid::nil(), StepUpdate::default()).await;
        assert!(matches!(result.unwrap_err(), StoreError::StepNotFound(_)));
    }

    // ---- list_steps ----

    #[tokio::test]
    async fn list_steps_ordered_by_position() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();

        // Insert out of order.
        store
            .create_step(NewStep {
                run_id: run.id,
                name: "deploy".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 2,
                input: None,
            })
            .await
            .unwrap();
        store
            .create_step(NewStep {
                run_id: run.id,
                name: "build".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();
        store
            .create_step(NewStep {
                run_id: run.id,
                name: "test".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 1,
                input: None,
            })
            .await
            .unwrap();

        let steps = store.list_steps(run.id).await.unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].name, "build");
        assert_eq!(steps[1].name, "test");
        assert_eq!(steps[2].name, "deploy");
    }

    #[tokio::test]
    async fn list_steps_empty_for_run_without_steps() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();
        let steps = store.list_steps(run.id).await.unwrap();
        assert!(steps.is_empty());
    }

    // ---- update_run ----

    #[tokio::test]
    async fn update_run_applies_cost_and_duration() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();

        store
            .update_run(
                run.id,
                RunUpdate {
                    cost_usd: Some(Decimal::new(123, 2)),
                    duration_ms: Some(5000),
                    ..RunUpdate::default()
                },
            )
            .await
            .unwrap();

        let fetched = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(fetched.cost_usd, Decimal::new(123, 2));
        assert_eq!(fetched.duration_ms, 5000);
    }

    #[tokio::test]
    async fn update_run_increment_retry() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();
        assert_eq!(run.retry_count, 0);

        store
            .update_run(
                run.id,
                RunUpdate {
                    increment_retry: true,
                    ..RunUpdate::default()
                },
            )
            .await
            .unwrap();

        let fetched = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(fetched.retry_count, 1);
    }

    #[tokio::test]
    async fn update_run_not_found() {
        let store = InMemoryStore::new();
        let result = store.update_run(Uuid::nil(), RunUpdate::default()).await;
        assert!(matches!(result.unwrap_err(), StoreError::RunNotFound(_)));
    }

    // ---- concurrent access ----

    #[tokio::test]
    async fn concurrent_pick_next_pending_no_double_pick() {
        let store = InMemoryStore::new();

        // Create 10 pending runs.
        for i in 0..10 {
            store
                .create_run(new_run_req(&format!("wf-{i}")))
                .await
                .unwrap();
        }

        // Concurrently pick from multiple tasks.
        let mut handles = Vec::new();
        for _ in 0..10 {
            let s = store.clone();
            handles.push(spawn(async move { s.pick_next_pending().await }));
        }

        let mut picked_ids = Vec::new();
        for h in handles {
            if let Ok(Ok(Some(run))) = h.await {
                picked_ids.push(run.id);
            }
        }

        // Each run should be picked at most once.
        let unique: std::collections::HashSet<_> = picked_ids.iter().collect();
        assert_eq!(unique.len(), picked_ids.len());
    }

    // ---- get_stats ----

    #[tokio::test]
    async fn get_stats_empty_store() {
        let store = InMemoryStore::new();
        let stats = store.get_stats().await.unwrap();
        assert_eq!(stats.total_runs, 0);
        assert_eq!(stats.completed_runs, 0);
        assert_eq!(stats.failed_runs, 0);
        assert_eq!(stats.cancelled_runs, 0);
        assert_eq!(stats.active_runs, 0);
        assert_eq!(stats.total_cost_usd, Decimal::ZERO);
        assert_eq!(stats.total_duration_ms, 0);
    }

    #[tokio::test]
    async fn get_stats_aggregates_counts_and_totals() {
        let store = InMemoryStore::new();

        // Create runs in various states.
        let r1 = store.create_run(new_run_req("wf1")).await.unwrap();
        let r2 = store.create_run(new_run_req("wf2")).await.unwrap();
        let r3 = store.create_run(new_run_req("wf3")).await.unwrap();
        let _r4 = store.create_run(new_run_req("wf4")).await.unwrap();

        // Transition r1 to Running, then Completed.
        store
            .update_run_status(r1.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(r1.id, RunStatus::Completed)
            .await
            .unwrap();

        // Transition r2 to Running, then Failed.
        store
            .update_run_status(r2.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(r2.id, RunStatus::Failed)
            .await
            .unwrap();

        // Transition r3 to Cancelled.
        store
            .update_run_status(r3.id, RunStatus::Cancelled)
            .await
            .unwrap();

        // r4 remains Pending (active).

        // Update cost and duration.
        store
            .update_run(
                r1.id,
                RunUpdate {
                    cost_usd: Some(Decimal::new(1000, 2)),
                    duration_ms: Some(1000),
                    ..RunUpdate::default()
                },
            )
            .await
            .unwrap();

        store
            .update_run(
                r2.id,
                RunUpdate {
                    cost_usd: Some(Decimal::new(500, 2)),
                    duration_ms: Some(500),
                    ..RunUpdate::default()
                },
            )
            .await
            .unwrap();

        let stats = store.get_stats().await.unwrap();
        assert_eq!(stats.total_runs, 4);
        assert_eq!(stats.completed_runs, 1);
        assert_eq!(stats.failed_runs, 1);
        assert_eq!(stats.cancelled_runs, 1);
        assert_eq!(stats.active_runs, 1); // r4 is Pending
        assert_eq!(stats.total_cost_usd, Decimal::new(1500, 2));
        assert_eq!(stats.total_duration_ms, 1500);
    }

    // ── UserStore ───────────────────────────────────────────────────

    fn new_user(email: &str, username: &str) -> NewUser {
        NewUser {
            email: email.to_string(),
            username: username.to_string(),
            password_hash: "argon2hash".to_string(),
        }
    }

    #[tokio::test]
    async fn create_user_returns_user() {
        let store = InMemoryStore::new();
        let user = store
            .create_user(new_user("alice@example.com", "alice"))
            .await
            .unwrap();

        assert_eq!(user.email, "alice@example.com");
        assert_eq!(user.username, "alice");
        assert_eq!(user.password_hash, "argon2hash");
        assert!(!user.is_admin);
    }

    #[tokio::test]
    async fn create_user_duplicate_email_returns_error() {
        let store = InMemoryStore::new();
        store
            .create_user(new_user("alice@example.com", "alice"))
            .await
            .unwrap();

        let err = store
            .create_user(new_user("alice@example.com", "bob"))
            .await
            .unwrap_err();

        assert!(
            matches!(err, StoreError::DuplicateEmail(ref e) if e == "alice@example.com"),
            "expected DuplicateEmail, got: {err}"
        );
    }

    #[tokio::test]
    async fn create_user_duplicate_username_returns_error() {
        let store = InMemoryStore::new();
        store
            .create_user(new_user("alice@example.com", "alice"))
            .await
            .unwrap();

        let err = store
            .create_user(new_user("bob@example.com", "alice"))
            .await
            .unwrap_err();

        assert!(
            matches!(err, StoreError::DuplicateUsername(ref u) if u == "alice"),
            "expected DuplicateUsername, got: {err}"
        );
    }

    #[tokio::test]
    async fn find_user_by_email_existing() {
        let store = InMemoryStore::new();
        let created = store
            .create_user(new_user("alice@example.com", "alice"))
            .await
            .unwrap();

        let found = store
            .find_user_by_email("alice@example.com")
            .await
            .unwrap()
            .expect("user should exist");

        assert_eq!(found.id, created.id);
        assert_eq!(found.email, "alice@example.com");
    }

    #[tokio::test]
    async fn find_user_by_email_missing_returns_none() {
        let store = InMemoryStore::new();
        let found = store
            .find_user_by_email("nobody@example.com")
            .await
            .unwrap();

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn find_user_by_id_existing() {
        let store = InMemoryStore::new();
        let created = store
            .create_user(new_user("alice@example.com", "alice"))
            .await
            .unwrap();

        let found = store
            .find_user_by_id(created.id)
            .await
            .unwrap()
            .expect("user should exist");

        assert_eq!(found.email, "alice@example.com");
        assert_eq!(found.username, "alice");
    }

    #[tokio::test]
    async fn find_user_by_id_missing_returns_none() {
        let store = InMemoryStore::new();
        let found = store.find_user_by_id(Uuid::now_v7()).await.unwrap();
        assert!(found.is_none());
    }

    // ─── Additional Edge Cases ──────────────────────────────────

    #[tokio::test]
    async fn update_run_status_running_to_retrying() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();

        store
            .update_run_status(run.id, RunStatus::Retrying)
            .await
            .unwrap();

        let fetched = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(fetched.status.state, RunStatus::Retrying);
        assert!(!fetched.status.state.is_terminal());
        assert!(fetched.completed_at.is_none()); // Not a terminal state
    }

    #[tokio::test]
    async fn update_run_status_retrying_to_running_allowed() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::Retrying)
            .await
            .unwrap();

        // Retrying → Running should be allowed
        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();

        let fetched = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(fetched.status.state, RunStatus::Running);
    }

    #[tokio::test]
    async fn update_run_with_invalid_status_transition_errors() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();

        // Try to apply invalid status transition via update_run
        let result = store
            .update_run(
                run.id,
                RunUpdate {
                    status: Some(RunStatus::Completed), // invalid from Pending
                    ..RunUpdate::default()
                },
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn create_step_with_complex_input() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();

        let complex_input = json!({
            "command": "cargo build",
            "env": {
                "RUST_LOG": "debug",
                "CUSTOM": "value"
            },
            "timeout": 60,
            "retry_policy": {
                "max_attempts": 3,
                "backoff": "exponential"
            }
        });

        let step = store
            .create_step(NewStep {
                run_id: run.id,
                name: "build".to_string(),
                kind: crate::entities::StepKind::Agent,
                position: 0,
                input: Some(complex_input.clone()),
            })
            .await
            .unwrap();

        assert_eq!(step.input, Some(complex_input));
    }

    #[tokio::test]
    async fn update_step_with_error_message() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();

        let step = store
            .create_step(NewStep {
                run_id: run.id,
                name: "build".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    ..StepUpdate::default()
                },
            )
            .await
            .unwrap();

        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Failed),
                    error: Some("Connection timeout after 30s".to_string()),
                    duration_ms: Some(30000),
                    ..StepUpdate::default()
                },
            )
            .await
            .unwrap();

        let steps = store.list_steps(run.id).await.unwrap();
        assert_eq!(steps[0].status.state, StepStatus::Failed);
        assert_eq!(
            steps[0].error,
            Some("Connection timeout after 30s".to_string())
        );
        assert_eq!(steps[0].duration_ms, 30000);
    }

    #[tokio::test]
    async fn list_steps_for_nonexistent_run_returns_empty() {
        let store = InMemoryStore::new();
        let steps = store.list_steps(Uuid::nil()).await.unwrap();
        assert!(steps.is_empty());
    }

    #[tokio::test]
    async fn update_step_pending_to_skipped() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();

        let step = store
            .create_step(NewStep {
                run_id: run.id,
                name: "build".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        // Pending → Skipped is allowed (when prior step failed)
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Skipped),
                    ..StepUpdate::default()
                },
            )
            .await
            .unwrap();

        let steps = store.list_steps(run.id).await.unwrap();
        assert_eq!(steps[0].status.state, StepStatus::Skipped);
    }

    #[tokio::test]
    async fn list_runs_with_combined_filters() {
        let store = InMemoryStore::new();

        let r1 = store.create_run(new_run_req("deploy")).await.unwrap();
        let r2 = store.create_run(new_run_req("deploy")).await.unwrap();
        let _r3 = store.create_run(new_run_req("test")).await.unwrap();

        // r1: Pending → Running → Completed
        store
            .update_run_status(r1.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(r1.id, RunStatus::Completed)
            .await
            .unwrap();

        // r2: Pending → Running
        store
            .update_run_status(r2.id, RunStatus::Running)
            .await
            .unwrap();

        // Filter by workflow AND status
        let filter = RunFilter {
            workflow_name: Some("deploy".to_string()),
            status: Some(RunStatus::Running),
            ..RunFilter::default()
        };

        let page = store.list_runs(filter, 1, 100).await.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].id, r2.id);
    }

    #[tokio::test]
    async fn list_runs_workflow_filter_is_case_insensitive_partial_match() {
        let store = InMemoryStore::new();
        store
            .create_run(new_run_req("weather-report"))
            .await
            .unwrap();
        store.create_run(new_run_req("deploy-prod")).await.unwrap();

        // Partial match
        let filter = RunFilter {
            workflow_name: Some("weather".to_string()),
            ..RunFilter::default()
        };
        let page = store.list_runs(filter, 1, 100).await.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].workflow_name, "weather-report");

        // Case-insensitive
        let filter = RunFilter {
            workflow_name: Some("Weather-REPORT".to_string()),
            ..RunFilter::default()
        };
        let page = store.list_runs(filter, 1, 100).await.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].workflow_name, "weather-report");

        // Suffix match
        let filter = RunFilter {
            workflow_name: Some("report".to_string()),
            ..RunFilter::default()
        };
        let page = store.list_runs(filter, 1, 100).await.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].workflow_name, "weather-report");

        // No match
        let filter = RunFilter {
            workflow_name: Some("build".to_string()),
            ..RunFilter::default()
        };
        let page = store.list_runs(filter, 1, 100).await.unwrap();
        assert_eq!(page.total, 0);
    }

    #[tokio::test]
    async fn get_stats_with_mixed_active_statuses() {
        let store = InMemoryStore::new();

        let _r1 = store.create_run(new_run_req("wf")).await.unwrap(); // Pending
        let r2 = store.create_run(new_run_req("wf")).await.unwrap();
        let r3 = store.create_run(new_run_req("wf")).await.unwrap();

        store
            .update_run_status(r2.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(r3.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(r3.id, RunStatus::Retrying)
            .await
            .unwrap();

        let stats = store.get_stats().await.unwrap();
        assert_eq!(stats.active_runs, 3); // _r1 (Pending), r2 (Running), r3 (Retrying)
    }

    #[tokio::test]
    async fn run_with_different_trigger_kinds() {
        let store = InMemoryStore::new();

        let r1 = store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 1,
            })
            .await
            .unwrap();

        let r2 = store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Webhook {
                    path: "/hooks/github".to_string(),
                },
                payload: json!({}),
                max_retries: 1,
            })
            .await
            .unwrap();

        let r3 = store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Cron {
                    schedule: "0 0 * * *".to_string(),
                },
                payload: json!({}),
                max_retries: 1,
            })
            .await
            .unwrap();

        let r4 = store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Api,
                payload: json!({}),
                max_retries: 1,
            })
            .await
            .unwrap();

        let r5 = store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Retry {
                    parent_run_id: Uuid::nil(),
                },
                payload: json!({}),
                max_retries: 1,
            })
            .await
            .unwrap();

        assert_eq!(r1.trigger, TriggerKind::Manual);
        assert!(matches!(r2.trigger, TriggerKind::Webhook { .. }));
        assert!(matches!(r3.trigger, TriggerKind::Cron { .. }));
        assert_eq!(r4.trigger, TriggerKind::Api);
        assert!(matches!(r5.trigger, TriggerKind::Retry { .. }));
    }

    // ---- update_run_returning ----

    #[tokio::test]
    async fn update_run_returning_applies_and_returns() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();

        // Transition Pending -> Running first
        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();

        let updated = store
            .update_run_returning(
                run.id,
                RunUpdate {
                    status: Some(RunStatus::Completed),
                    cost_usd: Some(Decimal::new(4200, 2)),
                    duration_ms: Some(1500),
                    ..RunUpdate::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.id, run.id);
        assert_eq!(updated.status.state, RunStatus::Completed);
        assert_eq!(updated.cost_usd, Decimal::new(4200, 2));
        assert_eq!(updated.duration_ms, 1500);
        assert!(updated.completed_at.is_some());
    }

    #[tokio::test]
    async fn update_run_returning_not_found() {
        let store = InMemoryStore::new();
        let result = store
            .update_run_returning(
                Uuid::nil(),
                RunUpdate {
                    status: Some(RunStatus::Running),
                    ..RunUpdate::default()
                },
            )
            .await;

        assert!(matches!(result, Err(StoreError::RunNotFound(_))));
    }

    #[tokio::test]
    async fn update_run_returning_invalid_transition() {
        let store = InMemoryStore::new();
        let run = store.create_run(new_run_req("test")).await.unwrap();

        let result = store
            .update_run_returning(
                run.id,
                RunUpdate {
                    status: Some(RunStatus::Completed),
                    ..RunUpdate::default()
                },
            )
            .await;

        assert!(matches!(result, Err(StoreError::InvalidTransition { .. })));
    }
}
