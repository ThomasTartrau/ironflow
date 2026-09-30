//! Secret management routes (admin only).

pub mod create;
pub mod delete;
pub mod key_versions;
pub mod list;
pub mod rotate;
pub mod update;

use ironflow_store::entities::PROVIDER_ACCOUNT_SECRET_PREFIX;

use crate::error::ApiError;

/// Refuse keys of the Provider Account namespace, managed through
/// `/provider-accounts` only.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for a key under `accounts/`.
pub(crate) fn reject_provider_account_key(key: &str) -> Result<(), ApiError> {
    if key.starts_with(PROVIDER_ACCOUNT_SECRET_PREFIX) {
        return Err(ApiError::BadRequest(format!(
            "keys under '{PROVIDER_ACCOUNT_SECRET_PREFIX}' are managed through /provider-accounts"
        )));
    }
    Ok(())
}
