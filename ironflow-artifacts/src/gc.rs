//! Garbage collector for orphaned artifact blobs.
//!
//! A blob is orphaned when no artifact metadata record references its storage
//! key. This can happen after a crash between a blob upload and its metadata
//! insertion, or after a run purge that did not clean up shared blobs.
//!
//! The [`collect_garbage`] function compares the keys present in the blob store against a set of
//! referenced keys (provided by the caller from the metadata store) and deletes
//! the orphans, respecting a configurable grace period.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::blob_store::BlobStore;
use crate::error::ArtifactError;

/// Result of a single GC tick.
///
/// # Examples
///
/// ```
/// use ironflow_artifacts::gc::GcReport;
///
/// let report = GcReport {
///     scanned: 100,
///     orphans_found: 5,
///     deleted: 3,
///     bytes_freed: 1024,
///     skipped_grace_period: 2,
///     errors: 0,
/// };
/// assert_eq!(report.orphans_found, report.deleted + report.skipped_grace_period);
/// ```
#[derive(Debug, Clone, Default)]
pub struct GcReport {
    /// Total blob keys scanned.
    pub scanned: u64,
    /// Keys not referenced by any artifact.
    pub orphans_found: u64,
    /// Orphan blobs actually deleted.
    pub deleted: u64,
    /// Estimated bytes freed (sum of deleted blob sizes, when available).
    pub bytes_freed: u64,
    /// Orphans within the grace period, left untouched.
    pub skipped_grace_period: u64,
    /// Deletion attempts that failed.
    pub errors: u64,
}

/// Configuration for the blob garbage collector.
///
/// # Examples
///
/// ```
/// use ironflow_artifacts::gc::GcConfig;
/// use std::time::Duration;
///
/// let config = GcConfig {
///     grace_period: Duration::from_secs(7 * 86400),
///     dry_run: false,
///     prefix: "artifacts/".to_string(),
/// };
/// assert!(!config.dry_run);
/// ```
#[derive(Debug, Clone)]
pub struct GcConfig {
    /// How long an orphan must exist before it is eligible for deletion.
    pub grace_period: Duration,
    /// When `true`, log what would be deleted without actually deleting.
    pub dry_run: bool,
    /// Blob key prefix to scan.
    pub prefix: String,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            grace_period: Duration::from_secs(7 * 86400),
            dry_run: false,
            prefix: "artifacts/".to_string(),
        }
    }
}

/// Trait for resolving the creation time of a blob, used for grace-period checks.
///
/// Implementations differ by backend: local filesystem uses file mtime,
/// S3 uses the object's `LastModified` header.
pub trait BlobTimestamp: Send + Sync {
    /// Return the creation or last-modified time of a blob.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError::NotFound`] when the key does not exist,
    /// and [`ArtifactError::Io`] on any other failure.
    fn created_at(
        &self,
        key: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<DateTime<Utc>, ArtifactError>> + Send + '_>,
    >;
}

/// [`BlobTimestamp`] backed by local filesystem mtime.
///
/// # Examples
///
/// ```no_run
/// use ironflow_artifacts::gc::LocalBlobTimestamp;
/// use std::path::PathBuf;
///
/// let ts = LocalBlobTimestamp::new(PathBuf::from("/var/lib/ironflow/artifacts"));
/// ```
pub struct LocalBlobTimestamp {
    root: std::path::PathBuf,
}

impl LocalBlobTimestamp {
    /// Create a timestamp resolver rooted at the blob store's directory.
    pub fn new(root: std::path::PathBuf) -> Self {
        Self { root }
    }
}

impl BlobTimestamp for LocalBlobTimestamp {
    fn created_at(
        &self,
        key: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<DateTime<Utc>, ArtifactError>> + Send + '_>,
    > {
        let path = self.root.join(key);
        Box::pin(async move { local_mtime(&path) })
    }
}

/// Read a file's mtime as a UTC timestamp.
fn local_mtime(path: &Path) -> Result<DateTime<Utc>, ArtifactError> {
    let meta = std::fs::metadata(path).map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => ArtifactError::NotFound(path.display().to_string()),
        _ => ArtifactError::from(err),
    })?;
    let modified = meta.modified().map_err(|err| {
        ArtifactError::Io(format!("failed to read mtime of {}: {err}", path.display()))
    })?;
    Ok(DateTime::<Utc>::from(modified))
}

/// Run one GC pass: find orphaned blobs and delete them.
///
/// `referenced_keys` is the set of storage keys currently referenced by at
/// least one artifact record. Keys present in the blob store but absent from
/// this set are orphans.
///
/// # Errors
///
/// Individual deletion errors are counted in the report; this function only
/// returns `Err` when the initial key listing fails.
///
/// # Examples
///
/// ```no_run
/// use std::collections::HashSet;
/// use ironflow_artifacts::gc::{GcConfig, LocalBlobTimestamp, collect_garbage};
/// use ironflow_artifacts::local::LocalBlobStore;
/// use std::path::PathBuf;
///
/// # async fn example() -> Result<(), ironflow_artifacts::error::ArtifactError> {
/// let store = LocalBlobStore::new("/var/lib/ironflow/artifacts");
/// let ts = LocalBlobTimestamp::new(PathBuf::from("/var/lib/ironflow/artifacts"));
/// let referenced: HashSet<String> = HashSet::new();
/// let config = GcConfig::default();
///
/// let report = collect_garbage(&store, &ts, &referenced, &config).await?;
/// println!("deleted {} orphans, freed {} bytes", report.deleted, report.bytes_freed);
/// # Ok(())
/// # }
/// ```
pub async fn collect_garbage(
    store: &dyn BlobStore,
    timestamps: &dyn BlobTimestamp,
    referenced_keys: &HashSet<String>,
    config: &GcConfig,
) -> Result<GcReport, ArtifactError> {
    let all_keys = store.list_keys(&config.prefix).await?;
    let now = Utc::now();
    let mut report = GcReport {
        scanned: all_keys.len() as u64,
        ..Default::default()
    };

    for key in &all_keys {
        if referenced_keys.contains(key) {
            continue;
        }

        report.orphans_found += 1;

        match timestamps.created_at(key).await {
            Ok(created) => {
                let age = now.signed_duration_since(created);
                if age < chrono::Duration::from_std(config.grace_period).unwrap_or_default() {
                    report.skipped_grace_period += 1;
                    continue;
                }
            }
            Err(ArtifactError::NotFound(_)) => continue,
            Err(_) => {
                report.skipped_grace_period += 1;
                continue;
            }
        }

        if config.dry_run {
            report.deleted += 1;
            continue;
        }

        match store.delete(key).await {
            Ok(true) => report.deleted += 1,
            Ok(false) => {}
            Err(_) => report.errors += 1,
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::time::Duration;

    use tempfile::TempDir;

    use crate::blob_store::BlobStore;
    use crate::local::LocalBlobStore;
    use crate::stream_from_bytes;

    use super::*;

    fn store_and_ts() -> (TempDir, LocalBlobStore, LocalBlobTimestamp) {
        let dir = TempDir::new().expect("temp dir");
        let store = LocalBlobStore::new(dir.path());
        let ts = LocalBlobTimestamp::new(dir.path().to_path_buf());
        (dir, store, ts)
    }

    #[tokio::test]
    async fn gc_deletes_orphan_blob() {
        let (_dir, store, ts) = store_and_ts();
        store
            .put("artifacts/a/b/orphan", stream_from_bytes(b"dead".to_vec()))
            .await
            .expect("put");

        let referenced = HashSet::new();
        let config = GcConfig {
            grace_period: Duration::ZERO,
            dry_run: false,
            prefix: "artifacts/".to_string(),
        };

        let report = collect_garbage(&store, &ts, &referenced, &config)
            .await
            .expect("gc");

        assert_eq!(report.scanned, 1);
        assert_eq!(report.orphans_found, 1);
        assert_eq!(report.deleted, 1);

        let keys = store.list_keys("artifacts/").await.expect("list");
        assert!(keys.is_empty(), "orphan should be deleted");
    }

    #[tokio::test]
    async fn gc_does_not_delete_referenced_blob() {
        let (_dir, store, ts) = store_and_ts();
        let key = "artifacts/a/b/referenced";
        store
            .put(key, stream_from_bytes(b"alive".to_vec()))
            .await
            .expect("put");

        let mut referenced = HashSet::new();
        referenced.insert(key.to_string());

        let config = GcConfig {
            grace_period: Duration::ZERO,
            dry_run: false,
            prefix: "artifacts/".to_string(),
        };

        let report = collect_garbage(&store, &ts, &referenced, &config)
            .await
            .expect("gc");

        assert_eq!(report.scanned, 1);
        assert_eq!(report.orphans_found, 0);
        assert_eq!(report.deleted, 0);

        let keys = store.list_keys("artifacts/").await.expect("list");
        assert_eq!(keys.len(), 1, "referenced blob should survive");
    }

    #[tokio::test]
    async fn gc_dry_run_does_not_delete() {
        let (_dir, store, ts) = store_and_ts();
        store
            .put("artifacts/a/b/orphan", stream_from_bytes(b"data".to_vec()))
            .await
            .expect("put");

        let referenced = HashSet::new();
        let config = GcConfig {
            grace_period: Duration::ZERO,
            dry_run: true,
            prefix: "artifacts/".to_string(),
        };

        let report = collect_garbage(&store, &ts, &referenced, &config)
            .await
            .expect("gc");

        assert_eq!(report.orphans_found, 1);
        assert_eq!(report.deleted, 1); // counted as "would delete" in dry-run

        let keys = store.list_keys("artifacts/").await.expect("list");
        assert_eq!(keys.len(), 1, "dry-run should not delete");
    }

    #[tokio::test]
    async fn gc_respects_grace_period() {
        let (_dir, store, ts) = store_and_ts();
        store
            .put("artifacts/a/b/recent", stream_from_bytes(b"new".to_vec()))
            .await
            .expect("put");

        let referenced = HashSet::new();
        let config = GcConfig {
            grace_period: Duration::from_secs(86400),
            dry_run: false,
            prefix: "artifacts/".to_string(),
        };

        let report = collect_garbage(&store, &ts, &referenced, &config)
            .await
            .expect("gc");

        assert_eq!(report.orphans_found, 1);
        assert_eq!(report.skipped_grace_period, 1);
        assert_eq!(report.deleted, 0);

        let keys = store.list_keys("artifacts/").await.expect("list");
        assert_eq!(keys.len(), 1, "recent orphan should survive grace period");
    }
}
