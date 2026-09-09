//! Object operations: upload, download, head, delete, copy, and list.
//!
//! Each struct implements [`Operation`](ironflow_core::operation::Operation) for
//! tracked workflow steps and provides a typed `run` method for direct use.

mod copy;
mod delete;
mod get;
mod head;
mod list;
mod put;

pub use copy::{CopyObject, CopyObjectOutput};
pub use delete::{DeleteObject, DeleteObjectError, DeleteObjects, DeleteObjectsOutput};
pub use get::{GetObject, GetObjectOutput};
pub use head::{HeadObject, HeadObjectOutput};
pub use list::{ListObjects, ListObjectsOutput, ObjectEntry};
pub use put::{PutObject, PutObjectOutput};
