//! The [`UserStore`] trait — async storage abstraction for users.

use uuid::Uuid;

use crate::entities::{NewUser, User};
use crate::store::StoreFuture;

/// Async storage abstraction for users.
///
/// All methods return a [`StoreFuture`] (boxed future) for object safety,
/// allowing the store to be used as `Arc<dyn UserStore>`.
pub trait UserStore: Send + Sync {
    /// Create a new user.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::DuplicateEmail`] or [`StoreError::DuplicateUsername`]
    /// if the email or username is already taken.
    fn create_user(&self, req: NewUser) -> StoreFuture<'_, User>;

    /// Find a user by email. Returns `None` if not found.
    fn find_user_by_email(&self, email: &str) -> StoreFuture<'_, Option<User>>;

    /// Find a user by ID. Returns `None` if not found.
    fn find_user_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<User>>;
}
