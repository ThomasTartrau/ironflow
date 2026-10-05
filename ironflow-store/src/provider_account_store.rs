//! The [`ProviderAccountStore`] trait -- storage of Provider Accounts and their usage.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::entities::{
    NewProviderAccount, NewProviderAccountObservation, Page, ProviderAccount,
    ProviderAccountCandidate, ProviderAccountUpdate, ProviderAccountUsagePoint,
    ProviderAccountWindow,
};
use crate::store::StoreFuture;

/// Async storage abstraction for Provider Accounts.
///
/// The credential of an account is not stored here: it lives in the system
/// secret [`ProviderAccount::secret_key`], managed through
/// [`SecretStore`](crate::secret_store::SecretStore).
pub trait ProviderAccountStore: Send + Sync {
    /// Create an account.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::DuplicateProviderAccount`](crate::error::StoreError)
    /// when the name is taken.
    fn create_provider_account(&self, req: NewProviderAccount) -> StoreFuture<'_, ProviderAccount>;

    /// Find an account by ID.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn get_provider_account(&self, id: Uuid) -> StoreFuture<'_, Option<ProviderAccount>>;

    /// Find several accounts by ID in one lookup.
    ///
    /// Unknown IDs are silently omitted and an empty `ids` returns an empty
    /// list without touching storage. The order of the result is unspecified.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn list_provider_accounts_by_ids(
        &self,
        ids: Vec<Uuid>,
    ) -> StoreFuture<'_, Vec<ProviderAccount>>;

    /// Find an account by name.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn find_provider_account_by_name(&self, name: &str)
    -> StoreFuture<'_, Option<ProviderAccount>>;

    /// List accounts, optionally of one kind, ordered by `priority` then `name`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn list_provider_accounts(
        &self,
        kind: Option<String>,
        page: u32,
        per_page: u32,
    ) -> StoreFuture<'_, Page<ProviderAccount>>;

    /// Update an account and bump `updated_at`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::ProviderAccountNotFound`](crate::error::StoreError)
    /// when the account does not exist.
    fn update_provider_account(
        &self,
        id: Uuid,
        update: ProviderAccountUpdate,
    ) -> StoreFuture<'_, ProviderAccount>;

    /// Delete an account. Its windows and usage are deleted with it and the
    /// steps that ran under it lose their `account_id`.
    ///
    /// Returns `false` when the account did not exist.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn delete_provider_account(&self, id: Uuid) -> StoreFuture<'_, bool>;

    /// Latest windows of every account in `ids`, in one query.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn list_provider_account_windows(
        &self,
        ids: Vec<Uuid>,
    ) -> StoreFuture<'_, Vec<ProviderAccountWindow>>;

    /// Usage history of an account since `since`, oldest first.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn list_provider_account_usage(
        &self,
        id: Uuid,
        since: DateTime<Utc>,
    ) -> StoreFuture<'_, Vec<ProviderAccountUsagePoint>>;

    /// Record observed windows in one transaction and return the current windows.
    ///
    /// Every window is upserted unless a newer observation is already stored,
    /// and appended to the history. `auth_failed_at` is set when
    /// `auth_failed`, and cleared when windows were recorded without it.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::ProviderAccountNotFound`](crate::error::StoreError)
    /// when the account does not exist.
    fn record_provider_account_observation(
        &self,
        id: Uuid,
        observation: NewProviderAccountObservation,
    ) -> StoreFuture<'_, Vec<ProviderAccountWindow>>;

    /// Delete usage history observed before `before`. Returns the number of rows deleted.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn purge_provider_account_usage(&self, before: DateTime<Utc>) -> StoreFuture<'_, u64>;

    /// Accounts of `kind` a step may run under: enabled, credential not
    /// rejected and not expired, with their windows and running steps.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn list_provider_account_candidates(
        &self,
        kind: String,
    ) -> StoreFuture<'_, Vec<ProviderAccountCandidate>>;
}
