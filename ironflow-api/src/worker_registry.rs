//! In-memory record of the workers that recently asked for a run.
//!
//! Every call to the internal pick route carrying a `worker_id` records the
//! capabilities the worker advertised. The run detail route reads it back to
//! tell whether a pending run can be taken by any worker seen recently, so a
//! run whose required tags no worker carries is flagged instead of waiting
//! in silence.
//!
//! The record lives in the API process: with several API replicas, each one
//! only knows the workers that polled it.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, Instant};

use ironflow_store::entities::WorkerCapabilities;

use crate::entities::WorkerRouting;

/// How long a worker counts as seen after its last pick request.
///
/// Workers poll every few seconds while idle; a worker busy on long runs
/// still polls whenever a slot frees up.
pub const WORKER_SEEN_TTL: Duration = Duration::from_secs(5 * 60);

/// A worker as last seen by the pick route.
#[derive(Debug, Clone)]
struct SeenWorker {
    /// `None` for a worker that predates routing: it takes every run.
    capabilities: Option<WorkerCapabilities>,
    last_seen: Instant,
}

/// Workers seen by the pick route within [`WORKER_SEEN_TTL`].
///
/// Cheap to clone: clones share the same record.
///
/// # Examples
///
/// ```
/// use ironflow_api::worker_registry::WorkerRegistry;
/// use ironflow_store::entities::WorkerCapabilities;
///
/// let registry = WorkerRegistry::new();
/// registry.record("worker-1", Some(WorkerCapabilities::new(None, vec!["gpu".to_string()])));
///
/// let routing = registry.routing_for("transcode", &["gpu".to_string()]);
/// assert_eq!(routing.seen_workers, 1);
/// assert_eq!(routing.eligible_workers, 1);
/// ```
#[derive(Debug, Clone)]
pub struct WorkerRegistry {
    seen: Arc<RwLock<HashMap<String, SeenWorker>>>,
    ttl: Duration,
}

impl Default for WorkerRegistry {
    fn default() -> Self {
        Self::with_ttl(WORKER_SEEN_TTL)
    }
}

impl WorkerRegistry {
    /// Create an empty registry using [`WORKER_SEEN_TTL`].
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_api::worker_registry::WorkerRegistry;
    ///
    /// let registry = WorkerRegistry::new();
    /// assert_eq!(registry.routing_for("deploy", &[]).seen_workers, 0);
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an empty registry where a worker counts as seen for `ttl`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use ironflow_api::worker_registry::WorkerRegistry;
    ///
    /// let registry = WorkerRegistry::with_ttl(Duration::from_secs(30));
    /// registry.record("worker-1", None);
    /// assert_eq!(registry.routing_for("deploy", &[]).seen_workers, 1);
    /// ```
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            seen: Arc::new(RwLock::new(HashMap::new())),
            ttl,
        }
    }

    /// Record that `worker_id` asked for a run with these capabilities.
    ///
    /// Replaces what was known about that worker, and drops the workers not
    /// seen within the TTL.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_api::worker_registry::WorkerRegistry;
    ///
    /// let registry = WorkerRegistry::new();
    /// registry.record("worker-1", None);
    /// registry.record("worker-1", None);
    /// assert_eq!(registry.routing_for("deploy", &[]).seen_workers, 1);
    /// ```
    pub fn record(&self, worker_id: &str, capabilities: Option<WorkerCapabilities>) {
        let now = Instant::now();
        // The record is a cache: a writer that panicked leaves nothing worth
        // refusing to read.
        let mut workers = self.seen.write().unwrap_or_else(PoisonError::into_inner);
        workers.retain(|_, worker| now.duration_since(worker.last_seen) < self.ttl);
        workers.insert(
            worker_id.to_string(),
            SeenWorker {
                capabilities,
                last_seen: now,
            },
        );
    }

    /// Count the workers seen within the TTL, and those that could take a run
    /// of `workflow_name` requiring `required_tags`.
    ///
    /// A worker without capabilities (an older release) can take any run.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_api::worker_registry::WorkerRegistry;
    /// use ironflow_store::entities::WorkerCapabilities;
    ///
    /// let registry = WorkerRegistry::new();
    /// registry.record("cpu-1", Some(WorkerCapabilities::new(None, Vec::new())));
    ///
    /// let routing = registry.routing_for("transcode", &["gpu".to_string()]);
    /// assert_eq!(routing.seen_workers, 1);
    /// assert_eq!(routing.eligible_workers, 0);
    /// ```
    pub fn routing_for(&self, workflow_name: &str, required_tags: &[String]) -> WorkerRouting {
        let now = Instant::now();
        let workers = self.seen.read().unwrap_or_else(PoisonError::into_inner);
        let seen: Vec<&SeenWorker> = workers
            .values()
            .filter(|seen| now.duration_since(seen.last_seen) < self.ttl)
            .collect();
        let eligible = seen
            .iter()
            .filter(|seen| {
                seen.capabilities
                    .as_ref()
                    .is_none_or(|caps| caps.can_take(workflow_name, required_tags))
            })
            .count();

        WorkerRouting {
            seen_workers: saturating_u32(seen.len()),
            eligible_workers: saturating_u32(eligible),
        }
    }
}

fn saturating_u32(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::thread::sleep;

    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    fn caps(workflows: Option<&[&str]>, tags: &[&str]) -> Option<WorkerCapabilities> {
        Some(WorkerCapabilities::new(
            workflows.map(strings),
            strings(tags),
        ))
    }

    #[test]
    fn empty_registry_has_no_worker() {
        let routing = WorkerRegistry::new().routing_for("deploy", &[]);
        assert_eq!(
            routing,
            WorkerRouting {
                seen_workers: 0,
                eligible_workers: 0
            }
        );
    }

    #[test]
    fn worker_without_required_tag_is_seen_but_not_eligible() {
        let registry = WorkerRegistry::new();
        registry.record("cpu-1", caps(None, &["arm"]));
        registry.record("gpu-1", caps(None, &["gpu"]));

        let routing = registry.routing_for("transcode", &strings(&["gpu"]));
        assert_eq!(routing.seen_workers, 2);
        assert_eq!(routing.eligible_workers, 1);
    }

    #[test]
    fn worker_without_the_workflow_is_not_eligible() {
        let registry = WorkerRegistry::new();
        registry.record("worker-1", caps(Some(&["deploy"]), &[]));

        assert_eq!(registry.routing_for("deploy", &[]).eligible_workers, 1);
        assert_eq!(registry.routing_for("build", &[]).eligible_workers, 0);
    }

    #[test]
    fn legacy_worker_is_eligible_for_everything() {
        let registry = WorkerRegistry::new();
        registry.record("legacy", None);

        let routing = registry.routing_for("transcode", &strings(&["gpu"]));
        assert_eq!(routing.seen_workers, 1);
        assert_eq!(routing.eligible_workers, 1);
    }

    #[test]
    fn record_replaces_previous_capabilities() {
        let registry = WorkerRegistry::new();
        registry.record("worker-1", caps(None, &["gpu"]));
        registry.record("worker-1", caps(None, &[]));

        let routing = registry.routing_for("transcode", &strings(&["gpu"]));
        assert_eq!(routing.seen_workers, 1);
        assert_eq!(routing.eligible_workers, 0);
    }

    #[test]
    fn workers_not_seen_within_ttl_are_ignored_and_pruned() {
        let registry = WorkerRegistry::with_ttl(Duration::from_millis(20));
        registry.record("old", None);
        sleep(Duration::from_millis(40));

        assert_eq!(registry.routing_for("deploy", &[]).seen_workers, 0);

        registry.record("new", None);
        let workers = registry.seen.read().unwrap();
        assert_eq!(workers.len(), 1);
        assert!(workers.contains_key("new"));
    }

    #[test]
    fn clones_share_the_record() {
        let registry = WorkerRegistry::new();
        let clone = registry.clone();
        clone.record("worker-1", None);
        assert_eq!(registry.routing_for("deploy", &[]).seen_workers, 1);
    }
}
