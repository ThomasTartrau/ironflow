//! Live log streaming infrastructure for real-time step output.
//!
//! During workflow execution, shell commands and agent invocations produce
//! output incrementally. This module provides a channel-based mechanism to
//! stream those lines to an external consumer (e.g. the worker's log pusher)
//! so that SSE clients can display output in real time.
//!
//! # Architecture
//!
//! ```text
//! ShellExecutor ──► StepLogSender ──► LogSender (mpsc) ──► LogPusher ──► API
//! AgentExecutor ─┘                                          (worker)
//! ```
//!
//! The [`LogSender`] is an unbounded MPSC channel sender. The
//! [`StepLogSender`] wraps it with step metadata (run ID, step ID, step name)
//! so that executors can emit lines with a simple `emit(stream, line)` call.
//!
//! # Examples
//!
//! ```
//! use ironflow_engine::log_sender::{self, StepLogSender};
//! use ironflow_engine::notify::LogStream;
//! use uuid::Uuid;
//!
//! let (sender, mut receiver) = log_sender::channel();
//! let step_sender = StepLogSender::new(
//!     sender,
//!     Uuid::now_v7(),
//!     Uuid::now_v7(),
//!     "build".to_string(),
//! );
//!
//! step_sender.emit(LogStream::Stdout, "Compiling ironflow v0.1.0");
//!
//! let line = receiver.try_recv().unwrap();
//! assert_eq!(line.line, "Compiling ironflow v0.1.0");
//! assert_eq!(&*line.step_name, "build");
//! ```

use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::notify::LogStream;

/// A single log line emitted during step execution.
#[derive(Debug, Clone)]
pub struct LogLine {
    /// Run that owns this step.
    pub run_id: Uuid,
    /// Step that produced the line.
    pub step_id: Uuid,
    /// Human-readable step name (shared across lines from the same step).
    pub step_name: Arc<str>,
    /// Output stream (stdout, stderr, or system).
    pub stream: LogStream,
    /// The line content.
    pub line: String,
    /// When the line was emitted.
    pub at: DateTime<Utc>,
}

/// Sender half of the log streaming channel.
pub type LogSender = mpsc::UnboundedSender<LogLine>;

/// Receiver half of the log streaming channel.
pub type LogReceiver = mpsc::UnboundedReceiver<LogLine>;

/// Create a new log streaming channel.
///
/// Returns a `(sender, receiver)` pair. Clone the sender for each
/// executor; the receiver is consumed by a single log pusher.
///
/// # Examples
///
/// ```
/// use ironflow_engine::log_sender;
///
/// let (sender, receiver) = log_sender::channel();
/// drop(receiver);
/// ```
pub fn channel() -> (LogSender, LogReceiver) {
    mpsc::unbounded_channel()
}

/// Step-scoped log sender with pre-filled metadata.
///
/// Wraps a [`LogSender`] with the run ID, step ID, and step name so that
/// executors can emit lines without repeating metadata on every call.
///
/// # Examples
///
/// ```
/// use ironflow_engine::log_sender::{self, StepLogSender};
/// use ironflow_engine::notify::LogStream;
/// use uuid::Uuid;
///
/// let (sender, _rx) = log_sender::channel();
/// let step_sender = StepLogSender::new(
///     sender,
///     Uuid::now_v7(),
///     Uuid::now_v7(),
///     "deploy".to_string(),
/// );
///
/// step_sender.emit(LogStream::Stderr, "warning: deprecated API");
/// ```
#[derive(Clone)]
pub struct StepLogSender {
    inner: LogSender,
    run_id: Uuid,
    step_id: Uuid,
    step_name: Arc<str>,
}

impl StepLogSender {
    /// Create a new step-scoped sender.
    pub fn new(inner: LogSender, run_id: Uuid, step_id: Uuid, step_name: String) -> Self {
        Self {
            inner,
            run_id,
            step_id,
            step_name: Arc::from(step_name),
        }
    }

    /// Emit a log line on the given stream.
    ///
    /// Silently drops the line if the receiver has been closed.
    pub fn emit(&self, stream: LogStream, line: &str) {
        let _ = self.inner.send(LogLine {
            run_id: self.run_id,
            step_id: self.step_id,
            step_name: self.step_name.clone(),
            stream,
            line: line.to_string(),
            at: Utc::now(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_send_and_receive() {
        let (sender, mut receiver) = channel();
        let step = StepLogSender::new(sender, Uuid::now_v7(), Uuid::now_v7(), "build".to_string());

        step.emit(LogStream::Stdout, "line 1");
        step.emit(LogStream::Stderr, "error!");

        let l1 = receiver.try_recv().unwrap();
        assert_eq!(l1.line, "line 1");
        assert_eq!(l1.stream, LogStream::Stdout);
        assert_eq!(&*l1.step_name, "build");

        let l2 = receiver.try_recv().unwrap();
        assert_eq!(l2.line, "error!");
        assert_eq!(l2.stream, LogStream::Stderr);
    }

    #[test]
    fn emit_after_receiver_dropped_does_not_panic() {
        let (sender, receiver) = channel();
        let step = StepLogSender::new(sender, Uuid::now_v7(), Uuid::now_v7(), "test".to_string());
        drop(receiver);
        step.emit(LogStream::System, "should not panic");
    }

    #[test]
    fn clone_step_sender_shares_channel() {
        let (sender, mut receiver) = channel();
        let step = StepLogSender::new(sender, Uuid::now_v7(), Uuid::now_v7(), "test".to_string());

        let cloned = step.clone();
        step.emit(LogStream::Stdout, "from original");
        cloned.emit(LogStream::Stdout, "from clone");

        assert_eq!(receiver.try_recv().unwrap().line, "from original");
        assert_eq!(receiver.try_recv().unwrap().line, "from clone");
    }
}
