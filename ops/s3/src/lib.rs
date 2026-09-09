//! AWS S3 integration for Ironflow workflows.
//!
//! This crate provides a thin integration layer between the
//! [`aws-sdk-s3`](https://crates.io/crates/aws-sdk-s3) crate and Ironflow's
//! workflow engine. Each S3 API call is exposed as an
//! [`Operation`](ironflow_core::operation::Operation) that can be executed as a
//! tracked workflow step via `WorkflowContext::operation()`.
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_ops_s3::S3Client;
//! use ironflow_core::operation::{OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//! let s3 = S3Client::from_context(&ctx).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Object operations
//!
//! ```no_run
//! use ironflow_ops_s3::{S3Client, objects::{PutObject, GetObject}};
//! use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//!
//! // Upload
//! let put = PutObject::new(&s3, "my-bucket", "hello.txt", b"hello world".to_vec());
//! let result = put.execute(&ctx).await?;
//!
//! // Download (typed -- includes body bytes)
//! let get = GetObject::new(&s3, "my-bucket", "hello.txt");
//! let output = get.run().await?;
//! assert_eq!(output.body, b"hello world");
//! # Ok(())
//! # }
//! ```
//!
//! # Presigned URLs
//!
//! ```no_run
//! use ironflow_ops_s3::{S3Client, presigned::PresignGetObject};
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
//! let presign = PresignGetObject::new(&s3, "my-bucket", "file.pdf", 3600);
//! let output = presign.run().await?;
//! println!("Download URL: {}", output.url);
//! # Ok(())
//! # }
//! ```

pub mod acl;
pub mod bucket_config;
pub mod buckets;
mod client;
pub(crate) mod error;
pub mod multipart;
pub mod objects;
pub mod presigned;
pub mod tagging;

pub use client::S3Client;

#[cfg(test)]
mod tests {
    use ironflow_core::operation::Operation;

    use crate::S3Client;

    fn s3() -> S3Client {
        S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None).unwrap()
    }

    #[test]
    fn all_object_ops_return_kind_s3() {
        use crate::objects::*;
        let s3 = s3();

        let ops: Vec<Box<dyn Operation>> = vec![
            Box::new(PutObject::new(&s3, "b", "k", vec![])),
            Box::new(GetObject::new(&s3, "b", "k")),
            Box::new(HeadObject::new(&s3, "b", "k")),
            Box::new(DeleteObject::new(&s3, "b", "k")),
            Box::new(DeleteObjects::new(&s3, "b", vec!["k".into()])),
            Box::new(CopyObject::new(&s3, "b", "k", "b2", "k2")),
            Box::new(ListObjects::new(&s3, "b")),
        ];

        for op in &ops {
            assert_eq!(op.kind(), "s3", "operation returned wrong kind");
        }
    }

    #[test]
    fn all_bucket_ops_return_kind_s3() {
        use crate::buckets::*;
        let s3 = s3();

        let ops: Vec<Box<dyn Operation>> = vec![
            Box::new(CreateBucket::new(&s3, "b", None)),
            Box::new(DeleteBucket::new(&s3, "b")),
            Box::new(ListBuckets::new(&s3)),
            Box::new(HeadBucket::new(&s3, "b")),
            Box::new(GetBucketLocation::new(&s3, "b")),
        ];

        for op in &ops {
            assert_eq!(op.kind(), "s3");
        }
    }

    #[test]
    fn all_other_ops_return_kind_s3() {
        use crate::acl::*;
        use crate::multipart::*;
        use crate::presigned::*;
        use crate::tagging::*;
        let s3 = s3();

        let ops: Vec<Box<dyn Operation>> = vec![
            Box::new(GetObjectTagging::new(&s3, "b", "k")),
            Box::new(PutObjectTagging::new(&s3, "b", "k", vec![])),
            Box::new(DeleteObjectTagging::new(&s3, "b", "k")),
            Box::new(GetObjectAcl::new(&s3, "b", "k")),
            Box::new(PutObjectAcl::new(&s3, "b", "k", "private")),
            Box::new(RestoreObject::new(&s3, "b", "k", 1)),
            Box::new(CreateMultipartUpload::new(&s3, "b", "k")),
            Box::new(UploadPart::new(&s3, "b", "k", "uid", 1, vec![])),
            Box::new(CompleteMultipartUpload::new(&s3, "b", "k", "uid", vec![])),
            Box::new(AbortMultipartUpload::new(&s3, "b", "k", "uid")),
            Box::new(ListMultipartUploads::new(&s3, "b")),
            Box::new(ListParts::new(&s3, "b", "k", "uid")),
            Box::new(PresignGetObject::new(&s3, "b", "k", 3600)),
            Box::new(PresignPutObject::new(&s3, "b", "k", 3600)),
        ];

        for op in &ops {
            assert_eq!(op.kind(), "s3");
        }
    }

    #[test]
    fn no_operation_leaks_secrets_in_input() {
        use crate::objects::*;
        let s3 = s3();

        let ops: Vec<Box<dyn Operation>> = vec![
            Box::new(PutObject::new(&s3, "b", "k", b"data".to_vec())),
            Box::new(GetObject::new(&s3, "b", "k")),
            Box::new(HeadObject::new(&s3, "b", "k")),
            Box::new(DeleteObject::new(&s3, "b", "k")),
            Box::new(ListObjects::new(&s3, "b")),
        ];

        for op in &ops {
            if let Some(input) = op.input() {
                let input_str = input.to_string();
                assert!(
                    !input_str.contains("AKID"),
                    "input leaked access key: {input_str}"
                );
                assert!(
                    !input_str.contains("SECRET"),
                    "input leaked secret key: {input_str}"
                );
            }
        }
    }
}
