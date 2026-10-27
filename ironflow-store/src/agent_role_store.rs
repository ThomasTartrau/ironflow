//! The [`AgentRoleStore`] trait -- the cost counter of each agent role,
//! shared by every worker.

use rust_decimal::Decimal;
use uuid::Uuid;

use crate::entities::{
    AgentRoleRecord, Page, RoleCostAdmission, RoleCostReservationRequest, RoleCostSettlement,
};
use crate::store::StoreFuture;

/// Async storage for the cost counters of agent roles.
///
/// Both backends share these semantics:
///
/// - **Reserve** upserts the role by name and, under its row lock, decides
///   the admission. The stored code cap and period follow the request
///   (display only). The counter rolls over (`spent_usd = 0`) when the
///   request's `period_start` is newer than the stored one; an older
///   `period_start` (a worker clock behind) never resets it, nor does a
///   different period alone (two worker versions declaring different periods
///   for one role would otherwise wipe each other's spend).
///   Reserving again with an existing `reservation_id` returns `Admitted` with
///   the amount held. The enforced cap is the lower of the request's code cap
///   and the operator cap. Without a cap the step is admitted and holds its
///   budget (zero when it has none). When `spent + reserved >= cap` the step
///   is `Refused`. Otherwise it holds its budget, or all the remaining room
///   (`cap - spent - reserved`) when it declares none. Reservations expired
///   for more than a day are purged.
/// - **Settle** deletes the reservation and, when it existed, applies the same
///   rollover rule with the settlement's period, then adds the cost. An
///   unknown reservation is already settled: nothing changes.
/// - **Read** (`get`, `list`) reports in `reserved_usd` the live reservations
///   only (`expires_at` in the future).
///
/// # Examples
///
/// ```no_run
/// use chrono::Utc;
/// use ironflow_store::prelude::*;
/// use rust_decimal::Decimal;
/// use uuid::Uuid;
///
/// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
/// let store = InMemoryStore::new();
/// let reservation_id = Uuid::now_v7();
/// let admission = store
///     .reserve_agent_role_cost(RoleCostReservationRequest {
///         reservation_id,
///         role: "reviewer".to_string(),
///         code_cap_usd: Some(Decimal::TEN),
///         period: "month".to_string(),
///         period_start: Utc::now(),
///         budget_usd: Some(Decimal::ONE),
///         ttl_secs: 600,
///     })
///     .await?;
/// assert!(matches!(admission, RoleCostAdmission::Admitted { .. }));
/// store
///     .settle_agent_role_cost(RoleCostSettlement {
///         reservation_id,
///         period: "month".to_string(),
///         period_start: Utc::now(),
///         cost_usd: Decimal::new(5, 1),
///     })
///     .await?;
/// # Ok(())
/// # }
/// ```
pub trait AgentRoleStore: Send + Sync {
    /// Reserve room for one agent step of a role, or refuse it at the cap.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn reserve_agent_role_cost(
        &self,
        request: RoleCostReservationRequest,
    ) -> StoreFuture<'_, RoleCostAdmission>;

    /// Extend a reservation to `ttl_secs` from now. An unknown ID is a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn renew_agent_role_reservation(&self, id: Uuid, ttl_secs: u32) -> StoreFuture<'_, ()>;

    /// Release a reservation and add the cost of its step to the counter.
    /// Idempotent: an unknown ID is already settled.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn settle_agent_role_cost(&self, settlement: RoleCostSettlement) -> StoreFuture<'_, ()>;

    /// Get the counter of the role `name`, `None` if it was never seen.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn get_agent_role(&self, name: &str) -> StoreFuture<'_, Option<AgentRoleRecord>>;

    /// List one page of the roles, ordered by name. `page` is 1-based.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn list_agent_roles(&self, page: u32, per_page: u32) -> StoreFuture<'_, Page<AgentRoleRecord>>;

    /// Set (`Some`) or clear (`None`) the operator cap of the role `name`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::AgentRoleNotFound`](crate::error::StoreError::AgentRoleNotFound)
    /// when the role was never seen, or a storage error.
    fn set_agent_role_operator_cap(
        &self,
        name: &str,
        cap_usd: Option<Decimal>,
    ) -> StoreFuture<'_, AgentRoleRecord>;
}
