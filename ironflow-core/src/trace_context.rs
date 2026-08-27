//! W3C Trace Context propagation for workflow distributed tracing.
//!
//! Provides [`WorkflowTraceContext`] to generate and parse W3C `traceparent`
//! headers, enabling correlation between Ironflow workflow spans and
//! downstream service spans (LLM providers, MCP servers, etc.).
//!
//! The `traceparent` header follows the
//! [W3C Trace Context](https://www.w3.org/TR/trace-context/) format:
//!
//! ```text
//! 00-{trace_id}-{span_id}-{trace_flags}
//!  |     |          |          |
//!  |     |          |          +-- 2 hex (01 = sampled)
//!  |     |          +-- 16 hex (8 bytes)
//!  |     +-- 32 hex (16 bytes)
//!  +-- version (always "00")
//! ```
//!
//! # Examples
//!
//! ```
//! use ironflow_core::trace_context::WorkflowTraceContext;
//!
//! // Create a root context and emit the traceparent header.
//! let root = WorkflowTraceContext::new_root();
//! let header = root.to_traceparent();
//! assert!(header.starts_with("00-"));
//!
//! // Derive a child span (preserves trace_id, new span_id).
//! let child = root.child();
//! assert_eq!(child.trace_id(), root.trace_id());
//! assert_ne!(child.span_id(), root.span_id());
//!
//! // Parse an incoming traceparent header.
//! let parsed = WorkflowTraceContext::from_traceparent(&header).unwrap();
//! assert_eq!(parsed.trace_id(), root.trace_id());
//! ```

use std::fmt;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

static SPAN_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Errors that can occur when parsing a `traceparent` header.
///
/// # Examples
///
/// ```
/// use ironflow_core::trace_context::{WorkflowTraceContext, TraceContextError};
///
/// let err = WorkflowTraceContext::from_traceparent("bad").unwrap_err();
/// assert!(matches!(err, TraceContextError::InvalidFormat { .. }));
/// ```
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TraceContextError {
    /// The header does not have the expected 4-field format.
    #[error("invalid traceparent format: expected 4 dash-separated fields, got {field_count}")]
    InvalidFormat {
        /// Number of fields found.
        field_count: usize,
    },

    /// The trace-id field is not valid lowercase hex of the expected length.
    #[error("invalid trace-id: expected 32 lowercase hex chars, got \"{value}\"")]
    InvalidTraceId {
        /// The raw value found.
        value: String,
    },

    /// The span-id (parent-id) field is not valid lowercase hex of the expected length.
    #[error("invalid span-id: expected 16 lowercase hex chars, got \"{value}\"")]
    InvalidSpanId {
        /// The raw value found.
        value: String,
    },

    /// The trace-id is all zeros, which is invalid per the W3C spec.
    #[error("trace-id must not be all zeros")]
    ZeroTraceId,

    /// The span-id is all zeros, which is invalid per the W3C spec.
    #[error("span-id must not be all zeros")]
    ZeroSpanId,
}

/// W3C Trace Context for distributed tracing across workflow steps.
///
/// Each `WorkflowTraceContext` carries a `trace_id` (32 lowercase hex chars)
/// and a `span_id` (16 lowercase hex chars). Use [`new_root`](Self::new_root)
/// to start a new trace, [`child`](Self::child) to create a child span, and
/// [`to_traceparent`](Self::to_traceparent) to emit the W3C header.
///
/// # Examples
///
/// ```
/// use ironflow_core::trace_context::WorkflowTraceContext;
///
/// let ctx = WorkflowTraceContext::new_root();
/// assert_eq!(ctx.trace_id().len(), 32);
/// assert_eq!(ctx.span_id().len(), 16);
/// assert_eq!(ctx.to_traceparent().len(), 55); // 2 + 1 + 32 + 1 + 16 + 1 + 2
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTraceContext {
    trace_id: String,
    span_id: String,
}

impl WorkflowTraceContext {
    /// Create a new root trace context with a random trace-id and span-id.
    ///
    /// The trace-id is derived from the current timestamp, process ID, and an
    /// atomic counter to ensure uniqueness without requiring a CSPRNG.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::trace_context::WorkflowTraceContext;
    ///
    /// let ctx = WorkflowTraceContext::new_root();
    /// assert_eq!(ctx.trace_id().len(), 32);
    /// assert_eq!(ctx.span_id().len(), 16);
    /// ```
    pub fn new_root() -> Self {
        let trace_id = generate_trace_id();
        let span_id = generate_span_id();
        Self { trace_id, span_id }
    }

    /// Create a trace context derived from a workflow run ID.
    ///
    /// The `run_id` is hashed (SHA-256) to produce a deterministic 32-hex
    /// trace-id. This allows correlating all spans of a given workflow run
    /// under a single trace. A fresh span-id is generated for this context.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::trace_context::WorkflowTraceContext;
    ///
    /// let ctx = WorkflowTraceContext::from_workflow_run_id("run-abc-123");
    /// assert_eq!(ctx.trace_id().len(), 32);
    ///
    /// // Same run_id always produces the same trace_id.
    /// let ctx2 = WorkflowTraceContext::from_workflow_run_id("run-abc-123");
    /// assert_eq!(ctx.trace_id(), ctx2.trace_id());
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `run_id` is empty.
    pub fn from_workflow_run_id(run_id: &str) -> Self {
        assert!(!run_id.is_empty(), "run_id must not be empty");
        let mut hasher = Sha256::new();
        hasher.update(run_id.as_bytes());
        let hash = hasher.finalize();
        let trace_id = hex_encode(&hash[..16]);
        let span_id = generate_span_id();
        Self { trace_id, span_id }
    }

    /// Create a child context that shares this trace-id but has a new span-id.
    ///
    /// Use this when a workflow step fans out to sub-steps: each child gets
    /// its own span-id while remaining part of the same trace.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::trace_context::WorkflowTraceContext;
    ///
    /// let parent = WorkflowTraceContext::new_root();
    /// let child = parent.child();
    /// assert_eq!(child.trace_id(), parent.trace_id());
    /// assert_ne!(child.span_id(), parent.span_id());
    /// ```
    pub fn child(&self) -> Self {
        Self {
            trace_id: self.trace_id.clone(),
            span_id: generate_span_id(),
        }
    }

    /// Format as a W3C `traceparent` header value.
    ///
    /// The output follows the format `00-{trace_id}-{span_id}-01`, where
    /// `01` indicates the trace is sampled.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::trace_context::WorkflowTraceContext;
    ///
    /// let ctx = WorkflowTraceContext::from_traceparent(
    ///     "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
    /// ).unwrap();
    /// assert_eq!(
    ///     ctx.to_traceparent(),
    ///     "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
    /// );
    /// ```
    pub fn to_traceparent(&self) -> String {
        format!("00-{}-{}-01", self.trace_id, self.span_id)
    }

    /// Parse a W3C `traceparent` header into a `WorkflowTraceContext`.
    ///
    /// Accepts any version field (not just `"00"`), but always emits
    /// version `"00"` when calling [`to_traceparent`](Self::to_traceparent).
    /// Extra fields beyond the 4th are silently ignored.
    ///
    /// # Errors
    ///
    /// Returns [`TraceContextError`] if the header is malformed:
    /// - Fewer than 4 dash-separated fields
    /// - trace-id is not exactly 32 lowercase hex characters
    /// - span-id is not exactly 16 lowercase hex characters
    /// - trace-id or span-id is all zeros
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::trace_context::WorkflowTraceContext;
    ///
    /// let ctx = WorkflowTraceContext::from_traceparent(
    ///     "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
    /// ).unwrap();
    /// assert_eq!(ctx.trace_id(), "4bf92f3577b34da6a3ce929d0e0e4736");
    /// assert_eq!(ctx.span_id(), "00f067aa0ba902b7");
    /// ```
    pub fn from_traceparent(header: &str) -> Result<Self, TraceContextError> {
        let parts: Vec<&str> = header.split('-').collect();
        if parts.len() < 4 {
            return Err(TraceContextError::InvalidFormat {
                field_count: parts.len(),
            });
        }

        let trace_id = parts[1];
        let span_id = parts[2];

        if trace_id.len() != 32 || !trace_id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(TraceContextError::InvalidTraceId {
                value: trace_id.to_string(),
            });
        }

        if trace_id.chars().all(|c| c == '0') {
            return Err(TraceContextError::ZeroTraceId);
        }

        if span_id.len() != 16 || !span_id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(TraceContextError::InvalidSpanId {
                value: span_id.to_string(),
            });
        }

        if span_id.chars().all(|c| c == '0') {
            return Err(TraceContextError::ZeroSpanId);
        }

        Ok(Self {
            trace_id: trace_id.to_lowercase(),
            span_id: span_id.to_lowercase(),
        })
    }

    /// Return the trace-id (32 lowercase hex characters).
    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    /// Return the span-id (16 lowercase hex characters).
    pub fn span_id(&self) -> &str {
        &self.span_id
    }
}

impl fmt::Display for WorkflowTraceContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_traceparent())
    }
}

fn generate_trace_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = SPAN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = process::id();

    let mut hasher = Sha256::new();
    hasher.update(nanos.to_le_bytes());
    hasher.update(counter.to_le_bytes());
    hasher.update(pid.to_le_bytes());
    hasher.update(b"trace");
    let hash = hasher.finalize();
    hex_encode(&hash[..16])
}

fn generate_span_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = SPAN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = process::id();

    let mut hasher = Sha256::new();
    hasher.update(nanos.to_le_bytes());
    hasher.update(counter.to_le_bytes());
    hasher.update(pid.to_le_bytes());
    hasher.update(b"span");
    let hash = hasher.finalize();
    hex_encode(&hash[..8])
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_root_produces_valid_ids() {
        let ctx = WorkflowTraceContext::new_root();
        assert_eq!(ctx.trace_id().len(), 32, "trace_id must be 32 hex chars");
        assert_eq!(ctx.span_id().len(), 16, "span_id must be 16 hex chars");
        assert!(
            ctx.trace_id().chars().all(|c| c.is_ascii_hexdigit()),
            "trace_id must be valid hex"
        );
        assert!(
            ctx.span_id().chars().all(|c| c.is_ascii_hexdigit()),
            "span_id must be valid hex"
        );
    }

    #[test]
    fn new_root_produces_unique_contexts() {
        let a = WorkflowTraceContext::new_root();
        let b = WorkflowTraceContext::new_root();
        assert_ne!(a.trace_id(), b.trace_id());
    }

    #[test]
    fn from_workflow_run_id_deterministic() {
        let a = WorkflowTraceContext::from_workflow_run_id("run-abc-123");
        let b = WorkflowTraceContext::from_workflow_run_id("run-abc-123");
        assert_eq!(
            a.trace_id(),
            b.trace_id(),
            "same run_id must produce same trace_id"
        );
        assert_eq!(a.trace_id().len(), 32);
        assert!(a.trace_id().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn from_workflow_run_id_different_inputs() {
        let a = WorkflowTraceContext::from_workflow_run_id("run-1");
        let b = WorkflowTraceContext::from_workflow_run_id("run-2");
        assert_ne!(a.trace_id(), b.trace_id());
    }

    #[test]
    #[should_panic(expected = "run_id must not be empty")]
    fn from_workflow_run_id_empty_panics() {
        WorkflowTraceContext::from_workflow_run_id("");
    }

    #[test]
    fn child_preserves_trace_id() {
        let parent = WorkflowTraceContext::new_root();
        let child = parent.child();
        assert_eq!(child.trace_id(), parent.trace_id());
        assert_ne!(child.span_id(), parent.span_id());
        assert_eq!(child.span_id().len(), 16);
    }

    #[test]
    fn child_children_are_unique() {
        let parent = WorkflowTraceContext::new_root();
        let c1 = parent.child();
        let c2 = parent.child();
        assert_ne!(c1.span_id(), c2.span_id());
        assert_eq!(c1.trace_id(), c2.trace_id());
    }

    #[test]
    fn to_traceparent_format() {
        let ctx = WorkflowTraceContext::new_root();
        let header = ctx.to_traceparent();

        assert!(header.starts_with("00-"), "must start with version 00");
        assert!(header.ends_with("-01"), "must end with trace-flags 01");
        assert_eq!(header.len(), 55, "00-<32>-<16>-01 = 55 chars");

        let parts: Vec<&str> = header.split('-').collect();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "00");
        assert_eq!(parts[1], ctx.trace_id());
        assert_eq!(parts[2], ctx.span_id());
        assert_eq!(parts[3], "01");
    }

    #[test]
    fn display_matches_to_traceparent() {
        let ctx = WorkflowTraceContext::new_root();
        assert_eq!(format!("{ctx}"), ctx.to_traceparent());
    }

    #[test]
    fn from_traceparent_valid() {
        let ctx = WorkflowTraceContext::from_traceparent(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        )
        .unwrap();
        assert_eq!(ctx.trace_id(), "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(ctx.span_id(), "00f067aa0ba902b7");
    }

    #[test]
    fn from_traceparent_accepts_other_versions() {
        let ctx = WorkflowTraceContext::from_traceparent(
            "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        )
        .unwrap();
        assert_eq!(ctx.trace_id(), "4bf92f3577b34da6a3ce929d0e0e4736");
    }

    #[test]
    fn from_traceparent_ignores_extra_fields() {
        let ctx = WorkflowTraceContext::from_traceparent(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra-stuff",
        )
        .unwrap();
        assert_eq!(ctx.trace_id(), "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(ctx.span_id(), "00f067aa0ba902b7");
    }

    #[test]
    fn from_traceparent_uppercase_hex_normalized() {
        let ctx = WorkflowTraceContext::from_traceparent(
            "00-4BF92F3577B34DA6A3CE929D0E0E4736-00F067AA0BA902B7-01",
        )
        .unwrap();
        assert_eq!(ctx.trace_id(), "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(ctx.span_id(), "00f067aa0ba902b7");
    }

    #[test]
    fn from_traceparent_invalid_too_few_fields() {
        let err = WorkflowTraceContext::from_traceparent("00-abc").unwrap_err();
        assert!(matches!(
            err,
            TraceContextError::InvalidFormat { field_count: 2 }
        ));
    }

    #[test]
    fn from_traceparent_invalid_trace_id_length() {
        let err = WorkflowTraceContext::from_traceparent("00-abc-00f067aa0ba902b7-01").unwrap_err();
        assert!(matches!(err, TraceContextError::InvalidTraceId { .. }));
    }

    #[test]
    fn from_traceparent_invalid_trace_id_hex() {
        let err = WorkflowTraceContext::from_traceparent(
            "00-zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz-00f067aa0ba902b7-01",
        )
        .unwrap_err();
        assert!(matches!(err, TraceContextError::InvalidTraceId { .. }));
    }

    #[test]
    fn from_traceparent_invalid_span_id_length() {
        let err =
            WorkflowTraceContext::from_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-abc-01")
                .unwrap_err();
        assert!(matches!(err, TraceContextError::InvalidSpanId { .. }));
    }

    #[test]
    fn from_traceparent_zero_trace_id() {
        let err = WorkflowTraceContext::from_traceparent(
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
        )
        .unwrap_err();
        assert!(matches!(err, TraceContextError::ZeroTraceId));
    }

    #[test]
    fn from_traceparent_zero_span_id() {
        let err = WorkflowTraceContext::from_traceparent(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
        )
        .unwrap_err();
        assert!(matches!(err, TraceContextError::ZeroSpanId));
    }

    #[test]
    fn from_traceparent_empty_string() {
        let err = WorkflowTraceContext::from_traceparent("").unwrap_err();
        assert!(matches!(err, TraceContextError::InvalidFormat { .. }));
    }

    #[test]
    fn roundtrip_to_from_traceparent() {
        let original = WorkflowTraceContext::new_root();
        let header = original.to_traceparent();
        let parsed = WorkflowTraceContext::from_traceparent(&header).unwrap();
        assert_eq!(parsed.trace_id(), original.trace_id());
        assert_eq!(parsed.span_id(), original.span_id());
        assert_eq!(parsed, original);
    }

    #[test]
    fn roundtrip_from_workflow_run_id() {
        let ctx = WorkflowTraceContext::from_workflow_run_id("my-run-42");
        let header = ctx.to_traceparent();
        let parsed = WorkflowTraceContext::from_traceparent(&header).unwrap();
        assert_eq!(parsed.trace_id(), ctx.trace_id());
        assert_eq!(parsed.span_id(), ctx.span_id());
    }

    #[test]
    fn serde_roundtrip() {
        let ctx = WorkflowTraceContext::new_root();
        let json = serde_json::to_string(&ctx).unwrap();
        let back: WorkflowTraceContext = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ctx);
    }

    #[test]
    fn serde_contains_expected_fields() {
        let ctx = WorkflowTraceContext::from_traceparent(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        )
        .unwrap();
        let json: serde_json::Value = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["trace_id"], "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(json["span_id"], "00f067aa0ba902b7");
    }
}
