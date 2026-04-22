//! Log pusher -- reads log lines from the engine and pushes them to the API.
//!
//! Batches lines by time window (200ms) and groups them by
//! `(run_id, step_id, stream)` before sending to minimize HTTP overhead.

use std::collections::HashMap;
use std::time::Duration;

use reqwest::Client;
use serde::Serialize;
use tracing::warn;
use uuid::Uuid;

use ironflow_engine::log_sender::LogReceiver;
use ironflow_engine::notify::LogStream;

const BATCH_WINDOW: Duration = Duration::from_millis(200);
const MAX_BATCH_SIZE: usize = 100;

/// Pushes buffered log lines to the API.
pub struct LogPusher {
    client: Client,
    base_url: String,
    token: String,
}

/// Request body for `POST /api/v1/internal/runs/:id/logs`.
#[derive(Serialize)]
struct PushLogsPayload {
    step_id: Uuid,
    step_name: String,
    stream: LogStream,
    lines: Vec<String>,
}

/// Grouping key for batched log lines.
#[derive(PartialEq, Eq, Hash)]
struct GroupKey {
    run_id: Uuid,
    step_id: Uuid,
    stream: LogStream,
}

impl LogPusher {
    /// Create a new log pusher targeting the given API.
    pub fn new(base_url: &str, token: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build log pusher HTTP client");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    /// Run the pusher loop until the channel closes.
    pub async fn run(self, mut receiver: LogReceiver) {
        let mut buffer = Vec::new();

        loop {
            match tokio::time::timeout(BATCH_WINDOW, receiver.recv()).await {
                Ok(Some(line)) => {
                    buffer.push(line);
                    while buffer.len() < MAX_BATCH_SIZE {
                        match receiver.try_recv() {
                            Ok(line) => buffer.push(line),
                            Err(_) => break,
                        }
                    }
                }
                Ok(None) => break,
                Err(_) => {}
            }

            if !buffer.is_empty() {
                self.flush(&mut buffer).await;
            }
        }

        if !buffer.is_empty() {
            self.flush(&mut buffer).await;
        }
    }

    /// Group buffered lines and send one HTTP request per group.
    async fn flush(&self, buffer: &mut Vec<ironflow_engine::log_sender::LogLine>) {
        let mut groups: HashMap<GroupKey, (String, Vec<String>)> = HashMap::new();

        for line in buffer.drain(..) {
            let key = GroupKey {
                run_id: line.run_id,
                step_id: line.step_id,
                stream: line.stream,
            };
            let entry = groups
                .entry(key)
                .or_insert_with(|| (line.step_name.to_string(), Vec::new()));
            entry.1.push(line.line);
        }

        for (key, (step_name, lines)) in groups {
            let url = format!("{}/api/v1/internal/runs/{}/logs", self.base_url, key.run_id);
            let payload = PushLogsPayload {
                step_id: key.step_id,
                step_name,
                stream: key.stream,
                lines,
            };

            let result = self
                .client
                .post(&url)
                .bearer_auth(&self.token)
                .json(&payload)
                .send()
                .await;

            match result {
                Ok(resp) if !resp.status().is_success() => {
                    warn!(
                        run_id = %key.run_id,
                        status = %resp.status(),
                        "log push returned non-success status"
                    );
                }
                Err(e) => {
                    warn!(
                        run_id = %key.run_id,
                        error = %e,
                        "failed to push logs"
                    );
                }
                _ => {}
            }
        }
    }
}
