//! [`LogStore`] trait implementation for [`InMemoryStore`].

use chrono::Utc;
use uuid::Uuid;

use crate::entities::{LogEntry, LogFilter, NewLogEntries};
use crate::log_store::LogStore;
use crate::store::StoreFuture;

use super::InMemoryStore;

impl LogStore for InMemoryStore {
    fn append_logs(&self, entries: NewLogEntries) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let now = Utc::now();
            let mut state = self.state.write().await;

            for line in entries.lines {
                state.log_entries.push(LogEntry {
                    id: Uuid::now_v7(),
                    run_id: entries.run_id,
                    step_id: entries.step_id,
                    step_name: entries.step_name.clone(),
                    stream: entries.stream,
                    line,
                    created_at: now,
                });
            }

            Ok(())
        })
    }

    fn get_logs(
        &self,
        run_id: Uuid,
        filter: LogFilter,
        cursor: Option<Uuid>,
        limit: u32,
    ) -> StoreFuture<'_, Vec<LogEntry>> {
        Box::pin(async move {
            let state = self.state.read().await;

            let entries: Vec<LogEntry> = state
                .log_entries
                .iter()
                .filter(|e| {
                    if e.run_id != run_id {
                        return false;
                    }
                    if let Some(step_id) = filter.step_id
                        && e.step_id != step_id
                    {
                        return false;
                    }
                    if let Some(stream) = filter.stream
                        && e.stream != stream
                    {
                        return false;
                    }
                    if let Some(cursor) = cursor
                        && e.id <= cursor
                    {
                        return false;
                    }
                    true
                })
                .take(limit as usize)
                .cloned()
                .collect();

            Ok(entries)
        })
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::entities::{LogFilter, LogStream, NewLogEntries};
    use crate::log_store::LogStore;
    use crate::memory::InMemoryStore;

    fn new_entries(run_id: Uuid, step_id: Uuid, stream: LogStream) -> NewLogEntries {
        NewLogEntries {
            run_id,
            step_id,
            step_name: "build".to_string(),
            stream,
            lines: vec!["line 1".to_string(), "line 2".to_string()],
        }
    }

    #[tokio::test]
    async fn append_and_get_golden_path() {
        let store = InMemoryStore::new();
        let run_id = Uuid::now_v7();
        let step_id = Uuid::now_v7();

        store
            .append_logs(new_entries(run_id, step_id, LogStream::Stdout))
            .await
            .unwrap();

        let logs = store
            .get_logs(run_id, LogFilter::default(), None, 100)
            .await
            .unwrap();

        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].line, "line 1");
        assert_eq!(logs[1].line, "line 2");
        assert_eq!(logs[0].run_id, run_id);
        assert_eq!(logs[0].step_id, step_id);
        assert_eq!(logs[0].step_name, "build");
        assert_eq!(logs[0].stream, LogStream::Stdout);
    }

    #[tokio::test]
    async fn get_empty_returns_empty_vec() {
        let store = InMemoryStore::new();
        let run_id = Uuid::now_v7();

        let logs = store
            .get_logs(run_id, LogFilter::default(), None, 100)
            .await
            .unwrap();

        assert!(logs.is_empty());
    }

    #[tokio::test]
    async fn cursor_based_pagination() {
        let store = InMemoryStore::new();
        let run_id = Uuid::now_v7();
        let step_id = Uuid::now_v7();

        store
            .append_logs(NewLogEntries {
                run_id,
                step_id,
                step_name: "build".to_string(),
                stream: LogStream::Stdout,
                lines: (0..5).map(|i| format!("line {i}")).collect(),
            })
            .await
            .unwrap();

        let page1 = store
            .get_logs(run_id, LogFilter::default(), None, 2)
            .await
            .unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page1[0].line, "line 0");
        assert_eq!(page1[1].line, "line 1");

        let cursor = page1.last().unwrap().id;
        let page2 = store
            .get_logs(run_id, LogFilter::default(), Some(cursor), 2)
            .await
            .unwrap();
        assert_eq!(page2.len(), 2);
        assert_eq!(page2[0].line, "line 2");
        assert_eq!(page2[1].line, "line 3");

        let cursor = page2.last().unwrap().id;
        let page3 = store
            .get_logs(run_id, LogFilter::default(), Some(cursor), 2)
            .await
            .unwrap();
        assert_eq!(page3.len(), 1);
        assert_eq!(page3[0].line, "line 4");
    }

    #[tokio::test]
    async fn filter_by_step_id() {
        let store = InMemoryStore::new();
        let run_id = Uuid::now_v7();
        let step_a = Uuid::now_v7();
        let step_b = Uuid::now_v7();

        store
            .append_logs(new_entries(run_id, step_a, LogStream::Stdout))
            .await
            .unwrap();
        store
            .append_logs(new_entries(run_id, step_b, LogStream::Stdout))
            .await
            .unwrap();

        let filter = LogFilter {
            step_id: Some(step_a),
            ..LogFilter::default()
        };
        let logs = store.get_logs(run_id, filter, None, 100).await.unwrap();

        assert_eq!(logs.len(), 2);
        assert!(logs.iter().all(|e| e.step_id == step_a));
    }

    #[tokio::test]
    async fn filter_by_stream() {
        let store = InMemoryStore::new();
        let run_id = Uuid::now_v7();
        let step_id = Uuid::now_v7();

        store
            .append_logs(new_entries(run_id, step_id, LogStream::Stdout))
            .await
            .unwrap();
        store
            .append_logs(new_entries(run_id, step_id, LogStream::Stderr))
            .await
            .unwrap();

        let filter = LogFilter {
            stream: Some(LogStream::Stderr),
            ..LogFilter::default()
        };
        let logs = store.get_logs(run_id, filter, None, 100).await.unwrap();

        assert_eq!(logs.len(), 2);
        assert!(logs.iter().all(|e| e.stream == LogStream::Stderr));
    }

    #[tokio::test]
    async fn filter_by_step_id_and_stream() {
        let store = InMemoryStore::new();
        let run_id = Uuid::now_v7();
        let step_a = Uuid::now_v7();
        let step_b = Uuid::now_v7();

        store
            .append_logs(new_entries(run_id, step_a, LogStream::Stdout))
            .await
            .unwrap();
        store
            .append_logs(new_entries(run_id, step_a, LogStream::Stderr))
            .await
            .unwrap();
        store
            .append_logs(new_entries(run_id, step_b, LogStream::Stdout))
            .await
            .unwrap();

        let filter = LogFilter {
            step_id: Some(step_a),
            stream: Some(LogStream::Stderr),
        };
        let logs = store.get_logs(run_id, filter, None, 100).await.unwrap();

        assert_eq!(logs.len(), 2);
        assert!(logs.iter().all(|e| e.step_id == step_a));
        assert!(logs.iter().all(|e| e.stream == LogStream::Stderr));
    }

    #[tokio::test]
    async fn different_runs_are_isolated() {
        let store = InMemoryStore::new();
        let run_a = Uuid::now_v7();
        let run_b = Uuid::now_v7();
        let step_id = Uuid::now_v7();

        store
            .append_logs(new_entries(run_a, step_id, LogStream::Stdout))
            .await
            .unwrap();
        store
            .append_logs(new_entries(run_b, step_id, LogStream::Stdout))
            .await
            .unwrap();

        let logs_a = store
            .get_logs(run_a, LogFilter::default(), None, 100)
            .await
            .unwrap();
        assert_eq!(logs_a.len(), 2);
        assert!(logs_a.iter().all(|e| e.run_id == run_a));

        let logs_b = store
            .get_logs(run_b, LogFilter::default(), None, 100)
            .await
            .unwrap();
        assert_eq!(logs_b.len(), 2);
        assert!(logs_b.iter().all(|e| e.run_id == run_b));
    }

    #[tokio::test]
    async fn entries_have_unique_ids() {
        let store = InMemoryStore::new();
        let run_id = Uuid::now_v7();
        let step_id = Uuid::now_v7();

        store
            .append_logs(new_entries(run_id, step_id, LogStream::Stdout))
            .await
            .unwrap();

        let logs = store
            .get_logs(run_id, LogFilter::default(), None, 100)
            .await
            .unwrap();

        assert_ne!(logs[0].id, logs[1].id);
    }
}
