//! [`LocalBlobStore`] -- artifact blobs on the local filesystem.
//!
//! Suitable for development, CI, single-node deployments, and any topology
//! where the API replicas share a volume. In a multi-node deployment without a
//! shared filesystem, a download can land on a replica that does not hold the
//! blob: use an object-storage backend there.

use std::path::{Path, PathBuf};

use futures_util::stream::StreamExt;
use sha2::{Digest, Sha256};
use tokio::fs::{File, create_dir_all, remove_file, rename};
use tokio::io::AsyncWriteExt;
use tracing::warn;
use uuid::Uuid;

use crate::blob_store::{BlobDigest, BlobFuture, BlobStore, ByteStream};
use crate::error::ArtifactError;
use crate::stream_from_file;

/// Default per-artifact size limit: 100 MiB.
pub const DEFAULT_MAX_ARTIFACT_BYTES: u64 = 100 * 1024 * 1024;

/// Filesystem-backed [`BlobStore`].
///
/// Blobs are written to a temporary file in the destination directory and
/// renamed into place, so a reader never observes a partially written blob.
///
/// # Examples
///
/// ```no_run
/// use ironflow_artifacts::blob_store::BlobStore;
/// use ironflow_artifacts::local::LocalBlobStore;
/// use ironflow_artifacts::stream_from_bytes;
///
/// # async fn example() -> Result<(), ironflow_artifacts::error::ArtifactError> {
/// let store = LocalBlobStore::new("/var/lib/ironflow/artifacts").max_bytes(10 * 1024 * 1024);
///
/// store
///     .put("artifacts/run/step/id", stream_from_bytes(b"report".to_vec()))
///     .await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct LocalBlobStore {
    root: PathBuf,
    max_bytes: u64,
}

impl LocalBlobStore {
    /// Create a store rooted at `root`, with the default size limit.
    ///
    /// The directory is created lazily on the first write.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_artifacts::local::{DEFAULT_MAX_ARTIFACT_BYTES, LocalBlobStore};
    ///
    /// let store = LocalBlobStore::new("/tmp/ironflow-artifacts");
    /// assert_eq!(store.max_bytes_limit(), DEFAULT_MAX_ARTIFACT_BYTES);
    /// ```
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            max_bytes: DEFAULT_MAX_ARTIFACT_BYTES,
        }
    }

    /// Override the per-artifact size limit.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_artifacts::local::LocalBlobStore;
    ///
    /// let store = LocalBlobStore::new("/tmp/a").max_bytes(1024);
    /// assert_eq!(store.max_bytes_limit(), 1024);
    /// ```
    pub fn max_bytes(mut self, limit: u64) -> Self {
        self.max_bytes = limit;
        self
    }

    /// The configured per-artifact size limit, in bytes.
    pub fn max_bytes_limit(&self) -> u64 {
        self.max_bytes
    }

    /// The filesystem root holding the blobs.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Map a storage key to an absolute path under [`root`](Self::root).
    ///
    /// Keys are produced by [`storage_key`](crate::name::storage_key) and only
    /// ever contain UUIDs; this check is defence in depth against a caller that
    /// builds one by hand.
    fn resolve(&self, key: &str) -> Result<PathBuf, ArtifactError> {
        let reject = |reason: &'static str| {
            Err(ArtifactError::InvalidKey {
                key: key.to_string(),
                reason,
            })
        };

        if key.is_empty() {
            return reject("must not be empty");
        }
        if key.contains('\\') || key.contains('\0') || key.contains(':') {
            return reject("contains a forbidden character");
        }
        if key.starts_with('/') {
            return reject("must be relative");
        }

        let mut path = self.root.clone();
        for component in key.split('/') {
            if component.is_empty() || component == "." || component == ".." {
                return reject("contains an empty or relative path component");
            }
            path.push(component);
        }

        Ok(path)
    }
}

impl BlobStore for LocalBlobStore {
    fn put<'a>(&'a self, key: &'a str, mut content: ByteStream) -> BlobFuture<'a, BlobDigest> {
        Box::pin(async move {
            let path = self.resolve(key)?;
            let parent = path
                .parent()
                .ok_or_else(|| ArtifactError::InvalidKey {
                    key: key.to_string(),
                    reason: "has no parent directory",
                })?
                .to_path_buf();
            create_dir_all(&parent).await?;

            let temp_path = parent.join(format!(".{}.part", Uuid::now_v7()));
            let mut file = File::create(&temp_path).await?;

            let mut hasher = Sha256::new();
            let mut size_bytes: u64 = 0;

            while let Some(chunk) = content.next().await {
                let outcome = match chunk {
                    Ok(bytes) => {
                        size_bytes += bytes.len() as u64;
                        if size_bytes > self.max_bytes {
                            Err(ArtifactError::TooLarge {
                                limit_bytes: self.max_bytes,
                            })
                        } else {
                            hasher.update(&bytes);
                            file.write_all(&bytes).await.map_err(ArtifactError::from)
                        }
                    }
                    Err(err) => Err(err),
                };

                if let Err(err) = outcome {
                    discard(&temp_path).await;
                    return Err(err);
                }
            }

            if let Err(err) = file.flush().await {
                discard(&temp_path).await;
                return Err(err.into());
            }
            drop(file);

            if let Err(err) = rename(&temp_path, &path).await {
                discard(&temp_path).await;
                return Err(err.into());
            }

            Ok(BlobDigest {
                size_bytes,
                sha256: hex::encode(hasher.finalize()),
            })
        })
    }

    fn get<'a>(&'a self, key: &'a str) -> BlobFuture<'a, ByteStream> {
        Box::pin(async move {
            let path = self.resolve(key)?;
            let file = File::open(&path).await.map_err(|err| match err.kind() {
                std::io::ErrorKind::NotFound => ArtifactError::NotFound(key.to_string()),
                _ => ArtifactError::from(err),
            })?;

            Ok(stream_from_file(file))
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> BlobFuture<'a, bool> {
        Box::pin(async move {
            let path = self.resolve(key)?;
            match remove_file(&path).await {
                Ok(()) => Ok(true),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(err) => Err(err.into()),
            }
        })
    }
}

/// Remove a partially written temporary file, logging rather than failing.
///
/// The caller is already returning an error; a leftover `.part` file is noise,
/// not a second failure to propagate.
async fn discard(temp_path: &Path) {
    if let Err(err) = remove_file(temp_path).await
        && err.kind() != std::io::ErrorKind::NotFound
    {
        warn!(
            path = %temp_path.display(),
            error = %err,
            "failed to remove partial artifact file"
        );
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures_util::TryStreamExt;
    use tempfile::TempDir;

    use super::*;
    use crate::stream_from_bytes;

    /// SHA-256 of the empty input, per FIPS 180-4.
    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn store() -> (TempDir, LocalBlobStore) {
        let dir = TempDir::new().expect("temp dir");
        let store = LocalBlobStore::new(dir.path());
        (dir, store)
    }

    async fn read_all(store: &LocalBlobStore, key: &str) -> Result<Vec<u8>, ArtifactError> {
        let chunks: Vec<Bytes> = store.get(key).await?.try_collect().await?;
        Ok(chunks.concat())
    }

    #[tokio::test]
    async fn put_then_get_roundtrips_the_content() {
        let (_dir, store) = store();

        let digest = store
            .put("artifacts/a/b/c", stream_from_bytes(b"hello".to_vec()))
            .await
            .expect("put");

        assert_eq!(digest.size_bytes, 5);
        assert_eq!(read_all(&store, "artifacts/a/b/c").await.unwrap(), b"hello");
    }

    #[tokio::test]
    async fn put_computes_the_sha256() {
        let (_dir, store) = store();

        let digest = store
            .put("artifacts/a/b/c", stream_from_bytes(b"abc".to_vec()))
            .await
            .expect("put");

        assert_eq!(
            digest.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[tokio::test]
    async fn put_accepts_an_empty_blob() {
        let (_dir, store) = store();

        let digest = store
            .put("artifacts/a/b/empty", stream_from_bytes(Vec::new()))
            .await
            .expect("put");

        assert_eq!(digest.size_bytes, 0);
        assert_eq!(digest.sha256, EMPTY_SHA256);
        assert!(
            read_all(&store, "artifacts/a/b/empty")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn put_handles_binary_content_larger_than_one_chunk() {
        let (_dir, store) = store();
        let payload: Vec<u8> = (0..200_000u32).map(|i| (i % 256) as u8).collect();

        let digest = store
            .put("artifacts/a/b/bin", stream_from_bytes(payload.clone()))
            .await
            .expect("put");

        assert_eq!(digest.size_bytes, payload.len() as u64);
        assert_eq!(
            read_all(&store, "artifacts/a/b/bin").await.unwrap(),
            payload
        );
    }

    #[tokio::test]
    async fn put_overwrites_an_existing_blob() {
        let (_dir, store) = store();

        store
            .put("artifacts/a/b/c", stream_from_bytes(b"first".to_vec()))
            .await
            .expect("first put");
        store
            .put("artifacts/a/b/c", stream_from_bytes(b"second".to_vec()))
            .await
            .expect("second put");

        assert_eq!(
            read_all(&store, "artifacts/a/b/c").await.unwrap(),
            b"second"
        );
    }

    #[tokio::test]
    async fn put_rejects_a_payload_over_the_limit_and_leaves_nothing_behind() {
        let (dir, store) = store();
        let store = store.max_bytes(4);

        let err = store
            .put("artifacts/a/b/c", stream_from_bytes(b"toolong".to_vec()))
            .await
            .expect_err("should exceed the limit");

        assert!(matches!(err, ArtifactError::TooLarge { limit_bytes: 4 }));
        assert!(matches!(
            read_all(&store, "artifacts/a/b/c").await,
            Err(ArtifactError::NotFound(_))
        ));

        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("artifacts/a/b"))
            .expect("read dir")
            .filter_map(Result::ok)
            .collect();
        assert!(leftovers.is_empty(), "partial file was not cleaned up");
    }

    #[tokio::test]
    async fn get_on_an_unknown_key_reports_not_found() {
        let (_dir, store) = store();

        let err = read_all(&store, "artifacts/nope/nope/nope")
            .await
            .expect_err("should not exist");

        assert!(matches!(err, ArtifactError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_reports_whether_the_blob_existed() {
        let (_dir, store) = store();
        store
            .put("artifacts/a/b/c", stream_from_bytes(b"x".to_vec()))
            .await
            .expect("put");

        assert!(store.delete("artifacts/a/b/c").await.expect("delete"));
        assert!(!store.delete("artifacts/a/b/c").await.expect("delete again"));
    }

    #[tokio::test]
    async fn keys_escaping_the_root_are_rejected() {
        let (_dir, store) = store();

        for key in [
            "../outside",
            "artifacts/../../outside",
            "/etc/passwd",
            "artifacts\\a\\b",
            "artifacts/./a",
            "",
        ] {
            assert!(
                matches!(
                    store.put(key, stream_from_bytes(b"x".to_vec())).await,
                    Err(ArtifactError::InvalidKey { .. })
                ),
                "key {key:?} should have been rejected"
            );
        }
    }

    #[tokio::test]
    async fn a_rejected_key_never_reaches_the_filesystem() {
        let (dir, store) = store();

        let _ = store
            .put("../escaped", stream_from_bytes(b"x".to_vec()))
            .await;

        let parent = dir.path().parent().expect("temp dir has a parent");
        assert!(!parent.join("escaped").exists());
    }
}
