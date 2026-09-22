//! The [`ApprovalDelegationStore`] trait -- async storage for approval delegations.

use uuid::Uuid;

use crate::entities::{ApprovalDelegation, DelegationFilter, NewApprovalDelegation};
use crate::store::StoreFuture;

/// Async storage abstraction for approval delegations.
///
/// All methods return a [`StoreFuture`] (boxed future) for object safety,
/// allowing the store to be used as `Arc<dyn ApprovalDelegationStore>`.
///
/// # Examples
///
/// ```no_run
/// use chrono::{TimeDelta, Utc};
/// use ironflow_store::prelude::*;
/// use uuid::Uuid;
///
/// # async fn example(alice: Uuid, bob: Uuid) -> Result<(), ironflow_store::error::StoreError> {
/// let store = InMemoryStore::new();
/// let now = Utc::now();
///
/// store.create_delegation(NewApprovalDelegation {
///     from_user_id: alice,
///     to_user_id: bob,
///     valid_from: now,
///     valid_until: now + TimeDelta::days(7),
///     workflow_filter: Some("deploy-*".to_string()),
/// }).await?;
///
/// let received = store.list_active_delegations(DelegationFilter {
///     to_user_id: Some(bob),
///     ..DelegationFilter::default()
/// }).await?;
/// assert_eq!(received.len(), 1);
/// # Ok(())
/// # }
/// ```
pub trait ApprovalDelegationStore: Send + Sync {
    /// Create a new delegation.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn create_delegation(&self, req: NewApprovalDelegation) -> StoreFuture<'_, ApprovalDelegation>;

    /// Find a delegation by ID. Returns `None` if not found.
    ///
    /// Unlike [`list_active_delegations`](Self::list_active_delegations) this
    /// also returns expired and not-yet-started rows, so an expired delegation
    /// can still be revoked.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn find_delegation_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<ApprovalDelegation>>;

    /// List the delegations matching `filter`, newest first.
    ///
    /// Active delegations only: `valid_from <= now < valid_until`. Expired and
    /// not-yet-started rows are filtered out at read time; nothing is deleted.
    /// Ordered by `created_at` descending.
    ///
    /// The workflow glob is *not* applied here -- callers narrow the result with
    /// [`ApprovalDelegation::matches_workflow`].
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn list_active_delegations(
        &self,
        filter: DelegationFilter,
    ) -> StoreFuture<'_, Vec<ApprovalDelegation>>;

    /// Revoke a delegation by ID.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::DelegationNotFound`](crate::error::StoreError) if
    /// the delegation does not exist.
    fn delete_delegation(&self, id: Uuid) -> StoreFuture<'_, ()>;
}
