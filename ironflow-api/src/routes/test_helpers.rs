use std::collections::HashMap;

use serde_json::json;

use ironflow_store::entities::{Run, RunStatus, TriggerKind};
use ironflow_store::models::NewRun;
use ironflow_store::store::Store;

pub(crate) async fn create_terminal_run(store: &dyn Store, name: &str, status: RunStatus) -> Run {
    let run = store
        .create_run(NewRun {
            workflow_name: name.to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            max_cost_usd: None,
        })
        .await
        .unwrap();
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    store.update_run_status(run.id, status).await.unwrap();
    store.get_run(run.id).await.unwrap().unwrap()
}
