//! The [`UserStore`] trait — async storage abstraction for users.

use uuid::Uuid;

use crate::entities::{NewUser, Page, User};
use crate::store::StoreFuture;

/// Async storage abstraction for users.
///
/// All methods return a [`StoreFuture`] (boxed future) for object safety,
/// allowing the store to be used as `Arc<dyn UserStore>`.
pub trait UserStore: Send + Sync {
    /// Create a new user.
    ///
    /// If this is the first user in the store, `is_admin` is automatically
    /// set to `true` regardless of the input, making them the superadmin.
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

    /// Count all users in the store.
    fn count_users(&self) -> StoreFuture<'_, u64>;

    /// List users with pagination, ordered by `created_at` descending.
    fn list_users(&self, page: u32, per_page: u32) -> StoreFuture<'_, Page<User>>;

    /// Delete a user by ID.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::UserNotFound`] if the user does not exist.
    fn delete_user(&self, id: Uuid) -> StoreFuture<'_, ()>;

    /// Update a user's admin role.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::UserNotFound`] if the user does not exist.
    fn update_user_role(&self, id: Uuid, is_admin: bool) -> StoreFuture<'_, User>;
}
