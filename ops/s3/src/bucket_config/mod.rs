//! Bucket configuration operations: versioning, lifecycle, policy, CORS, encryption, notification.

mod cors;
mod encryption;
mod lifecycle;
mod notification;
mod policy;
mod versioning;

pub use cors::{DeleteBucketCors, GetBucketCors};
pub use encryption::{GetBucketEncryption, PutBucketEncryption};
pub use lifecycle::{DeleteBucketLifecycle, GetBucketLifecycle};
pub use notification::GetBucketNotification;
pub use policy::{DeleteBucketPolicy, GetBucketPolicy, PutBucketPolicy};
pub use versioning::{GetBucketVersioning, PutBucketVersioning};
