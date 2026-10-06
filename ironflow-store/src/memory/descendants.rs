//! Descendant lookup of the in-memory store, behind
//! [`RunStore::list_active_descendants`](crate::store::RunStore::list_active_descendants).

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::entities::{PARENT_RUN_ID_LABEL, Run, TriggerKind};

/// The parent recorded on a sub-workflow child run, if any.
fn parent_of(run: &Run) -> Option<Uuid> {
    if !matches!(run.trigger, TriggerKind::Workflow) {
        return None;
    }
    Uuid::parse_str(run.labels.get(PARENT_RUN_ID_LABEL)?).ok()
}

/// The non-terminal descendants of `run_id` among `runs`, oldest first.
pub(super) fn active_descendants(runs: &HashMap<Uuid, Run>, run_id: Uuid) -> Vec<Run> {
    let mut children: HashMap<Uuid, Vec<&Run>> = HashMap::new();
    for run in runs.values() {
        if let Some(parent) = parent_of(run) {
            children.entry(parent).or_default().push(run);
        }
    }

    let mut visited = HashSet::from([run_id]);
    let mut frontier = vec![run_id];
    let mut found = Vec::new();
    while let Some(id) = frontier.pop() {
        for child in children.get(&id).into_iter().flatten() {
            if !visited.insert(child.id) {
                continue;
            }
            frontier.push(child.id);
            if !child.status.state.is_terminal() {
                found.push((*child).clone());
            }
        }
    }

    found.sort_by_key(|r| (r.created_at, r.id));
    found
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use uuid::Uuid;

    use crate::entities::{PARENT_RUN_ID_LABEL, Run, RunStatus, TriggerKind};
    use crate::memory::InMemoryStore;
    use crate::memory::tests::new_run_req;
    use crate::store::RunStore;

    /// Create a sub-workflow child of `parent`, moved to `status`.
    async fn create_child(store: &InMemoryStore, parent: Uuid, status: RunStatus) -> Run {
        let mut req = new_run_req("child");
        req.trigger = TriggerKind::Workflow;
        req.labels = HashMap::from([(PARENT_RUN_ID_LABEL.to_string(), parent.to_string())]);
        let child = store.create_run(req).await.unwrap().into_run();
        if status != RunStatus::Pending {
            store
                .update_run_status(child.id, RunStatus::Running)
                .await
                .unwrap();
        }
        if !matches!(status, RunStatus::Pending | RunStatus::Running) {
            store.update_run_status(child.id, status).await.unwrap();
        }
        store.get_run(child.id).await.unwrap().unwrap()
    }

    async fn create_root(store: &InMemoryStore) -> Run {
        store
            .create_run(new_run_req("root"))
            .await
            .unwrap()
            .into_run()
    }

    fn ids(runs: &[Run]) -> Vec<Uuid> {
        runs.iter().map(|r| r.id).collect()
    }

    #[tokio::test]
    async fn active_descendants_of_a_run_without_children_is_empty() {
        let store = InMemoryStore::new();
        let root = create_root(&store).await;

        assert!(
            store
                .list_active_descendants(root.id)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .list_active_descendants(Uuid::now_v7())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn active_descendants_are_found_at_any_depth_oldest_first() {
        let store = InMemoryStore::new();
        let root = create_root(&store).await;
        let child = create_child(&store, root.id, RunStatus::Running).await;
        let grandchild = create_child(&store, child.id, RunStatus::AwaitingApproval).await;
        let sibling = create_child(&store, root.id, RunStatus::Pending).await;

        let found = store.list_active_descendants(root.id).await.unwrap();
        assert_eq!(ids(&found), [child.id, grandchild.id, sibling.id]);

        let below_child = store.list_active_descendants(child.id).await.unwrap();
        assert_eq!(ids(&below_child), [grandchild.id]);
    }

    #[tokio::test]
    async fn active_descendants_skip_terminal_runs_but_cross_them() {
        let store = InMemoryStore::new();
        let root = create_root(&store).await;
        let finished = create_child(&store, root.id, RunStatus::Completed).await;
        let left_running = create_child(&store, finished.id, RunStatus::Running).await;
        create_child(&store, root.id, RunStatus::Cancelled).await;

        let found = store.list_active_descendants(root.id).await.unwrap();
        assert_eq!(ids(&found), [left_running.id]);
    }

    #[tokio::test]
    async fn active_descendants_ignore_runs_not_started_by_a_workflow_step() {
        let store = InMemoryStore::new();
        let root = create_root(&store).await;
        let mut req = new_run_req("impostor");
        req.labels = HashMap::from([(PARENT_RUN_ID_LABEL.to_string(), root.id.to_string())]);
        store.create_run(req).await.unwrap();

        assert!(
            store
                .list_active_descendants(root.id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn active_descendants_stop_on_a_label_cycle() {
        let store = InMemoryStore::new();
        let root = create_root(&store).await;
        let child = create_child(&store, root.id, RunStatus::Running).await;
        let grandchild = create_child(&store, child.id, RunStatus::Running).await;
        // The root claims to be a child of its own grand-child.
        {
            let mut state = store.state.write().await;
            let root = state.runs.get_mut(&root.id).unwrap();
            root.trigger = TriggerKind::Workflow;
            root.labels =
                HashMap::from([(PARENT_RUN_ID_LABEL.to_string(), grandchild.id.to_string())]);
        }

        let found = store.list_active_descendants(root.id).await.unwrap();
        assert_eq!(ids(&found), [child.id, grandchild.id]);
    }
}
