use std::collections::HashMap;

use chrono::Utc;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::entities::{
    ApiKey, IDEMPOTENCY_WINDOW, NewRun, NewStep, NewStepDependency, Page, Run, RunActor,
    RunCreation, RunFilter, RunStats, RunStatus, RunUpdate, Step, StepDependency, StepStatus,
    StepUpdate, User,
};
use crate::error::StoreError;
use crate::store::{RunStore, StoreFuture};

use super::{InMemoryStore, State};

/// Resolve [`Run::created_by_label`] from the current users and API keys.
///
/// Mirrors the `LEFT JOIN` the PostgreSQL store performs at read time, so a
/// renamed API key or a renamed user is reflected immediately.
fn resolve_created_by_label(
    actor: Option<&RunActor>,
    users: &HashMap<Uuid, User>,
    api_keys: &HashMap<Uuid, ApiKey>,
) -> Option<String> {
    let actor = actor?;
    let username = users.get(&actor.user_id()).map(|u| u.username.clone());

    match actor.api_key_id() {
        None => username,
        Some(api_key_id) => {
            let key_name = api_keys.get(&api_key_id).map(|k| k.name.clone())?;
            Some(match username {
                Some(username) => format!("{key_name} ({username})"),
                None => key_name,
            })
        }
    }
}

/// Return a clone of `run` with its display label resolved against `state`.
fn run_with_label(run: &Run, state: &State) -> Run {
    let mut run = run.clone();
    run.created_by_label =
        resolve_created_by_label(run.created_by.as_ref(), &state.users, &state.api_keys);
    run
}

fn run_matches_filter(run: &Run, filter: &RunFilter, steps: &HashMap<Uuid, Step>) -> bool {
    if let Some(ref wf) = filter.workflow_name
        && !run
            .workflow_name
            .to_lowercase()
            .contains(&wf.to_lowercase())
    {
        return false;
    }
    if let Some(ref status) = filter.status
        && &run.status.state != status
    {
        return false;
    }
    if let Some(after) = filter.created_after
        && run.created_at < after
    {
        return false;
    }
    if let Some(before) = filter.created_before
        && run.created_at > before
    {
        return false;
    }
    if let Some(has_steps) = filter.has_steps
        && matches!(
            run.status.state,
            RunStatus::Completed | RunStatus::Cancelled
        )
    {
        let run_has_steps = steps.values().any(|s| s.run_id == run.id);
        if has_steps != run_has_steps {
            return false;
        }
    }
    if let Some(ref labels) = filter.labels {
        for (key, value) in labels {
            if run.labels.get(key) != Some(value) {
                return false;
            }
        }
    }
    if let Some(user_id) = filter.created_by_user_id
        && run.created_by.as_ref().map(RunActor::user_id) != Some(user_id)
    {
        return false;
    }
    true
}

impl RunStore for InMemoryStore {
    fn create_run(&self, req: NewRun) -> StoreFuture<'_, RunCreation> {
        Box::pin(async move {
            let now = Utc::now();

            // Single critical section: the key lookup and the insert cannot be
            // interleaved by a concurrent call sharing the same key.
            let mut state = self.state.write().await;

            if let Some(ref key) = req.idempotency_key
                && let Some(existing) = state
                    .idempotency_keys
                    .get(key)
                    .and_then(|id| state.runs.get(id))
            {
                if now - existing.created_at < IDEMPOTENCY_WINDOW {
                    return Ok(RunCreation::Existing(run_with_label(existing, &state)));
                }
                // The key outlived its window: release it from the stale run.
                let stale_id = existing.id;
                state.idempotency_keys.remove(key);
                if let Some(stale) = state.runs.get_mut(&stale_id) {
                    stale.idempotency_key = None;
                }
            }

            let run = Run {
                id: Uuid::now_v7(),
                workflow_name: req.workflow_name,
                status: crate::entities::FsmState::new(RunStatus::Pending, Uuid::now_v7()),
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
                handler_version: req.handler_version,
                labels: req.labels,
                scheduled_at: req.scheduled_at,
                created_by: req.created_by,
                created_by_label: None,
                idempotency_key: req.idempotency_key.clone(),
                max_cost_usd: req.max_cost_usd,
            };

            if let Some(key) = req.idempotency_key {
                state.idempotency_keys.insert(key, run.id);
            }
            state.runs.insert(run.id, run.clone());
            Ok(RunCreation::Created(run_with_label(&run, &state)))
        })
    }

    fn find_run_by_idempotency_key(&self, key: &str) -> StoreFuture<'_, Option<Run>> {
        let key = key.to_string();
        Box::pin(async move {
            let now = Utc::now();
            let state = self.state.read().await;
            Ok(state
                .idempotency_keys
                .get(&key)
                .and_then(|id| state.runs.get(id))
                .filter(|run| now - run.created_at < IDEMPOTENCY_WINDOW)
                .map(|run| run_with_label(run, &state)))
        })
    }

    fn get_run(&self, id: Uuid) -> StoreFuture<'_, Option<Run>> {
        Box::pin(async move {
            let state = self.state.read().await;
            Ok(state.runs.get(&id).map(|r| run_with_label(r, &state)))
        })
    }

    fn list_runs(&self, filter: RunFilter, page: u32, per_page: u32) -> StoreFuture<'_, Page<Run>> {
        Box::pin(async move {
            let state = self.state.read().await;

            let mut runs: Vec<&Run> = state
                .runs
                .values()
                .filter(|r| run_matches_filter(r, &filter, &state.steps))
                .collect();

            // Sort newest first.
            runs.sort_by_key(|r| std::cmp::Reverse(r.created_at));

            let total = runs.len() as u64;
            let page = page.max(1);
            let per_page = per_page.clamp(1, 100);
            let offset = ((page - 1) * per_page) as usize;
            let items: Vec<Run> = runs
                .into_iter()
                .skip(offset)
                .take(per_page as usize)
                .map(|r| run_with_label(r, &state))
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

            if run.status.state == new_status && new_status.is_terminal() {
                return Ok(());
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
                if !(run.status.state == status && status.is_terminal()) {
                    run.status.state = status;
                    if status == RunStatus::Running && run.started_at.is_none() {
                        run.started_at = Some(now);
                    }
                    if status.is_terminal() {
                        run.completed_at = Some(now);
                    }
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
            if let Some(scheduled) = update.scheduled_at {
                run.scheduled_at = Some(scheduled);
            }

            run.updated_at = now;
            Ok(())
        })
    }

    fn pick_next_pending(&self) -> StoreFuture<'_, Option<Run>> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            let now = Utc::now();

            // Find the oldest run waiting for execution whose scheduled_at has
            // passed (or is None). `Retrying` runs are runs whose automatic retry
            // backoff has been armed: they become eligible again once
            // `scheduled_at` has passed.
            let oldest_id = state
                .runs
                .values()
                .filter(|r| {
                    matches!(r.status.state, RunStatus::Pending | RunStatus::Retrying)
                        && r.scheduled_at.is_none_or(|at| at <= now)
                })
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
            let run = run.clone();

            Ok(Some(run_with_label(&run, &state)))
        })
    }

    fn create_step(&self, req: NewStep) -> StoreFuture<'_, Step> {
        Box::pin(async move {
            let mut state = self.state.write().await;

            let attempt = state
                .runs
                .get(&req.run_id)
                .ok_or(StoreError::RunNotFound(req.run_id))?
                .retry_count
                + 1;

            let now = Utc::now();
            let step = Step {
                id: Uuid::now_v7(),
                run_id: req.run_id,
                name: req.name,
                kind: req.kind,
                position: req.position,
                status: crate::entities::FsmState::new(StepStatus::Pending, Uuid::now_v7()),
                attempt,
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
                debug_messages: None,
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
            if let Some(debug_msgs) = update.debug_messages {
                step.debug_messages = Some(debug_msgs);
            }

            step.updated_at = now;
            Ok(())
        })
    }

    fn get_step(&self, id: Uuid) -> StoreFuture<'_, Option<Step>> {
        Box::pin(async move {
            let state = self.state.read().await;
            Ok(state.steps.get(&id).cloned())
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

    fn get_stats(&self, filter: RunFilter) -> StoreFuture<'_, RunStats> {
        Box::pin(async move {
            let state = self.state.read().await;

            let mut total_cost_usd = Decimal::ZERO;
            let mut total_duration_ms = 0u64;
            let mut total_runs = 0u64;
            let mut completed_runs = 0u64;
            let mut failed_runs = 0u64;
            let mut cancelled_runs = 0u64;
            let mut active_runs = 0u64;

            for run in state.runs.values() {
                if !run_matches_filter(run, &filter, &state.steps) {
                    continue;
                }

                total_cost_usd += run.cost_usd;
                total_duration_ms += run.duration_ms;
                total_runs += 1;

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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;
    use tokio::spawn;

    use super::*;
    use crate::api_key_store::ApiKeyStore;
    use crate::entities::{ApiKeyScope, ApiKeyUpdate, NewApiKey, NewUser, TriggerKind};
    use crate::user_store::UserStore;

    use crate::memory::tests::{create_terminal_run, new_run_req};
    use crate::store::RunStore;

    use chrono::TimeDelta;

    use crate::entities::StepKind;

    fn new_step_req(run_id: Uuid, name: &str, position: u32) -> NewStep {
        NewStep {
            run_id,
            name: name.to_string(),
            kind: StepKind::Shell,
            position,
            input: None,
        }
    }

    // ---- create_run ----

    #[tokio::test]
    async fn create_run_returns_pending_status() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();
        assert_eq!(run.status.state, RunStatus::Pending);
        assert_eq!(run.workflow_name, "test");
        assert_eq!(run.retry_count, 0);
        assert_eq!(run.max_retries, 3);
    }

    #[tokio::test]
    async fn create_run_generates_unique_ids() {
        let store = InMemoryStore::new();
        let r1 = store.create_run(new_run_req("a")).await.unwrap().into_run();
        let r2 = store.create_run(new_run_req("b")).await.unwrap().into_run();
        assert_ne!(r1.id, r2.id);
    }

    // ---- get_run ----

    #[tokio::test]
    async fn get_run_returns_created_run() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();
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
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

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
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

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
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

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

    #[tokio::test]
    async fn update_run_status_terminal_to_same_is_idempotent() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::Failed)
            .await
            .unwrap();

        let before = store.get_run(run.id).await.unwrap().unwrap();
        let completed_at_before = before.completed_at;

        store
            .update_run_status(run.id, RunStatus::Failed)
            .await
            .unwrap();

        let after = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(after.status.state, RunStatus::Failed);
        assert_eq!(after.completed_at, completed_at_before);
    }

    #[tokio::test]
    async fn update_run_terminal_to_same_via_update_run_is_idempotent() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run(
                run.id,
                RunUpdate {
                    status: Some(RunStatus::Failed),
                    error: Some("first failure".to_string()),
                    ..RunUpdate::default()
                },
            )
            .await
            .unwrap();

        let before = store.get_run(run.id).await.unwrap().unwrap();

        store
            .update_run(
                run.id,
                RunUpdate {
                    status: Some(RunStatus::Failed),
                    ..RunUpdate::default()
                },
            )
            .await
            .unwrap();

        let after = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(after.status.state, RunStatus::Failed);
        assert_eq!(after.completed_at, before.completed_at);
        assert_eq!(after.error, Some("first failure".to_string()));
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
        store
            .create_run(new_run_req("deploy"))
            .await
            .unwrap()
            .into_run();
        store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();
        store
            .create_run(new_run_req("deploy"))
            .await
            .unwrap()
            .into_run();

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
        let run = store.create_run(new_run_req("a")).await.unwrap().into_run();
        store.create_run(new_run_req("b")).await.unwrap().into_run();

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
                .unwrap()
                .into_run();
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
        let r1 = store
            .create_run(new_run_req("first"))
            .await
            .unwrap()
            .into_run();
        let _r2 = store
            .create_run(new_run_req("second"))
            .await
            .unwrap()
            .into_run();

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
        let r1 = store.create_run(new_run_req("a")).await.unwrap().into_run();
        let r2 = store.create_run(new_run_req("b")).await.unwrap().into_run();

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
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

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
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

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
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

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
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();
        let steps = store.list_steps(run.id).await.unwrap();
        assert!(steps.is_empty());
    }

    // ---- update_run ----

    #[tokio::test]
    async fn update_run_applies_cost_and_duration() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

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
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();
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
                .unwrap()
                .into_run();
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
        let stats = store.get_stats(RunFilter::default()).await.unwrap();
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
        let r1 = store
            .create_run(new_run_req("wf1"))
            .await
            .unwrap()
            .into_run();
        let r2 = store
            .create_run(new_run_req("wf2"))
            .await
            .unwrap()
            .into_run();
        let r3 = store
            .create_run(new_run_req("wf3"))
            .await
            .unwrap()
            .into_run();
        let _r4 = store
            .create_run(new_run_req("wf4"))
            .await
            .unwrap()
            .into_run();

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

        let stats = store.get_stats(RunFilter::default()).await.unwrap();
        assert_eq!(stats.total_runs, 4);
        assert_eq!(stats.completed_runs, 1);
        assert_eq!(stats.failed_runs, 1);
        assert_eq!(stats.cancelled_runs, 1);
        assert_eq!(stats.active_runs, 1); // r4 is Pending
        assert_eq!(stats.total_cost_usd, Decimal::new(1500, 2));
        assert_eq!(stats.total_duration_ms, 1500);
    }

    #[tokio::test]
    async fn update_run_status_running_to_retrying() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

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
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

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
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

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
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

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
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

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
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

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

        let r1 = store
            .create_run(new_run_req("deploy"))
            .await
            .unwrap()
            .into_run();
        let r2 = store
            .create_run(new_run_req("deploy"))
            .await
            .unwrap()
            .into_run();
        let _r3 = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

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
            .unwrap()
            .into_run();
        store
            .create_run(new_run_req("deploy-prod"))
            .await
            .unwrap()
            .into_run();

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
    async fn list_runs_has_steps_true_only_filters_completed_and_cancelled() {
        let store = InMemoryStore::new();
        let run_with = create_terminal_run(&store, "with-steps", RunStatus::Completed).await;
        let _run_without = create_terminal_run(&store, "without-steps", RunStatus::Completed).await;

        store
            .create_step(NewStep {
                run_id: run_with.id,
                name: "build".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        let filter = RunFilter {
            has_steps: Some(true),
            ..RunFilter::default()
        };
        let page = store.list_runs(filter, 1, 100).await.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].id, run_with.id);
    }

    #[tokio::test]
    async fn list_runs_has_steps_false_only_filters_completed_and_cancelled() {
        let store = InMemoryStore::new();
        let run_with = create_terminal_run(&store, "with-steps", RunStatus::Cancelled).await;
        let run_without = create_terminal_run(&store, "without-steps", RunStatus::Cancelled).await;

        store
            .create_step(NewStep {
                run_id: run_with.id,
                name: "build".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        let filter = RunFilter {
            has_steps: Some(false),
            ..RunFilter::default()
        };
        let page = store.list_runs(filter, 1, 100).await.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].id, run_without.id);
    }

    #[tokio::test]
    async fn list_runs_has_steps_none_returns_all() {
        let store = InMemoryStore::new();
        let run_with = store
            .create_run(new_run_req("with-steps"))
            .await
            .unwrap()
            .into_run();
        let _run_without = store
            .create_run(new_run_req("without-steps"))
            .await
            .unwrap()
            .into_run();

        store
            .create_step(NewStep {
                run_id: run_with.id,
                name: "build".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        let filter = RunFilter {
            has_steps: None,
            ..RunFilter::default()
        };
        let page = store.list_runs(filter, 1, 100).await.unwrap();
        assert_eq!(page.total, 2);
    }

    #[tokio::test]
    async fn list_runs_has_steps_true_does_not_filter_non_terminal_runs() {
        let store = InMemoryStore::new();
        let pending_run = store
            .create_run(new_run_req("pending-empty"))
            .await
            .unwrap()
            .into_run();
        let running_run = store
            .create_run(new_run_req("running-empty"))
            .await
            .unwrap()
            .into_run();
        store
            .update_run_status(running_run.id, RunStatus::Running)
            .await
            .unwrap();

        let filter = RunFilter {
            has_steps: Some(true),
            ..RunFilter::default()
        };
        let page = store.list_runs(filter, 1, 100).await.unwrap();
        assert_eq!(page.total, 2);
        let ids: Vec<_> = page.items.iter().map(|r| r.id).collect();
        assert!(ids.contains(&pending_run.id));
        assert!(ids.contains(&running_run.id));
    }

    #[tokio::test]
    async fn get_stats_with_mixed_active_statuses() {
        let store = InMemoryStore::new();

        let _r1 = store
            .create_run(new_run_req("wf"))
            .await
            .unwrap()
            .into_run(); // Pending
        let r2 = store
            .create_run(new_run_req("wf"))
            .await
            .unwrap()
            .into_run();
        let r3 = store
            .create_run(new_run_req("wf"))
            .await
            .unwrap()
            .into_run();

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

        let stats = store.get_stats(RunFilter::default()).await.unwrap();
        assert_eq!(stats.active_runs, 3); // _r1 (Pending), r2 (Running), r3 (Retrying)
    }

    #[tokio::test]
    async fn run_with_different_trigger_kinds() {
        let store = InMemoryStore::new();

        let r1 = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 1,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        let r2 = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Webhook {
                    path: "/hooks/github".to_string(),
                },
                payload: json!({}),
                max_retries: 1,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        let r3 = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Cron {
                    schedule: "0 0 * * *".to_string(),
                },
                payload: json!({}),
                max_retries: 1,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        let r4 = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Api,
                payload: json!({}),
                max_retries: 1,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        let r5 = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Retry {
                    parent_run_id: Uuid::nil(),
                },
                payload: json!({}),
                max_retries: 1,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        assert_eq!(r1.trigger, TriggerKind::Manual);
        assert!(matches!(r2.trigger, TriggerKind::Webhook { .. }));
        assert!(matches!(r3.trigger, TriggerKind::Cron { .. }));
        assert_eq!(r4.trigger, TriggerKind::Api);
        assert!(matches!(r5.trigger, TriggerKind::Retry { .. }));
    }

    // ---- create_step_dependencies ----

    #[tokio::test]
    async fn create_step_dependencies_stores_dependencies() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

        let step1 = store
            .create_step(NewStep {
                run_id: run.id,
                name: "step1".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        let step2 = store
            .create_step(NewStep {
                run_id: run.id,
                name: "step2".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 1,
                input: None,
            })
            .await
            .unwrap();

        let result = store
            .create_step_dependencies(vec![NewStepDependency {
                step_id: step2.id,
                depends_on: step1.id,
            }])
            .await;

        assert!(result.is_ok());

        let deps = store.list_step_dependencies(run.id).await.unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].step_id, step2.id);
        assert_eq!(deps[0].depends_on, step1.id);
    }

    #[tokio::test]
    async fn create_step_dependencies_duplicate_dependencies_are_idempotent() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

        let step1 = store
            .create_step(NewStep {
                run_id: run.id,
                name: "step1".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        let step2 = store
            .create_step(NewStep {
                run_id: run.id,
                name: "step2".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 1,
                input: None,
            })
            .await
            .unwrap();

        let dep = NewStepDependency {
            step_id: step2.id,
            depends_on: step1.id,
        };

        store
            .create_step_dependencies(vec![dep.clone()])
            .await
            .unwrap();
        store.create_step_dependencies(vec![dep]).await.unwrap();

        let deps = store.list_step_dependencies(run.id).await.unwrap();
        assert_eq!(deps.len(), 1);
    }

    #[tokio::test]
    async fn create_step_dependencies_missing_step_id_returns_error() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

        let step1 = store
            .create_step(NewStep {
                run_id: run.id,
                name: "step1".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        let result = store
            .create_step_dependencies(vec![NewStepDependency {
                step_id: Uuid::nil(),
                depends_on: step1.id,
            }])
            .await;

        assert!(matches!(result.unwrap_err(), StoreError::StepNotFound(_)));
    }

    #[tokio::test]
    async fn create_step_dependencies_missing_depends_on_returns_error() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

        let step1 = store
            .create_step(NewStep {
                run_id: run.id,
                name: "step1".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        let result = store
            .create_step_dependencies(vec![NewStepDependency {
                step_id: step1.id,
                depends_on: Uuid::nil(),
            }])
            .await;

        assert!(matches!(result.unwrap_err(), StoreError::StepNotFound(_)));
    }

    #[tokio::test]
    async fn create_step_dependencies_multiple_dependencies() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

        let step1 = store
            .create_step(NewStep {
                run_id: run.id,
                name: "step1".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        let step2 = store
            .create_step(NewStep {
                run_id: run.id,
                name: "step2".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 1,
                input: None,
            })
            .await
            .unwrap();

        let step3 = store
            .create_step(NewStep {
                run_id: run.id,
                name: "step3".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 2,
                input: None,
            })
            .await
            .unwrap();

        let result = store
            .create_step_dependencies(vec![
                NewStepDependency {
                    step_id: step2.id,
                    depends_on: step1.id,
                },
                NewStepDependency {
                    step_id: step3.id,
                    depends_on: step2.id,
                },
            ])
            .await;

        assert!(result.is_ok());

        let deps = store.list_step_dependencies(run.id).await.unwrap();
        assert_eq!(deps.len(), 2);
    }

    // ---- list_step_dependencies ----

    #[tokio::test]
    async fn list_step_dependencies_empty_for_run_with_no_dependencies() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

        store
            .create_step(NewStep {
                run_id: run.id,
                name: "step1".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        let deps = store.list_step_dependencies(run.id).await.unwrap();
        assert!(deps.is_empty());
    }

    #[tokio::test]
    async fn list_step_dependencies_returns_only_deps_for_given_run() {
        let store = InMemoryStore::new();
        let run1 = store
            .create_run(new_run_req("test1"))
            .await
            .unwrap()
            .into_run();
        let run2 = store
            .create_run(new_run_req("test2"))
            .await
            .unwrap()
            .into_run();

        let step1_run1 = store
            .create_step(NewStep {
                run_id: run1.id,
                name: "step1".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        let step2_run1 = store
            .create_step(NewStep {
                run_id: run1.id,
                name: "step2".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 1,
                input: None,
            })
            .await
            .unwrap();

        let step1_run2 = store
            .create_step(NewStep {
                run_id: run2.id,
                name: "step1".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        let step2_run2 = store
            .create_step(NewStep {
                run_id: run2.id,
                name: "step2".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 1,
                input: None,
            })
            .await
            .unwrap();

        store
            .create_step_dependencies(vec![
                NewStepDependency {
                    step_id: step2_run1.id,
                    depends_on: step1_run1.id,
                },
                NewStepDependency {
                    step_id: step2_run2.id,
                    depends_on: step1_run2.id,
                },
            ])
            .await
            .unwrap();

        let deps_run1 = store.list_step_dependencies(run1.id).await.unwrap();
        let deps_run2 = store.list_step_dependencies(run2.id).await.unwrap();

        assert_eq!(deps_run1.len(), 1);
        assert_eq!(deps_run1[0].step_id, step2_run1.id);
        assert_eq!(deps_run1[0].depends_on, step1_run1.id);

        assert_eq!(deps_run2.len(), 1);
        assert_eq!(deps_run2[0].step_id, step2_run2.id);
        assert_eq!(deps_run2[0].depends_on, step1_run2.id);
    }

    #[tokio::test]
    async fn list_step_dependencies_returns_empty_for_nonexistent_run() {
        let store = InMemoryStore::new();
        let deps = store.list_step_dependencies(Uuid::nil()).await.unwrap();
        assert!(deps.is_empty());
    }

    #[tokio::test]
    async fn list_step_dependencies_sorted_by_created_at() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

        let step1 = store
            .create_step(NewStep {
                run_id: run.id,
                name: "step1".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        let step2 = store
            .create_step(NewStep {
                run_id: run.id,
                name: "step2".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 1,
                input: None,
            })
            .await
            .unwrap();

        let step3 = store
            .create_step(NewStep {
                run_id: run.id,
                name: "step3".to_string(),
                kind: crate::entities::StepKind::Shell,
                position: 2,
                input: None,
            })
            .await
            .unwrap();

        store
            .create_step_dependencies(vec![NewStepDependency {
                step_id: step2.id,
                depends_on: step1.id,
            }])
            .await
            .unwrap();

        store
            .create_step_dependencies(vec![NewStepDependency {
                step_id: step3.id,
                depends_on: step1.id,
            }])
            .await
            .unwrap();

        let deps = store.list_step_dependencies(run.id).await.unwrap();
        assert_eq!(deps.len(), 2);
        assert!(deps[0].created_at <= deps[1].created_at);
    }

    // ---- update_run_returning ----

    #[tokio::test]
    async fn update_run_returning_applies_and_returns() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

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
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

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

    // ---- retry scheduling ----

    #[tokio::test]
    async fn create_step_stamps_the_current_attempt() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("retry-wf"))
            .await
            .unwrap()
            .into_run();

        let first = store
            .create_step(new_step_req(run.id, "build", 0))
            .await
            .unwrap();
        assert_eq!(first.attempt, 1);

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run(
                run.id,
                RunUpdate {
                    status: Some(RunStatus::Retrying),
                    increment_retry: true,
                    ..RunUpdate::default()
                },
            )
            .await
            .unwrap();

        let second = store
            .create_step(new_step_req(run.id, "build", 0))
            .await
            .unwrap();
        assert_eq!(second.attempt, 2);
    }

    #[tokio::test]
    async fn pick_next_pending_ignores_retrying_run_before_its_backoff() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("retry-wf"))
            .await
            .unwrap()
            .into_run();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run(
                run.id,
                RunUpdate {
                    status: Some(RunStatus::Retrying),
                    increment_retry: true,
                    scheduled_at: Some(Utc::now() + TimeDelta::seconds(60)),
                    ..RunUpdate::default()
                },
            )
            .await
            .unwrap();

        assert!(store.pick_next_pending().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn pick_next_pending_resumes_retrying_run_after_its_backoff() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("retry-wf"))
            .await
            .unwrap()
            .into_run();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run(
                run.id,
                RunUpdate {
                    status: Some(RunStatus::Retrying),
                    increment_retry: true,
                    scheduled_at: Some(Utc::now() - TimeDelta::seconds(1)),
                    ..RunUpdate::default()
                },
            )
            .await
            .unwrap();

        let picked = store.pick_next_pending().await.unwrap().unwrap();
        assert_eq!(picked.id, run.id);
        assert_eq!(picked.status.state, RunStatus::Running);
        assert_eq!(picked.retry_count, 1);
    }

    #[tokio::test]
    async fn update_run_persists_scheduled_at() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();
        let when = Utc::now() + TimeDelta::seconds(30);

        store
            .update_run(
                run.id,
                RunUpdate {
                    scheduled_at: Some(when),
                    ..RunUpdate::default()
                },
            )
            .await
            .unwrap();

        let fetched = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(fetched.scheduled_at, Some(when));
    }

    // ---- created_by ----

    async fn seed_user(store: &InMemoryStore, username: &str) -> Uuid {
        store
            .create_user(NewUser {
                email: format!("{username}@example.com"),
                username: username.to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(false),
            })
            .await
            .unwrap()
            .id
    }

    async fn seed_api_key(store: &InMemoryStore, user_id: Uuid, name: &str) -> Uuid {
        store
            .create_api_key(NewApiKey {
                user_id,
                name: name.to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "irfl_0000".to_string(),
                scopes: vec![ApiKeyScope::RunsWrite],
                expires_at: None,
            })
            .await
            .unwrap()
            .id
    }

    fn run_req_by(actor: RunActor) -> NewRun {
        NewRun {
            created_by: Some(actor),
            ..new_run_req("test")
        }
    }

    #[tokio::test]
    async fn create_run_without_actor_has_no_author() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(new_run_req("test"))
            .await
            .unwrap()
            .into_run();

        assert!(run.created_by.is_none());
        assert!(run.created_by_label.is_none());
    }

    #[tokio::test]
    async fn create_run_by_user_resolves_username_as_label() {
        let store = InMemoryStore::new();
        let user_id = seed_user(&store, "alice").await;

        let run = store
            .create_run(run_req_by(RunActor::User { user_id }))
            .await
            .unwrap()
            .into_run();

        assert_eq!(run.created_by, Some(RunActor::User { user_id }));
        assert_eq!(run.created_by_label.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn create_run_by_api_key_resolves_key_and_owner_as_label() {
        let store = InMemoryStore::new();
        let user_id = seed_user(&store, "alice").await;
        let api_key_id = seed_api_key(&store, user_id, "ci-deploy").await;

        let run = store
            .create_run(run_req_by(RunActor::ApiKey {
                api_key_id,
                user_id,
            }))
            .await
            .unwrap()
            .into_run();

        assert_eq!(run.created_by_label.as_deref(), Some("ci-deploy (alice)"));
    }

    #[tokio::test]
    async fn label_follows_api_key_rename() {
        let store = InMemoryStore::new();
        let user_id = seed_user(&store, "alice").await;
        let api_key_id = seed_api_key(&store, user_id, "ci-deploy").await;
        let run = store
            .create_run(run_req_by(RunActor::ApiKey {
                api_key_id,
                user_id,
            }))
            .await
            .unwrap()
            .into_run();

        store
            .update_api_key(
                api_key_id,
                ApiKeyUpdate {
                    name: Some("ci-release".to_string()),
                    ..ApiKeyUpdate::default()
                },
            )
            .await
            .unwrap();

        let reread = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(
            reread.created_by_label.as_deref(),
            Some("ci-release (alice)")
        );
    }

    #[tokio::test]
    async fn label_is_none_when_user_is_unknown() {
        let store = InMemoryStore::new();
        let run = store
            .create_run(run_req_by(RunActor::User {
                user_id: Uuid::now_v7(),
            }))
            .await
            .unwrap()
            .into_run();

        assert!(run.created_by.is_some());
        assert!(run.created_by_label.is_none());
    }

    #[tokio::test]
    async fn label_is_key_name_only_when_owner_is_unknown() {
        let store = InMemoryStore::new();
        let owner = seed_user(&store, "alice").await;
        let api_key_id = seed_api_key(&store, owner, "ci-deploy").await;

        // An actor pointing at a user that was never created.
        let run = store
            .create_run(run_req_by(RunActor::ApiKey {
                api_key_id,
                user_id: Uuid::now_v7(),
            }))
            .await
            .unwrap()
            .into_run();

        assert_eq!(run.created_by_label.as_deref(), Some("ci-deploy"));
    }

    #[tokio::test]
    async fn list_runs_filters_by_author() {
        let store = InMemoryStore::new();
        let alice = seed_user(&store, "alice").await;
        let bob = seed_user(&store, "bob").await;

        store
            .create_run(run_req_by(RunActor::User { user_id: alice }))
            .await
            .unwrap()
            .into_run();
        store
            .create_run(run_req_by(RunActor::User { user_id: bob }))
            .await
            .unwrap()
            .into_run();
        store.create_run(new_run_req("anonymous")).await.unwrap();

        let page = store
            .list_runs(
                RunFilter {
                    created_by_user_id: Some(alice),
                    ..RunFilter::default()
                },
                1,
                20,
            )
            .await
            .unwrap();

        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].created_by_label.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn list_runs_author_filter_matches_runs_from_the_users_api_keys() {
        let store = InMemoryStore::new();
        let alice = seed_user(&store, "alice").await;
        let api_key_id = seed_api_key(&store, alice, "ci-deploy").await;

        store
            .create_run(run_req_by(RunActor::ApiKey {
                api_key_id,
                user_id: alice,
            }))
            .await
            .unwrap()
            .into_run();

        let page = store
            .list_runs(
                RunFilter {
                    created_by_user_id: Some(alice),
                    ..RunFilter::default()
                },
                1,
                20,
            )
            .await
            .unwrap();

        assert_eq!(page.total, 1);
    }

    #[tokio::test]
    async fn list_runs_author_filter_excludes_unrelated_users() {
        let store = InMemoryStore::new();
        let alice = seed_user(&store, "alice").await;

        store
            .create_run(run_req_by(RunActor::User { user_id: alice }))
            .await
            .unwrap()
            .into_run();

        let page = store
            .list_runs(
                RunFilter {
                    created_by_user_id: Some(Uuid::now_v7()),
                    ..RunFilter::default()
                },
                1,
                20,
            )
            .await
            .unwrap();

        assert_eq!(page.total, 0);
    }

    #[tokio::test]
    async fn list_runs_without_author_filter_returns_every_run() {
        let store = InMemoryStore::new();
        let alice = seed_user(&store, "alice").await;

        store
            .create_run(run_req_by(RunActor::User { user_id: alice }))
            .await
            .unwrap()
            .into_run();
        store.create_run(new_run_req("anonymous")).await.unwrap();

        let page = store.list_runs(RunFilter::default(), 1, 20).await.unwrap();
        assert_eq!(page.total, 2);
    }

    #[tokio::test]
    async fn pick_next_pending_resolves_author_label() {
        let store = InMemoryStore::new();
        let user_id = seed_user(&store, "alice").await;
        store
            .create_run(run_req_by(RunActor::User { user_id }))
            .await
            .unwrap()
            .into_run();

        let picked = store.pick_next_pending().await.unwrap().unwrap();
        assert_eq!(picked.created_by_label.as_deref(), Some("alice"));
    }
}
