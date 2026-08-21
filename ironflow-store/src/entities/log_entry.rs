//! Log entry entities for persisted run/step output.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::LogStream;

/// A persisted log line from step execution.
///
/// Each entry records a single line of output alongside its metadata.
/// Entries are ordered by [`id`](LogEntry::id) (UUID v7, time-ordered).
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{LogEntry, LogStream};
/// use uuid::Uuid;
/// use chrono::Utc;
///
/// let entry = LogEntry {
///     id: Uuid::now_v7(),
///     run_id: Uuid::now_v7(),
///     step_id: Uuid::now_v7(),
///     step_name: "build".to_string(),
///     stream: LogStream::Stdout,
///     line: "Compiling ironflow v0.1.0".to_string(),
///     created_at: Utc::now(),
/// };
/// assert_eq!(entry.stream, LogStream::Stdout);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LogEntry {
    /// Unique entry ID (UUID v7, time-ordered).
    pub id: Uuid,
    /// Run that produced this log line.
    pub run_id: Uuid,
    /// Step that produced this log line.
    pub step_id: Uuid,
    /// Human-readable step name.
    pub step_name: String,
    /// Output stream.
    pub stream: LogStream,
    /// The log line content.
    pub line: String,
    /// When the line was recorded.
    pub created_at: DateTime<Utc>,
}

/// Parameters for appending a batch of log lines.
///
/// All lines in a batch share the same run, step, and stream.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{NewLogEntries, LogStream};
/// use uuid::Uuid;
///
/// let entries = NewLogEntries {
///     run_id: Uuid::now_v7(),
///     step_id: Uuid::now_v7(),
///     step_name: "build".to_string(),
///     stream: LogStream::Stdout,
///     lines: vec!["line 1".to_string(), "line 2".to_string()],
/// };
/// assert_eq!(entries.lines.len(), 2);
/// ```
#[derive(Debug, Clone)]
pub struct NewLogEntries {
    /// Run that produced these log lines.
    pub run_id: Uuid,
    /// Step that produced these log lines.
    pub step_id: Uuid,
    /// Human-readable step name.
    pub step_name: String,
    /// Output stream.
    pub stream: LogStream,
    /// The log lines to persist.
    pub lines: Vec<String>,
}

/// Filter criteria for listing log entries.
///
/// All fields are optional. When `None`, no filtering is applied
/// for that dimension.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{LogFilter, LogStream};
/// use uuid::Uuid;
///
/// let filter = LogFilter {
///     step_id: Some(Uuid::now_v7()),
///     stream: Some(LogStream::Stderr),
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct LogFilter {
    /// Filter by step ID.
    pub step_id: Option<Uuid>,
    /// Filter by output stream.
    pub stream: Option<LogStream>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_entry_serde_roundtrip() {
        let entry = LogEntry {
            id: Uuid::now_v7(),
            run_id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            step_name: "build".to_string(),
            stream: LogStream::Stdout,
            line: "Compiling ironflow v0.1.0".to_string(),
            created_at: Utc::now(),
        };

        let json = serde_json::to_string(&entry).unwrap();
        let back: LogEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(back.id, entry.id);
        assert_eq!(back.stream, LogStream::Stdout);
        assert_eq!(back.line, "Compiling ironflow v0.1.0");
    }

    #[test]
    fn log_filter_default_is_empty() {
        let filter = LogFilter::default();
        assert!(filter.step_id.is_none());
        assert!(filter.stream.is_none());
    }

    #[test]
    fn new_log_entries_creation() {
        let entries = NewLogEntries {
            run_id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            step_name: "deploy".to_string(),
            stream: LogStream::Stderr,
            lines: vec!["error!".to_string()],
        };

        assert_eq!(entries.stream, LogStream::Stderr);
        assert_eq!(entries.lines.len(), 1);
    }
}
