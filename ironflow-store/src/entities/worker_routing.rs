//! Worker routing: the tags a run requires and the capabilities a worker
//! advertises when it asks for work.
//!
//! A run carries a list of required worker tags ([`Run::worker_tags`](super::Run::worker_tags)).
//! A worker advertises [`WorkerCapabilities`]: the workflows it registered and
//! the tags it carries. A run is only handed to a worker that
//! [can take](WorkerCapabilities::can_take) it.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Maximum length of a single worker tag, in bytes.
pub const MAX_WORKER_TAG_LEN: usize = 64;

/// Maximum number of worker tags on a run or a worker.
pub const MAX_WORKER_TAGS: usize = 32;

/// Characters accepted in a worker tag besides ASCII alphanumerics.
const EXTRA_TAG_CHARS: &[char] = &['-', '_', '.', ':', '/', '='];

/// Why a list of worker tags was refused.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::WorkerTagError;
///
/// let err = WorkerTagError::Empty;
/// assert_eq!(err.to_string(), "worker tag must not be empty");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkerTagError {
    /// A tag was empty or whitespace only.
    #[error("worker tag must not be empty")]
    Empty,
    /// A tag exceeds [`MAX_WORKER_TAG_LEN`] bytes.
    #[error("worker tag '{tag}' exceeds {max} bytes")]
    TooLong {
        /// The offending tag.
        tag: String,
        /// The maximum accepted length, in bytes.
        max: usize,
    },
    /// A tag contains a character outside ASCII alphanumerics and `- _ . : / =`.
    #[error(
        "worker tag '{tag}' contains an invalid character (allowed: ASCII letters, digits and - _ . : / =)"
    )]
    InvalidChar {
        /// The offending tag.
        tag: String,
    },
    /// More than [`MAX_WORKER_TAGS`] tags were given.
    #[error("{count} worker tags given, at most {max} are allowed")]
    TooMany {
        /// Number of tags given.
        count: usize,
        /// The maximum accepted number of tags.
        max: usize,
    },
}

/// Validate a list of worker tags.
///
/// Each tag is trimmed before the checks, so `" gpu "` is accepted and stored
/// as `"gpu"` by [`normalize_worker_tags`]. A comma is refused because it is
/// the separator used on the wire.
///
/// # Errors
///
/// Returns [`WorkerTagError::TooMany`] for more than [`MAX_WORKER_TAGS`] tags,
/// [`WorkerTagError::Empty`] for an empty or whitespace-only tag,
/// [`WorkerTagError::TooLong`] for a tag longer than [`MAX_WORKER_TAG_LEN`]
/// bytes and [`WorkerTagError::InvalidChar`] for a tag holding any other
/// character than ASCII alphanumerics and `- _ . : / =`.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::validate_worker_tags;
///
/// assert!(validate_worker_tags(&["gpu".to_string(), "region:eu".to_string()]).is_ok());
/// assert!(validate_worker_tags(&["bad,tag".to_string()]).is_err());
/// ```
pub fn validate_worker_tags(tags: &[String]) -> Result<(), WorkerTagError> {
    if tags.len() > MAX_WORKER_TAGS {
        return Err(WorkerTagError::TooMany {
            count: tags.len(),
            max: MAX_WORKER_TAGS,
        });
    }
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            return Err(WorkerTagError::Empty);
        }
        if trimmed.len() > MAX_WORKER_TAG_LEN {
            return Err(WorkerTagError::TooLong {
                tag: trimmed.to_string(),
                max: MAX_WORKER_TAG_LEN,
            });
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || EXTRA_TAG_CHARS.contains(&c))
        {
            return Err(WorkerTagError::InvalidChar {
                tag: trimmed.to_string(),
            });
        }
    }
    Ok(())
}

/// Normalize a list of worker tags: trim each tag, sort, and drop duplicates.
///
/// Empty tags left after trimming are dropped too. Call
/// [`validate_worker_tags`] first to refuse them instead.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::normalize_worker_tags;
///
/// let raw = vec![" gpu".to_string(), "arm".to_string(), "gpu".to_string()];
/// let tags = normalize_worker_tags(raw);
/// assert_eq!(tags, vec!["arm".to_string(), "gpu".to_string()]);
/// ```
pub fn normalize_worker_tags(tags: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut normalized: Vec<String> = tags
        .into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect();
    normalized.sort();
    normalized.dedup();
    normalized
}

/// What a worker can execute: the workflows it registered and the tags it carries.
///
/// Sent by a worker when it asks for a run. A worker that sends no
/// capabilities at all (an older worker) takes every run.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::WorkerCapabilities;
///
/// let caps = WorkerCapabilities::new(Some(vec!["deploy".to_string()]), vec!["gpu".to_string()]);
/// assert!(caps.can_take("deploy", &["gpu".to_string()]));
/// assert!(!caps.can_take("build", &[]));
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerCapabilities {
    /// Workflows the worker registered. `None` means any workflow.
    pub workflows: Option<Vec<String>>,
    /// Tags the worker carries. A run is only taken when every tag it
    /// requires is in this list; an empty list only takes runs requiring none.
    pub tags: Vec<String>,
}

impl WorkerCapabilities {
    /// Build worker capabilities.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::WorkerCapabilities;
    ///
    /// let caps = WorkerCapabilities::new(None, vec!["gpu".to_string()]);
    /// assert!(caps.workflows.is_none());
    /// assert_eq!(caps.tags, vec!["gpu".to_string()]);
    /// ```
    pub fn new(workflows: Option<Vec<String>>, tags: Vec<String>) -> Self {
        Self { workflows, tags }
    }

    /// Whether a worker with these capabilities may take a run of
    /// `workflow_name` requiring `required_tags`.
    ///
    /// True when the workflow list is `None` or contains `workflow_name`, and
    /// every required tag is carried by the worker.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::WorkerCapabilities;
    ///
    /// let caps = WorkerCapabilities::new(None, vec!["arm".to_string()]);
    /// assert!(caps.can_take("anything", &[]));
    /// assert!(!caps.can_take("anything", &["gpu".to_string()]));
    /// ```
    pub fn can_take(&self, workflow_name: &str, required_tags: &[String]) -> bool {
        let workflow_ok = self
            .workflows
            .as_ref()
            .is_none_or(|names| names.iter().any(|name| name == workflow_name));
        workflow_ok && required_tags.iter().all(|tag| self.tags.contains(tag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    #[test]
    fn validate_accepts_valid_tags() {
        let valid = tags(&["gpu", "region:eu", "os/linux", "a=b", "x_y.z-1"]);
        assert!(validate_worker_tags(&valid).is_ok());
    }

    #[test]
    fn validate_accepts_empty_list() {
        assert!(validate_worker_tags(&[]).is_ok());
    }

    #[test]
    fn validate_rejects_empty_tag() {
        assert_eq!(
            validate_worker_tags(&tags(&["  "])),
            Err(WorkerTagError::Empty)
        );
        assert_eq!(
            validate_worker_tags(&tags(&[""])),
            Err(WorkerTagError::Empty)
        );
    }

    #[test]
    fn validate_rejects_too_long_tag() {
        let long = "a".repeat(MAX_WORKER_TAG_LEN + 1);
        assert_eq!(
            validate_worker_tags(&tags(&[long.as_str()])),
            Err(WorkerTagError::TooLong {
                tag: long,
                max: MAX_WORKER_TAG_LEN
            })
        );
    }

    #[test]
    fn validate_accepts_tag_at_max_len() {
        let exact = "a".repeat(MAX_WORKER_TAG_LEN);
        assert!(validate_worker_tags(&tags(&[exact.as_str()])).is_ok());
    }

    #[test]
    fn validate_rejects_comma() {
        assert_eq!(
            validate_worker_tags(&tags(&["bad,tag"])),
            Err(WorkerTagError::InvalidChar {
                tag: "bad,tag".to_string()
            })
        );
    }

    #[test]
    fn validate_rejects_space_and_unicode() {
        assert!(matches!(
            validate_worker_tags(&tags(&["two words"])),
            Err(WorkerTagError::InvalidChar { .. })
        ));
        assert!(matches!(
            validate_worker_tags(&tags(&["caf\u{e9}"])),
            Err(WorkerTagError::InvalidChar { .. })
        ));
    }

    #[test]
    fn validate_trims_before_checking() {
        assert!(validate_worker_tags(&tags(&[" gpu "])).is_ok());
    }

    #[test]
    fn validate_rejects_too_many_tags() {
        let many: Vec<String> = (0..=MAX_WORKER_TAGS).map(|i| format!("t{i}")).collect();
        assert_eq!(
            validate_worker_tags(&many),
            Err(WorkerTagError::TooMany {
                count: MAX_WORKER_TAGS + 1,
                max: MAX_WORKER_TAGS
            })
        );
    }

    #[test]
    fn validate_accepts_max_tags() {
        let many: Vec<String> = (0..MAX_WORKER_TAGS).map(|i| format!("t{i}")).collect();
        assert!(validate_worker_tags(&many).is_ok());
    }

    #[test]
    fn normalize_trims_sorts_and_dedups() {
        assert_eq!(
            normalize_worker_tags(tags(&["gpu", " arm ", "gpu", "", "arm"])),
            tags(&["arm", "gpu"])
        );
    }

    #[test]
    fn normalize_empty_is_empty() {
        assert!(normalize_worker_tags(Vec::new()).is_empty());
    }

    #[test]
    fn can_take_any_workflow() {
        let caps = WorkerCapabilities::new(None, Vec::new());
        assert!(caps.can_take("deploy", &[]));
    }

    #[test]
    fn can_take_refuses_unknown_workflow() {
        let caps = WorkerCapabilities::new(Some(tags(&["deploy"])), Vec::new());
        assert!(caps.can_take("deploy", &[]));
        assert!(!caps.can_take("build", &[]));
    }

    #[test]
    fn can_take_refuses_missing_tag() {
        let caps = WorkerCapabilities::new(None, tags(&["arm"]));
        assert!(!caps.can_take("deploy", &tags(&["gpu"])));
    }

    #[test]
    fn can_take_empty_required_tags() {
        let caps = WorkerCapabilities::new(None, tags(&["gpu"]));
        assert!(caps.can_take("deploy", &[]));
    }

    #[test]
    fn can_take_superset_of_tags() {
        let caps = WorkerCapabilities::new(None, tags(&["arm", "gpu", "region:eu"]));
        assert!(caps.can_take("deploy", &tags(&["gpu", "region:eu"])));
    }

    #[test]
    fn can_take_empty_workflow_list_takes_nothing() {
        let caps = WorkerCapabilities::new(Some(Vec::new()), Vec::new());
        assert!(!caps.can_take("deploy", &[]));
    }

    #[test]
    fn error_display() {
        assert_eq!(
            WorkerTagError::TooMany { count: 40, max: 32 }.to_string(),
            "40 worker tags given, at most 32 are allowed"
        );
        assert_eq!(
            WorkerTagError::TooLong {
                tag: "x".to_string(),
                max: 64
            }
            .to_string(),
            "worker tag 'x' exceeds 64 bytes"
        );
    }
}
