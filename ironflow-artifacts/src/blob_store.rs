//! The [`BlobStore`] trait -- async blob storage for artifact payloads.
//!
//! A blob store holds the *bytes* of an artifact. Its metadata (name, size,
//! hash, owning step) lives in the run store, which is the source of truth:
//! a blob with no metadata row is never served.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use futures_util::Stream;

use crate::error::ArtifactError;

/// Boxed future returned by [`BlobStore`] methods -- keeps the trait object safe.
pub type BlobFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ArtifactError>> + Send + 'a>>;

/// Stream of bytes, used for both upload and download.
///
/// Artifacts are never buffered whole in memory: an upload is consumed as it
/// arrives and a download is produced as it is read.
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, ArtifactError>> + Send>>;

/// What a [`BlobStore::put`] recorded about the bytes it just wrote.
///
/// # Examples
///
/// ```
/// use ironflow_artifacts::blob_store::BlobDigest;
///
/// let digest = BlobDigest {
///     size_bytes: 4,
///     sha256: "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08".to_string(),
/// };
/// assert_eq!(digest.size_bytes, 4);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobDigest {
    /// Number of bytes written.
    pub size_bytes: u64,
    /// Lowercase hex SHA-256 of the content.
    pub sha256: String,
}

/// Async blob storage for artifact payloads.
///
/// All methods return a [`BlobFuture`] so the store can be used as
/// `Arc<dyn BlobStore>`.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
///
/// use ironflow_artifacts::blob_store::BlobStore;
/// use ironflow_artifacts::local::LocalBlobStore;
/// use ironflow_artifacts::stream_from_bytes;
///
/// # async fn example() -> Result<(), ironflow_artifacts::error::ArtifactError> {
/// let store: Arc<dyn BlobStore> = Arc::new(LocalBlobStore::new("/var/lib/ironflow/artifacts"));
///
/// let digest = store
///     .put("artifacts/run/step/id", stream_from_bytes(b"hello".to_vec()))
///     .await?;
/// assert_eq!(digest.size_bytes, 5);
/// # Ok(())
/// # }
/// ```
pub trait BlobStore: Send + Sync {
    /// Write a blob under `key`, replacing any existing content.
    ///
    /// The size and SHA-256 are computed while the bytes are consumed, so the
    /// payload is never buffered whole.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError::TooLarge`] when the stream exceeds the store's
    /// configured limit, [`ArtifactError::InvalidKey`] for a key the backend
    /// refuses, and [`ArtifactError::Io`] on a storage failure.
    fn put<'a>(&'a self, key: &'a str, content: ByteStream) -> BlobFuture<'a, BlobDigest>;

    /// Open a blob for reading.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError::NotFound`] when no blob is stored under `key`,
    /// and [`ArtifactError::Io`] on a storage failure.
    fn get<'a>(&'a self, key: &'a str) -> BlobFuture<'a, ByteStream>;

    /// Delete a blob.
    ///
    /// Returns `true` when a blob existed and was removed, `false` when the key
    /// was already absent.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError::Io`] on a storage failure other than a missing key.
    fn delete<'a>(&'a self, key: &'a str) -> BlobFuture<'a, bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_digest_compares_by_value() {
        let a = BlobDigest {
            size_bytes: 1,
            sha256: "ab".to_string(),
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn blob_digest_differs_on_hash() {
        let a = BlobDigest {
            size_bytes: 1,
            sha256: "ab".to_string(),
        };
        let b = BlobDigest {
            size_bytes: 1,
            sha256: "cd".to_string(),
        };
        assert_ne!(a, b);
    }
}
