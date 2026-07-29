//! # ironflow-artifacts
//!
//! Blob storage for **ironflow** workflow artifacts: the bytes a step produces
//! and a later step consumes.
//!
//! This crate owns the payloads only. Their metadata (name, MIME type, size,
//! SHA-256, owning step) lives in `ironflow-store` and is the source of truth:
//! a blob without a metadata row is never listed and never served.
//!
//! # Implementations
//!
//! | Store | Feature | Description |
//! |-------|---------|-------------|
//! | [`LocalBlobStore`](local::LocalBlobStore) | `artifact-local` (default) | Local filesystem, for development, CI and shared-volume deployments. |
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_artifacts::prelude::*;
//!
//! # async fn example() -> Result<(), ArtifactError> {
//! let store = LocalBlobStore::new("/var/lib/ironflow/artifacts");
//!
//! let key = storage_key(uuid::Uuid::now_v7(), uuid::Uuid::now_v7(), uuid::Uuid::now_v7());
//! let digest = store.put(&key, stream_from_bytes(b"report".to_vec())).await?;
//!
//! println!("{} bytes, sha256 {}", digest.size_bytes, digest.sha256);
//! # Ok(())
//! # }
//! ```

pub mod blob_store;
pub mod error;
pub mod name;

#[cfg(feature = "artifact-local")]
pub mod local;

use std::path::Path;

use bytes::Bytes;
use futures_util::stream::{once, unfold};
use tokio::fs::File;
use tokio::io::AsyncReadExt;

use crate::blob_store::ByteStream;
use crate::error::ArtifactError;

/// Size of the chunks yielded when reading a file.
const READ_CHUNK_BYTES: usize = 64 * 1024;

/// Wrap an in-memory buffer as a [`ByteStream`].
///
/// Convenience for callers that already hold the whole payload -- custom
/// operations, tests, small generated files. Large payloads should be streamed
/// from their source instead.
///
/// # Examples
///
/// ```
/// use ironflow_artifacts::stream_from_bytes;
///
/// let stream = stream_from_bytes(b"hello".to_vec());
/// # drop(stream);
/// ```
pub fn stream_from_bytes(bytes: impl Into<Bytes>) -> ByteStream {
    let bytes = bytes.into();
    Box::pin(once(async move { Ok(bytes) }))
}

/// Read an open file as a [`ByteStream`], in fixed-size chunks.
///
/// # Examples
///
/// ```no_run
/// use ironflow_artifacts::stream_from_file;
/// use tokio::fs::File;
///
/// # async fn example() -> Result<(), ironflow_artifacts::error::ArtifactError> {
/// let file = File::open("report.html").await?;
/// let stream = stream_from_file(file);
/// # drop(stream);
/// # Ok(())
/// # }
/// ```
pub fn stream_from_file(file: File) -> ByteStream {
    Box::pin(unfold(Some(file), |state| async move {
        let mut file = state?;
        let mut buf = vec![0u8; READ_CHUNK_BYTES];
        match file.read(&mut buf).await {
            Ok(0) => None,
            Ok(read) => {
                buf.truncate(read);
                Some((Ok(Bytes::from(buf)), Some(file)))
            }
            Err(err) => Some((Err(ArtifactError::from(err)), None)),
        }
    }))
}

/// Open a file and read it as a [`ByteStream`].
///
/// # Errors
///
/// Returns [`ArtifactError::NotFound`] when the path does not exist, and
/// [`ArtifactError::Io`] on any other filesystem failure.
///
/// # Examples
///
/// ```no_run
/// use ironflow_artifacts::stream_from_path;
///
/// # async fn example() -> Result<(), ironflow_artifacts::error::ArtifactError> {
/// let stream = stream_from_path("target/report.html").await?;
/// # drop(stream);
/// # Ok(())
/// # }
/// ```
pub async fn stream_from_path(path: impl AsRef<Path>) -> Result<ByteStream, ArtifactError> {
    let path = path.as_ref();
    let file = File::open(path).await.map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => ArtifactError::NotFound(path.display().to_string()),
        _ => ArtifactError::from(err),
    })?;

    Ok(stream_from_file(file))
}

/// Convenience re-exports for common usage.
pub mod prelude {
    pub use crate::blob_store::{BlobDigest, BlobFuture, BlobStore, ByteStream};
    pub use crate::error::ArtifactError;
    pub use crate::name::{
        MAX_ARTIFACT_NAME_LEN, guess_content_type, storage_key, validate_artifact_name,
    };
    pub use crate::{stream_from_bytes, stream_from_file, stream_from_path};

    #[cfg(feature = "artifact-local")]
    pub use crate::local::{DEFAULT_MAX_ARTIFACT_BYTES, LocalBlobStore};
}

#[cfg(test)]
mod tests {
    use futures_util::TryStreamExt;

    use super::*;

    #[tokio::test]
    async fn stream_from_bytes_yields_the_whole_buffer() {
        let chunks: Vec<Bytes> = stream_from_bytes(b"hello".to_vec())
            .try_collect()
            .await
            .expect("collect");

        assert_eq!(chunks.concat(), b"hello");
    }

    #[tokio::test]
    async fn stream_from_bytes_supports_an_empty_buffer() {
        let chunks: Vec<Bytes> = stream_from_bytes(Vec::new())
            .try_collect()
            .await
            .expect("collect");

        assert!(chunks.concat().is_empty());
    }

    #[tokio::test]
    async fn stream_from_path_reads_a_file_larger_than_one_chunk() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let path = dir.path().join("big.bin");
        let payload: Vec<u8> = (0..200_000u32).map(|i| (i % 256) as u8).collect();
        std::fs::write(&path, &payload).expect("write");

        let chunks: Vec<Bytes> = stream_from_path(&path)
            .await
            .expect("open")
            .try_collect()
            .await
            .expect("collect");

        assert_eq!(chunks.concat(), payload);
        assert!(chunks.len() > 1, "large file should stream in chunks");
    }

    #[tokio::test]
    async fn stream_from_path_reports_a_missing_file_as_not_found() {
        let result = stream_from_path("/nonexistent/ironflow/artifact").await;

        assert!(matches!(result, Err(ArtifactError::NotFound(_))));
    }
}
