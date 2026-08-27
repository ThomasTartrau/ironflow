//! [`ArtifactStore`] implementation for [`InMemoryStore`].

use chrono::Utc;
use uuid::Uuid;

use crate::artifact_store::ArtifactStore;
use crate::entities::{Artifact, ArtifactLookup, NewArtifact};
use crate::error::StoreError;
use crate::store::StoreFuture;

use super::InMemoryStore;

impl ArtifactStore for InMemoryStore {
    fn create_artifact(&self, artifact: NewArtifact) -> StoreFuture<'_, Artifact> {
        Box::pin(async move {
            let mut state = self.state.write().await;

            if !state.steps.contains_key(&artifact.step_id) {
                return Err(StoreError::StepNotFound(artifact.step_id));
            }
            if state
                .artifacts
                .values()
                .any(|a| a.step_id == artifact.step_id && a.name == artifact.name)
            {
                return Err(StoreError::DuplicateArtifact {
                    step_id: artifact.step_id,
                    name: artifact.name,
                });
            }

            let now = Utc::now();
            let stored = Artifact {
                id: artifact.id,
                run_id: artifact.run_id,
                step_id: artifact.step_id,
                name: artifact.name,
                storage_key: artifact.storage_key,
                content_type: artifact.content_type,
                size_bytes: artifact.size_bytes,
                sha256: artifact.sha256,
                created_at: now,
                updated_at: now,
            };

            state.artifacts.insert(stored.id, stored.clone());
            Ok(stored)
        })
    }

    fn get_artifact(&self, step_id: Uuid, name: &str) -> StoreFuture<'_, Option<Artifact>> {
        let name = name.to_string();
        Box::pin(async move {
            let state = self.state.read().await;
            Ok(state
                .artifacts
                .values()
                .find(|a| a.step_id == step_id && a.name == name)
                .cloned())
        })
    }

    fn list_artifacts_for_run(&self, run_id: Uuid) -> StoreFuture<'_, Vec<Artifact>> {
        Box::pin(async move {
            let state = self.state.read().await;

            let mut artifacts: Vec<Artifact> = state
                .artifacts
                .values()
                .filter(|a| a.run_id == run_id)
                .cloned()
                .collect();

            artifacts.sort_by(|a, b| {
                let position = |artifact: &Artifact| {
                    state
                        .steps
                        .get(&artifact.step_id)
                        .map(|s| (s.attempt, s.position))
                        .unwrap_or((0, 0))
                };
                position(a)
                    .cmp(&position(b))
                    .then_with(|| a.name.cmp(&b.name))
            });

            Ok(artifacts)
        })
    }

    fn find_artifact_for_input(&self, lookup: ArtifactLookup) -> StoreFuture<'_, Option<Artifact>> {
        Box::pin(async move {
            let state = self.state.read().await;

            // Candidate steps: same run and attempt, matching name, positioned
            // strictly before the consumer. The closest one wins.
            let producer = state
                .steps
                .values()
                .filter(|s| {
                    s.run_id == lookup.run_id
                        && s.attempt == lookup.attempt
                        && s.name == lookup.step_name
                        && s.position < lookup.before_position
                })
                .max_by_key(|s| s.position);

            let Some(producer) = producer else {
                return Ok(None);
            };

            Ok(state
                .artifacts
                .values()
                .find(|a| a.step_id == producer.id && a.name == lookup.name)
                .cloned())
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::entities::{NewStep, StepKind, step_trace_id};
    use crate::store::RunStore;

    use super::super::tests::new_run_req;
    use super::*;

    /// Create a run with one step at `position`, returning both ids.
    async fn run_with_step(store: &InMemoryStore, step_name: &str, position: u32) -> (Uuid, Uuid) {
        let run = store
            .create_run(new_run_req("artifacts"))
            .await
            .expect("create run")
            .into_run();
        let step = store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: step_trace_id(run.id, step_name, position),
                name: step_name.to_string(),
                kind: StepKind::Shell,
                position,
                input: Some(json!({})),
                is_error_handler: false,
            })
            .await
            .expect("create step");

        (run.id, step.id)
    }

    fn new_artifact(run_id: Uuid, step_id: Uuid, name: &str) -> NewArtifact {
        let id = Uuid::now_v7();
        NewArtifact {
            id,
            run_id,
            step_id,
            name: name.to_string(),
            storage_key: format!("artifacts/{run_id}/{step_id}/{id}"),
            content_type: "text/plain".to_string(),
            size_bytes: 3,
            sha256: "0".repeat(64),
        }
    }

    #[tokio::test]
    async fn create_then_get_roundtrips() {
        let store = InMemoryStore::new();
        let (run_id, step_id) = run_with_step(&store, "build", 0).await;

        let created = store
            .create_artifact(new_artifact(run_id, step_id, "report.html"))
            .await
            .expect("create");

        let fetched = store
            .get_artifact(step_id, "report.html")
            .await
            .expect("get")
            .expect("present");

        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.name, "report.html");
        assert_eq!(fetched.size_bytes, 3);
    }

    #[tokio::test]
    async fn get_on_an_unknown_name_returns_none() {
        let store = InMemoryStore::new();
        let (_run_id, step_id) = run_with_step(&store, "build", 0).await;

        assert!(
            store
                .get_artifact(step_id, "nope")
                .await
                .expect("get")
                .is_none()
        );
    }

    #[tokio::test]
    async fn create_on_an_unknown_step_is_rejected() {
        let store = InMemoryStore::new();

        let err = store
            .create_artifact(new_artifact(Uuid::now_v7(), Uuid::now_v7(), "a.txt"))
            .await
            .expect_err("step does not exist");

        assert!(matches!(err, StoreError::StepNotFound(_)));
    }

    #[tokio::test]
    async fn the_same_name_twice_on_a_step_is_rejected() {
        let store = InMemoryStore::new();
        let (run_id, step_id) = run_with_step(&store, "build", 0).await;

        store
            .create_artifact(new_artifact(run_id, step_id, "a.txt"))
            .await
            .expect("first");

        let err = store
            .create_artifact(new_artifact(run_id, step_id, "a.txt"))
            .await
            .expect_err("duplicate");

        assert!(matches!(err, StoreError::DuplicateArtifact { .. }));
    }

    #[tokio::test]
    async fn the_same_name_on_two_steps_is_allowed() {
        let store = InMemoryStore::new();
        let (run_id, first) = run_with_step(&store, "build", 0).await;
        let second = store
            .create_step(NewStep {
                run_id,
                trace_id: step_trace_id(run_id, "test", 1),
                name: "test".to_string(),
                kind: StepKind::Shell,
                position: 1,
                input: None,
                is_error_handler: false,
            })
            .await
            .expect("create step")
            .id;

        store
            .create_artifact(new_artifact(run_id, first, "a.txt"))
            .await
            .expect("first");
        store
            .create_artifact(new_artifact(run_id, second, "a.txt"))
            .await
            .expect("second");

        assert_eq!(
            store
                .list_artifacts_for_run(run_id)
                .await
                .expect("list")
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn list_is_scoped_to_the_run() {
        let store = InMemoryStore::new();
        let (run_a, step_a) = run_with_step(&store, "build", 0).await;
        let (run_b, step_b) = run_with_step(&store, "build", 0).await;

        store
            .create_artifact(new_artifact(run_a, step_a, "a.txt"))
            .await
            .expect("a");
        store
            .create_artifact(new_artifact(run_b, step_b, "b.txt"))
            .await
            .expect("b");

        let listed = store.list_artifacts_for_run(run_a).await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "a.txt");
    }

    #[tokio::test]
    async fn list_on_a_run_without_artifacts_is_empty() {
        let store = InMemoryStore::new();
        let (run_id, _) = run_with_step(&store, "build", 0).await;

        assert!(
            store
                .list_artifacts_for_run(run_id)
                .await
                .expect("list")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn input_resolves_from_an_earlier_step() {
        let store = InMemoryStore::new();
        let (run_id, step_id) = run_with_step(&store, "build", 0).await;
        store
            .create_artifact(new_artifact(run_id, step_id, "report.html"))
            .await
            .expect("create");

        let found = store
            .find_artifact_for_input(ArtifactLookup {
                run_id,
                attempt: 1,
                before_position: 1,
                step_name: "build".to_string(),
                name: "report.html".to_string(),
            })
            .await
            .expect("lookup");

        assert_eq!(found.expect("present").step_id, step_id);
    }

    #[tokio::test]
    async fn input_ignores_a_step_at_or_after_the_consumer() {
        let store = InMemoryStore::new();
        let (run_id, step_id) = run_with_step(&store, "build", 2).await;
        store
            .create_artifact(new_artifact(run_id, step_id, "report.html"))
            .await
            .expect("create");

        let found = store
            .find_artifact_for_input(ArtifactLookup {
                run_id,
                attempt: 1,
                before_position: 2,
                step_name: "build".to_string(),
                name: "report.html".to_string(),
            })
            .await
            .expect("lookup");

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn input_picks_the_closest_producer_when_names_repeat() {
        let store = InMemoryStore::new();
        let (run_id, first) = run_with_step(&store, "build", 0).await;
        let second = store
            .create_step(NewStep {
                run_id,
                trace_id: step_trace_id(run_id, "build", 1),
                name: "build".to_string(),
                kind: StepKind::Shell,
                position: 1,
                input: None,
                is_error_handler: false,
            })
            .await
            .expect("create step")
            .id;

        store
            .create_artifact(new_artifact(run_id, first, "report.html"))
            .await
            .expect("first");
        store
            .create_artifact(new_artifact(run_id, second, "report.html"))
            .await
            .expect("second");

        let found = store
            .find_artifact_for_input(ArtifactLookup {
                run_id,
                attempt: 1,
                before_position: 2,
                step_name: "build".to_string(),
                name: "report.html".to_string(),
            })
            .await
            .expect("lookup")
            .expect("present");

        assert_eq!(found.step_id, second);
    }

    #[tokio::test]
    async fn input_does_not_cross_attempts() {
        let store = InMemoryStore::new();
        let (run_id, step_id) = run_with_step(&store, "build", 0).await;
        store
            .create_artifact(new_artifact(run_id, step_id, "report.html"))
            .await
            .expect("create");

        let found = store
            .find_artifact_for_input(ArtifactLookup {
                run_id,
                attempt: 2,
                before_position: 1,
                step_name: "build".to_string(),
                name: "report.html".to_string(),
            })
            .await
            .expect("lookup");

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn input_does_not_cross_runs() {
        let store = InMemoryStore::new();
        let (run_a, step_a) = run_with_step(&store, "build", 0).await;
        let (run_b, _) = run_with_step(&store, "build", 0).await;
        store
            .create_artifact(new_artifact(run_a, step_a, "report.html"))
            .await
            .expect("create");

        let found = store
            .find_artifact_for_input(ArtifactLookup {
                run_id: run_b,
                attempt: 1,
                before_position: 1,
                step_name: "build".to_string(),
                name: "report.html".to_string(),
            })
            .await
            .expect("lookup");

        assert!(found.is_none());
    }
}
