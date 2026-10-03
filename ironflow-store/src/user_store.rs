//! The [`UserStore`] trait — async storage abstraction for users.

use uuid::Uuid;

use crate::entities::{NewRefreshToken, NewUser, Page, User};
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

    /// Find a user by username. Returns `None` if not found.
    fn find_user_by_username(&self, username: &str) -> StoreFuture<'_, Option<User>>;

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
    /// Also bumps [`User::token_version`] and deletes the user's refresh
    /// tokens, so tokens carrying the old role stop being accepted.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::UserNotFound`] if the user does not exist.
    fn update_user_role(&self, id: Uuid, is_admin: bool) -> StoreFuture<'_, User>;

    /// Update a user's password hash.
    ///
    /// Also bumps [`User::token_version`] and deletes the user's refresh
    /// tokens, revoking every session issued before the change.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::UserNotFound`] if the user does not exist.
    fn update_user_password(&self, id: Uuid, password_hash: String) -> StoreFuture<'_, ()>;

    /// List the groups a user belongs to, sorted by name.
    ///
    /// Returns an empty list for an unknown user.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`](crate::error::StoreError::Database) on
    /// a storage failure.
    fn list_user_groups(&self, user_id: Uuid) -> StoreFuture<'_, Vec<String>>;

    /// Replace the groups a user belongs to.
    ///
    /// Returns the new membership, sorted and deduplicated. An empty list
    /// removes the user from every group.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::UserNotFound`](crate::error::StoreError::UserNotFound)
    /// if the user does not exist.
    fn set_user_groups(&self, user_id: Uuid, groups: Vec<String>) -> StoreFuture<'_, Vec<String>>;

    /// Revoke every session of a user.
    ///
    /// Increments [`User::token_version`], deletes all of the user's refresh
    /// tokens and returns the new version. Access tokens carrying an older
    /// version are rejected from then on.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::UserNotFound`](crate::error::StoreError::UserNotFound)
    /// if the user does not exist.
    fn revoke_user_sessions(&self, id: Uuid) -> StoreFuture<'_, i64>;

    /// Record an issued refresh token.
    ///
    /// Also drops the user's refresh tokens that have already expired.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::UserNotFound`](crate::error::StoreError::UserNotFound)
    /// if the user does not exist.
    fn store_refresh_token(&self, token: NewRefreshToken) -> StoreFuture<'_, ()>;

    /// Atomically remove a refresh token and return its owner.
    ///
    /// A refresh token is single use: once consumed it is gone. Returns `None`
    /// when the token is unknown, already used or expired.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`](crate::error::StoreError::Database) on
    /// a storage failure.
    fn consume_refresh_token(&self, token_hash: &str) -> StoreFuture<'_, Option<Uuid>>;
}
