//! In-memory [`SignalStore`] implementation.
//!
//! Every method runs under the single write lock of the store, which gives
//! the same atomicity as the PostgreSQL transactions.

use std::cmp::Reverse;

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::entities::{
    NewSignal, Page, RunStatus, Signal, SignalFilter, SignalInsert, SignalStepResolution, Step,
    StepKind, StepStatus,
};
use crate::error::StoreError;
use crate::memory::InMemoryStore;
use crate::signal_store::SignalStore;
use crate::store::StoreFuture;

/// Maximum number of signals returned by [`SignalStore::list_signals_for_key`].
const SIGNALS_FOR_KEY_LIMIT: usize = 100;

/// Whether `step` is a `signal` step still waiting for a delivery.
fn is_waiting_signal_step(step: &Step) -> bool {
    step.kind == StepKind::Signal && step.status.state == StepStatus::Running
}

/// Whether the `name`/`key` stored in a signal step's input match.
fn step_waits_for(step: &Step, name: &str, key: &str) -> bool {
    let Some(input) = step.input.as_ref() else {
        return false;
    };
    input.get("name").and_then(Value::as_str) == Some(name)
        && input.get("key").and_then(Value::as_str) == Some(key)
}

impl SignalStore for InMemoryStore {
    fn insert_signal(&self, signal: NewSignal) -> StoreFuture<'_, SignalInsert> {
        Box::pin(async move {
            let mut state = self.state.write().await;

            if let Some(ref idempotency_id) = signal.idempotency_id
                && let Some(existing) = state
                    .signal_idempotency
                    .get(idempotency_id)
                    .and_then(|id| state.signals.iter().find(|s| s.id == *id))
            {
                return Ok(SignalInsert::Duplicate(existing.clone()));
            }

            let stored = Signal {
                id: Uuid::now_v7(),
                name: signal.name,
                key: signal.key,
                payload: signal.payload,
                idempotency_id: signal.idempotency_id,
                received_at: Utc::now(),
            };
            if let Some(ref idempotency_id) = stored.idempotency_id {
                state
                    .signal_idempotency
                    .insert(idempotency_id.clone(), stored.id);
            }
            state.signals.push(stored.clone());
            Ok(SignalInsert::Created(stored))
        })
    }

    fn list_signals(
        &self,
        filter: SignalFilter,
        page: u32,
        per_page: u32,
    ) -> StoreFuture<'_, Page<Signal>> {
        Box::pin(async move {
            let state = self.state.read().await;
            let mut items: Vec<Signal> = state
                .signals
                .iter()
                .filter(|s| filter.name.as_ref().is_none_or(|name| &s.name == name))
                .filter(|s| filter.key.as_ref().is_none_or(|key| &s.key == key))
                .cloned()
                .collect();
            items.sort_by_key(|s| Reverse((s.received_at, s.id)));

            let total = items.len() as u64;
            let page = page.max(1);
            let per_page = per_page.clamp(1, 100);
            let offset = ((page - 1) * per_page) as usize;
            let items = items
                .into_iter()
                .skip(offset)
                .take(per_page as usize)
                .collect();

            Ok(Page {
                items,
                total,
                page,
                per_page,
            })
        })
    }

    fn list_signals_for_key(
        &self,
        name: &str,
        key: &str,
        since: DateTime<Utc>,
    ) -> StoreFuture<'_, Vec<Signal>> {
        let name = name.to_string();
        let key = key.to_string();
        Box::pin(async move {
            let state = self.state.read().await;
            let mut items: Vec<Signal> = state
                .signals
                .iter()
                .filter(|s| s.name == name && s.key == key && s.received_at >= since)
                .cloned()
                .collect();
            items.sort_by_key(|s| (s.received_at, s.id));
            items.truncate(SIGNALS_FOR_KEY_LIMIT);
            Ok(items)
        })
    }

    fn list_signal_waiters(&self, name: &str, key: &str) -> StoreFuture<'_, Vec<Step>> {
        let name = name.to_string();
        let key = key.to_string();
        Box::pin(async move {
            let state = self.state.read().await;
            let mut steps: Vec<Step> = state
                .steps
                .values()
                .filter(|s| is_waiting_signal_step(s) && step_waits_for(s, &name, &key))
                .filter(|s| {
                    state.runs.get(&s.run_id).is_some_and(|run| {
                        matches!(
                            run.status.state,
                            RunStatus::Sleeping
                                | RunStatus::Running
                                | RunStatus::Pending
                                | RunStatus::Paused
                        )
                    })
                })
                .cloned()
                .collect();
            steps.sort_by_key(|s| (s.created_at, s.id));
            Ok(steps)
        })
    }

    fn resolve_signal_step(
        &self,
        step_id: Uuid,
        output: Value,
    ) -> StoreFuture<'_, SignalStepResolution> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            let now = Utc::now();

            let step = state
                .steps
                .get_mut(&step_id)
                .ok_or(StoreError::StepNotFound(step_id))?;
            if !is_waiting_signal_step(step) {
                return Ok(SignalStepResolution::NotWaiting {
                    output: step.output.clone(),
                });
            }
            step.status.state = StepStatus::Completed;
            step.output = Some(output);
            step.completed_at = Some(now);
            step.duration_ms = step
                .started_at
                .map(|started| (now - started).num_milliseconds().max(0) as u64)
                .unwrap_or(0);
            step.updated_at = now;
            let run_id = step.run_id;

            let run = state
                .runs
                .get_mut(&run_id)
                .ok_or(StoreError::RunNotFound(run_id))?;
            let run_resumed = run.status.state == RunStatus::Sleeping;
            if run_resumed {
                run.status.state = RunStatus::Pending;
                run.scheduled_at = None;
                run.capacity_wait_kind = None;
                run.updated_at = now;
            } else if run.status.state == RunStatus::Paused
                && run.resume_status == Some(RunStatus::Sleeping)
            {
                // The signal ended the wait: the resume requeues the run
                // instead of putting it back to sleep.
                run.resume_status = Some(RunStatus::Pending);
                run.scheduled_at = None;
                run.capacity_wait_kind = None;
                run.updated_at = now;
            }

            Ok(SignalStepResolution::Resolved {
                run_id,
                run_resumed,
            })
        })
    }

    fn suspend_run_on_signal(
        &self,
        run_id: Uuid,
        step_id: Uuid,
        deadline_at: DateTime<Utc>,
    ) -> StoreFuture<'_, bool> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            let now = Utc::now();

            let waiting = state
                .steps
                .get(&step_id)
                .map(is_waiting_signal_step)
                .ok_or(StoreError::StepNotFound(step_id))?;

            let run = state
                .runs
                .get_mut(&run_id)
                .ok_or(StoreError::RunNotFound(run_id))?;
            if run.status.state != RunStatus::Running {
                return Err(StoreError::InvalidTransition {
                    from: run.status.state,
                    to: RunStatus::Sleeping,
                });
            }
            run.status.state = RunStatus::Sleeping;
            run.worker_id = None;
            run.capacity_wait_kind = None;
            run.lease_expires_at = None;
            // A signal that resolved the step before the run could sleep left
            // nothing to wait for: the next waker tick resumes it right away.
            run.scheduled_at = Some(if waiting { deadline_at } else { now });
            run.updated_at = now;

            Ok(waiting)
        })
    }

    fn purge_signals(&self, before: DateTime<Utc>) -> StoreFuture<'_, u64> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            let initial = state.signals.len();
            state.signals.retain(|s| s.received_at >= before);
            let removed = (initial - state.signals.len()) as u64;

            let kept: Vec<Uuid> = state.signals.iter().map(|s| s.id).collect();
            state
                .signal_idempotency
                .retain(|_, signal_id| kept.contains(signal_id));

            Ok(removed)
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use serde_json::json;

    use super::*;
    use crate::entities::{NewStep, StepUpdate, step_trace_id};
    use crate::memory::tests::new_run_req;
    use crate::store::RunStore;

    fn new_signal(name: &str, key: &str, idempotency_id: Option<&str>) -> NewSignal {
        NewSignal {
            name: name.to_string(),
            key: key.to_string(),
            payload: json!({"status": "success"}),
            idempotency_id: idempotency_id.map(str::to_string),
        }
    }

    /// Create a `Running` run holding a `Running` signal step on `(name, key)`.
    async fn waiting_step(store: &InMemoryStore, name: &str, key: &str) -> Step {
        let run = store
            .create_run(new_run_req("wait"))
            .await
            .unwrap()
            .into_run();
        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        let step = store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: step_trace_id(run.id, "wait-ci", 0),
                name: "wait-ci".to_string(),
                kind: StepKind::Signal,
                position: 0,
                input: Some(json!({"name": name, "key": key, "schema": {}})),
                is_error_handler: false,
            })
            .await
            .unwrap();
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    started_at: Some(Utc::now()),
                    ..StepUpdate::default()
                },
            )
            .await
            .unwrap();
        store.get_step(step.id).await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn insert_signal_is_idempotent() {
        let store = InMemoryStore::new();
        let first = store
            .insert_signal(new_signal("demo.done", "k1", Some("d-1")))
            .await
            .unwrap();
        assert!(!first.is_duplicate());

        let second = store
            .insert_signal(new_signal("demo.done", "k2", Some("d-1")))
            .await
            .unwrap();
        assert!(second.is_duplicate());
        assert_eq!(second.signal(), first.signal());

        let page = store
            .list_signals(SignalFilter::default(), 1, 20)
            .await
            .unwrap();
        assert_eq!(page.total, 1);
    }

    #[tokio::test]
    async fn insert_signal_without_idempotency_id_always_stores() {
        let store = InMemoryStore::new();
        for _ in 0..2 {
            let insert = store
                .insert_signal(new_signal("demo.done", "k1", None))
                .await
                .unwrap();
            assert!(!insert.is_duplicate());
        }
        let page = store
            .list_signals(SignalFilter::default(), 1, 20)
            .await
            .unwrap();
        assert_eq!(page.total, 2);
    }

    #[tokio::test]
    async fn list_signals_filters_and_orders_newest_first() {
        let store = InMemoryStore::new();
        store
            .insert_signal(new_signal("demo.done", "k1", None))
            .await
            .unwrap();
        let newest = store
            .insert_signal(new_signal("demo.done", "k2", None))
            .await
            .unwrap();
        store
            .insert_signal(new_signal("other", "k1", None))
            .await
            .unwrap();

        let page = store
            .list_signals(
                SignalFilter {
                    name: Some("demo.done".to_string()),
                    key: None,
                },
                1,
                20,
            )
            .await
            .unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.items[0].id, newest.signal().id);

        let page = store
            .list_signals(
                SignalFilter {
                    name: Some("demo.done".to_string()),
                    key: Some("k1".to_string()),
                },
                1,
                20,
            )
            .await
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].key, "k1");
    }

    #[tokio::test]
    async fn list_signals_for_key_respects_since_and_orders_oldest_first() {
        let store = InMemoryStore::new();
        let first = store
            .insert_signal(new_signal("demo.done", "k1", None))
            .await
            .unwrap();
        let second = store
            .insert_signal(new_signal("demo.done", "k1", None))
            .await
            .unwrap();
        store
            .insert_signal(new_signal("demo.done", "k2", None))
            .await
            .unwrap();

        let found = store
            .list_signals_for_key("demo.done", "k1", first.signal().received_at)
            .await
            .unwrap();
        let ids: Vec<Uuid> = found.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![first.signal().id, second.signal().id]);

        let later = store
            .list_signals_for_key("demo.done", "k1", Utc::now() + TimeDelta::seconds(5))
            .await
            .unwrap();
        assert!(later.is_empty());
    }

    #[tokio::test]
    async fn list_signal_waiters_matches_name_and_key() {
        let store = InMemoryStore::new();
        let step = waiting_step(&store, "demo.done", "k1").await;
        waiting_step(&store, "demo.done", "k2").await;

        let waiters = store.list_signal_waiters("demo.done", "k1").await.unwrap();
        assert_eq!(waiters.len(), 1);
        assert_eq!(waiters[0].id, step.id);
    }

    #[tokio::test]
    async fn list_signal_waiters_excludes_cancelled_runs() {
        let store = InMemoryStore::new();
        let step = waiting_step(&store, "demo.done", "k1").await;
        store
            .update_run_status(step.run_id, RunStatus::Cancelled)
            .await
            .unwrap();

        let waiters = store.list_signal_waiters("demo.done", "k1").await.unwrap();
        assert!(waiters.is_empty());
    }

    #[tokio::test]
    async fn resolve_signal_step_resumes_sleeping_run() {
        let store = InMemoryStore::new();
        let step = waiting_step(&store, "demo.done", "k1").await;
        let deadline = Utc::now() + TimeDelta::hours(1);
        assert!(
            store
                .suspend_run_on_signal(step.run_id, step.id, deadline)
                .await
                .unwrap()
        );

        let resolution = store
            .resolve_signal_step(step.id, json!({"timed_out": false}))
            .await
            .unwrap();
        assert_eq!(
            resolution,
            SignalStepResolution::Resolved {
                run_id: step.run_id,
                run_resumed: true,
            }
        );

        let run = store.get_run(step.run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Pending);
        assert!(run.scheduled_at.is_none());
        let step = store.get_step(step.id).await.unwrap().unwrap();
        assert_eq!(step.status.state, StepStatus::Completed);
        assert_eq!(step.output, Some(json!({"timed_out": false})));
    }

    #[tokio::test]
    async fn resolve_signal_step_leaves_running_run_alone() {
        let store = InMemoryStore::new();
        let step = waiting_step(&store, "demo.done", "k1").await;

        let resolution = store
            .resolve_signal_step(step.id, json!({"timed_out": false}))
            .await
            .unwrap();
        assert_eq!(
            resolution,
            SignalStepResolution::Resolved {
                run_id: step.run_id,
                run_resumed: false,
            }
        );
        let run = store.get_run(step.run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Running);
    }

    #[tokio::test]
    async fn resolve_signal_step_twice_returns_first_output() {
        let store = InMemoryStore::new();
        let step = waiting_step(&store, "demo.done", "k1").await;
        store
            .resolve_signal_step(step.id, json!({"first": true}))
            .await
            .unwrap();

        let second = store
            .resolve_signal_step(step.id, json!({"first": false}))
            .await
            .unwrap();
        assert_eq!(
            second,
            SignalStepResolution::NotWaiting {
                output: Some(json!({"first": true})),
            }
        );
    }

    #[tokio::test]
    async fn resolve_signal_step_unknown_step_errors() {
        let store = InMemoryStore::new();
        let err = store
            .resolve_signal_step(Uuid::now_v7(), json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::StepNotFound(_)));
    }

    #[tokio::test]
    async fn suspend_run_on_signal_after_resolution_schedules_now() {
        let store = InMemoryStore::new();
        let step = waiting_step(&store, "demo.done", "k1").await;
        store
            .resolve_signal_step(step.id, json!({"timed_out": false}))
            .await
            .unwrap();

        let deadline = Utc::now() + TimeDelta::hours(1);
        let waiting = store
            .suspend_run_on_signal(step.run_id, step.id, deadline)
            .await
            .unwrap();
        assert!(!waiting);

        let run = store.get_run(step.run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Sleeping);
        assert!(run.scheduled_at.is_some_and(|at| at < deadline));
    }

    #[tokio::test]
    async fn suspend_run_on_signal_rejects_non_running_run() {
        let store = InMemoryStore::new();
        let step = waiting_step(&store, "demo.done", "k1").await;
        store
            .update_run_status(step.run_id, RunStatus::Cancelled)
            .await
            .unwrap();

        let err = store
            .suspend_run_on_signal(step.run_id, step.id, Utc::now())
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidTransition { .. }));
    }

    #[tokio::test]
    async fn purge_signals_removes_old_signals_and_frees_idempotency_ids() {
        let store = InMemoryStore::new();
        store
            .insert_signal(new_signal("demo.done", "k1", Some("d-1")))
            .await
            .unwrap();

        let purged = store
            .purge_signals(Utc::now() + TimeDelta::seconds(1))
            .await
            .unwrap();
        assert_eq!(purged, 1);

        let again = store
            .insert_signal(new_signal("demo.done", "k1", Some("d-1")))
            .await
            .unwrap();
        assert!(!again.is_duplicate());
    }
}
