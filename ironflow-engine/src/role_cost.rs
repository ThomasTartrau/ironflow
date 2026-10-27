//! Cost cap of agent roles, enforced across every run of every workflow.
//!
//! [`RoleCostProvider`] wraps the engine's [`AgentProvider`]. An invocation
//! whose config carries an [`AgentRoleBinding`] (set by `AgentTask` for a
//! named [`AgentRole`](crate::config::AgentRole)) goes through three phases:
//!
//! 1. **Admission**: the store reserves room for the step under the role's
//!    cap, atomically across workers. Once the settled cost plus the room held
//!    by running steps reaches the cap, the step fails with
//!    [`AgentError::RoleCostCapReached`] and the provider is never called. A
//!    store failure refuses the step too ([`AgentError::RoleCostUnavailable`]):
//!    the cap fails closed.
//! 2. **Reservation**: the step holds its `max_budget_usd`, or all the
//!    remaining room when it declares none, for [`ROLE_RESERVATION_TTL`],
//!    renewed while the provider call runs. A crashed worker frees the room
//!    when the lease expires.
//! 3. **Settlement**: the cost the provider reported is added to the counter
//!    of the current period and the reservation is released. A failure raised
//!    before the model ran (capacity wait, rate limit, unknown account) costs
//!    nothing; any other failure that reports no usage, or a dropped
//!    invocation, is charged the room it held.
//!
//! Admission sees every running step's reservation, so the workers together
//! overshoot the cap by at most one step's actual cost above its budget.
//! A config without binding is passed through: the store is not touched.
//!
//! # Examples
//!
//! ```no_run
//! use std::sync::Arc;
//! use ironflow_core::providers::claude::ClaudeCodeProvider;
//! use ironflow_engine::role_cost::RoleCostProvider;
//! use ironflow_store::memory::InMemoryStore;
//!
//! let provider = RoleCostProvider::new(
//!     Arc::new(ClaudeCodeProvider::new()),
//!     Arc::new(InMemoryStore::new()),
//! );
//! # let _ = provider;
//! ```

use std::convert::Infallible;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use ironflow_core::agent_role::{AgentRoleBinding, CostPeriod};
use ironflow_core::error::AgentError;
use ironflow_core::provider::{
    AgentConfig, AgentOutput, AgentProvider, InvokeFuture, LogSink, ReleaseFuture,
};
use ironflow_store::entities::{RoleCostAdmission, RoleCostReservationRequest, RoleCostSettlement};
use ironflow_store::store::Store;
use rust_decimal::Decimal;
use tokio::runtime::Handle;
use tokio::select;
use tokio::time::{Instant, interval_at};
use tracing::{error, warn};
use uuid::Uuid;

/// Lifetime of a reservation: a worker that crashed mid-step frees the room
/// of its role after at most this long.
pub const ROLE_RESERVATION_TTL: Duration = Duration::from_secs(600);

/// How often a running step renews its reservation.
const RENEW_EVERY: Duration = Duration::from_secs(180);

/// An [`AgentProvider`] enforcing the cost cap of agent roles.
///
/// See the [module documentation](self).
pub struct RoleCostProvider {
    inner: Arc<dyn AgentProvider>,
    store: Arc<dyn Store>,
}

impl fmt::Debug for RoleCostProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RoleCostProvider").finish_non_exhaustive()
    }
}

/// Settles the reservation of an invocation dropped before it settled (a
/// guard timeout, a cancellation), so its room is freed at once instead of
/// at lease expiry.
struct ReservationGuard {
    store: Arc<dyn Store>,
    reservation_id: Uuid,
    role: String,
    period: CostPeriod,
    /// Room the reservation holds, charged when the step's real cost is
    /// unknown: the provider may have spent it, and charging zero would let
    /// a role whose steps keep failing spend without limit.
    reserved_usd: Decimal,
    settled: bool,
}

impl ReservationGuard {
    /// Guard of a reservation the store may be recording: the provider has
    /// not been called, so a release before admission completes charges
    /// nothing. Settling an id the store never recorded is a no-op.
    fn pending(store: Arc<dyn Store>, reservation_id: Uuid, binding: &AgentRoleBinding) -> Self {
        Self {
            store,
            reservation_id,
            role: binding.name.clone(),
            period: binding.period,
            reserved_usd: Decimal::ZERO,
            settled: false,
        }
    }

    async fn settle(mut self, cost_usd: Decimal) {
        self.settled = true;
        let settlement = RoleCostSettlement {
            reservation_id: self.reservation_id,
            period: self.period.as_str().to_string(),
            period_start: self.period.start(Utc::now()),
            cost_usd,
        };
        // The step already ran and paid: failing it here would let a retry
        // spend again. The reservation keeps holding the room until it expires.
        if let Err(err) = self.store.settle_agent_role_cost(settlement).await {
            error!(
                role = %self.role,
                reservation_id = %self.reservation_id,
                error = %err,
                "failed to record the cost of an agent role step"
            );
        }
    }
}

impl Drop for ReservationGuard {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let Ok(handle) = Handle::try_current() else {
            // No runtime to release it: the lease expiry frees the room.
            return;
        };
        let store = Arc::clone(&self.store);
        let role = self.role.clone();
        let settlement = RoleCostSettlement {
            reservation_id: self.reservation_id,
            period: self.period.as_str().to_string(),
            period_start: self.period.start(Utc::now()),
            cost_usd: self.reserved_usd,
        };
        handle.spawn(async move {
            let reservation_id = settlement.reservation_id;
            if let Err(err) = store.settle_agent_role_cost(settlement).await {
                warn!(
                    role = %role,
                    reservation_id = %reservation_id,
                    error = %err,
                    "failed to release the reservation of a dropped agent role step"
                );
            }
        });
    }
}

/// Cost an invocation reported, including the partial cost of a failure:
/// the same accounting as the run cost. `None` when a failure carries no
/// usage: the provider may have spent money it did not report.
fn reported_cost(result: &Result<AgentOutput, AgentError>) -> Option<Decimal> {
    let cost = match result {
        // A success without a cost comes from a provider that reports none.
        Ok(output) => return Some(to_decimal(output.cost_usd)),
        Err(AgentError::SchemaValidation { partial_usage, .. }) => partial_usage.cost_usd,
        Err(AgentError::BudgetExceeded { spent_usd, .. }) => Some(*spent_usd),
        // Raised before the model ran (capacity wait, unknown account, 429,
        // pre-flight checks): nothing was spent, so nothing is charged.
        Err(
            AgentError::CapacityWait { .. }
            | AgentError::NoCapacity { .. }
            | AgentError::AccountNotFound { .. }
            | AgentError::RateLimited { .. }
            | AgentError::PromptTooLarge { .. }
            | AgentError::UnknownToolProfile { .. }
            | AgentError::ToolProfileUnsupported { .. }
            | AgentError::RoleCostCapReached { .. }
            | AgentError::RoleCostUnavailable { .. },
        ) => return Some(Decimal::ZERO),
        Err(_) => None,
    };
    cost.map(|c| to_decimal(Some(c)))
}

fn to_decimal(cost_usd: Option<f64>) -> Decimal {
    cost_usd
        .and_then(|c| Decimal::try_from(c).ok())
        .unwrap_or(Decimal::ZERO)
}

impl RoleCostProvider {
    /// Wrap `inner`, charging agent roles to the counters of `store`.
    ///
    /// # Examples
    ///
    /// See the [module documentation](self).
    pub fn new(inner: Arc<dyn AgentProvider>, store: Arc<dyn Store>) -> Self {
        Self { inner, store }
    }

    async fn invoke_inner(
        &self,
        config: &AgentConfig,
        sink: Option<Arc<dyn LogSink>>,
    ) -> Result<AgentOutput, AgentError> {
        match sink {
            Some(sink) => self.inner.invoke_with_logs(config, sink).await,
            None => self.inner.invoke(config).await,
        }
    }

    async fn run(
        &self,
        config: &AgentConfig,
        sink: Option<Arc<dyn LogSink>>,
    ) -> Result<AgentOutput, AgentError> {
        let Some(binding) = config.agent_role.as_ref() else {
            return self.invoke_inner(config, sink).await;
        };
        let guard = self.admit(binding, config.max_budget_usd).await?;

        // Both futures are polled together: a slow renewal (the worker retries
        // the API call) never stalls the provider call or its log stream.
        let result = select! {
            result = self.invoke_inner(config, sink) => result,
            never = self.keep_reservation(binding, guard.reservation_id) => match never {},
        };

        let cost_usd = reported_cost(&result).unwrap_or(guard.reserved_usd);
        guard.settle(cost_usd).await;
        result
    }

    /// Renew the lease of a reservation every [`RENEW_EVERY`] until dropped.
    async fn keep_reservation(
        &self,
        binding: &AgentRoleBinding,
        reservation_id: Uuid,
    ) -> Infallible {
        let mut ticker = interval_at(Instant::now() + RENEW_EVERY, RENEW_EVERY);
        let ttl_secs = ttl_secs();
        loop {
            ticker.tick().await;
            if let Err(err) = self
                .store
                .renew_agent_role_reservation(reservation_id, ttl_secs)
                .await
            {
                warn!(
                    role = %binding.name,
                    reservation_id = %reservation_id,
                    error = %err,
                    "failed to renew the reservation of an agent role step"
                );
            }
        }
    }

    /// Reserve room for the step, or refuse it.
    async fn admit(
        &self,
        binding: &AgentRoleBinding,
        max_budget_usd: Option<f64>,
    ) -> Result<ReservationGuard, AgentError> {
        let now = Utc::now();
        let reservation_id = Uuid::now_v7();
        // NaN or a negative budget is no budget: the step holds the remaining room.
        let budget_usd = max_budget_usd
            .and_then(|b| Decimal::try_from(b).ok())
            .filter(|b| *b >= Decimal::ZERO);
        // Armed before the reservation exists: a future dropped while the store
        // records it still releases it, charging nothing: the provider was
        // never called.
        let mut guard = ReservationGuard::pending(Arc::clone(&self.store), reservation_id, binding);
        let request = RoleCostReservationRequest {
            reservation_id,
            role: binding.name.clone(),
            code_cap_usd: binding.cost_cap_usd,
            period: binding.period.as_str().to_string(),
            period_start: binding.period.start(now),
            budget_usd,
            ttl_secs: ttl_secs(),
        };
        match self.store.reserve_agent_role_cost(request).await {
            Err(err) => {
                // The guard stays armed: the store may have recorded the
                // reservation before the error reached us. It releases it at
                // zero cost, the provider was never called.
                warn!(
                    role = %binding.name,
                    error = %err,
                    "cost of agent role unavailable, step refused"
                );
                Err(AgentError::RoleCostUnavailable {
                    role: binding.name.clone(),
                    message: err.to_string(),
                })
            }
            Ok(RoleCostAdmission::Refused {
                cap_usd,
                spent_usd,
                reserved_usd,
            }) => {
                // Refused: nothing was reserved, nothing to release.
                guard.settled = true;
                warn!(
                    role = %binding.name,
                    cap_usd = %cap_usd,
                    spent_usd = %spent_usd,
                    reserved_usd = %reserved_usd,
                    "agent role cost cap reached, step refused"
                );
                Err(AgentError::RoleCostCapReached {
                    role: binding.name.clone(),
                    cap_usd,
                    spent_usd,
                    reserved_usd,
                    period: binding.period,
                    resets_at: binding.period.next_start(now),
                })
            }
            Ok(RoleCostAdmission::Admitted { amount_usd, .. }) => {
                // From here the provider is called: a drop is charged the room.
                guard.reserved_usd = amount_usd;
                Ok(guard)
            }
        }
    }
}

fn ttl_secs() -> u32 {
    u32::try_from(ROLE_RESERVATION_TTL.as_secs()).expect("the reservation TTL fits in u32")
}

impl AgentProvider for RoleCostProvider {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(self.run(config, None))
    }

    fn invoke_with_logs<'a>(
        &'a self,
        config: &'a AgentConfig,
        log_sink: Arc<dyn LogSink>,
    ) -> InvokeFuture<'a> {
        Box::pin(self.run(config, Some(log_sink)))
    }

    fn release_run<'a>(&'a self, run_id: &'a str) -> ReleaseFuture<'a> {
        self.inner.release_run(run_id)
    }

    fn account_kind(&self) -> Option<&'static str> {
        self.inner.account_kind()
    }

    fn account_kind_for(&self, config: &AgentConfig) -> Option<&'static str> {
        self.inner.account_kind_for(config)
    }
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ironflow_core::agent_role::RoleCostCap;
    use ironflow_core::error::PartialUsage;
    use ironflow_store::agent_role_store::AgentRoleStore;
    use ironflow_store::memory::InMemoryStore;
    use serde_json::json;
    use tokio::time::{sleep, timeout};

    use super::*;
    use crate::testing::MockAgentProvider;

    /// Provider whose invocation never ends, to drop it mid-call.
    struct Hanging;

    impl AgentProvider for Hanging {
        fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
            Box::pin(pending::<Result<AgentOutput, AgentError>>())
        }
    }

    fn bound(cap: Option<Decimal>, budget: Option<f64>) -> AgentConfig {
        let mut config = AgentConfig::new("review").agent_role(AgentRoleBinding::new(
            "reviewer",
            cap.map(RoleCostCap::per_month),
        ));
        config.max_budget_usd = budget;
        config
    }

    fn costing(cost: f64, calls: Arc<AtomicUsize>) -> Arc<dyn AgentProvider> {
        Arc::new(MockAgentProvider::new(move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            let mut output = AgentOutput::new(json!({"ok": true}));
            output.cost_usd = Some(cost);
            Ok(output)
        }))
    }

    #[tokio::test]
    async fn a_config_without_binding_is_passed_through() {
        let store = Arc::new(InMemoryStore::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = RoleCostProvider::new(costing(0.5, calls.clone()), store.clone());
        provider
            .invoke(&AgentConfig::new("review"))
            .await
            .expect("passes through");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.list_agent_roles(1, 10).await.unwrap().total, 0);
    }

    #[tokio::test]
    async fn a_successful_step_settles_its_reported_cost() {
        let store = Arc::new(InMemoryStore::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = RoleCostProvider::new(costing(0.25, calls.clone()), store.clone());
        provider
            .invoke(&bound(Some(Decimal::ONE), Some(0.5)))
            .await
            .expect("admitted");
        let record = store.get_agent_role("reviewer").await.unwrap().unwrap();
        assert_eq!(record.spent_usd, Decimal::new(25, 2));
        assert_eq!(record.reserved_usd, Decimal::ZERO);
    }

    #[tokio::test]
    async fn a_refused_step_never_calls_the_provider() {
        let store = Arc::new(InMemoryStore::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = RoleCostProvider::new(costing(0.6, calls.clone()), store.clone());
        let config = bound(Some(Decimal::new(5, 1)), None);
        provider.invoke(&config).await.expect("first is admitted");

        let err = provider.invoke(&config).await.expect_err("cap reached");
        assert!(matches!(
            err,
            AgentError::RoleCostCapReached { ref role, cap_usd, period: CostPeriod::Month, .. }
                if role == "reviewer" && cap_usd == Decimal::new(5, 1)
        ));
        assert!(err.to_string().contains("reviewer"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_partial_cost_of_a_schema_failure_is_settled() {
        let store = Arc::new(InMemoryStore::new());
        let inner: Arc<dyn AgentProvider> = Arc::new(MockAgentProvider::new(|_| {
            Err(AgentError::SchemaValidation {
                expected: "object".to_string(),
                got: "string".to_string(),
                debug_messages: Vec::new(),
                partial_usage: Box::new(PartialUsage {
                    cost_usd: Some(0.3),
                    ..PartialUsage::default()
                }),
                raw_response: None,
            })
        }));
        let provider = RoleCostProvider::new(inner, store.clone());
        provider
            .invoke(&bound(None, Some(1.0)))
            .await
            .expect_err("schema failure");
        let record = store.get_agent_role("reviewer").await.unwrap().unwrap();
        assert_eq!(record.spent_usd, Decimal::new(3, 1));
    }

    #[tokio::test]
    async fn a_dropped_invocation_is_charged_its_reservation() {
        let store = Arc::new(InMemoryStore::new());
        let provider = RoleCostProvider::new(Arc::new(Hanging), store.clone());
        let config = bound(Some(Decimal::ONE), None);
        timeout(Duration::from_millis(50), provider.invoke(&config))
            .await
            .expect_err("never ends");

        // The release runs on a spawned task.
        let mut reserved = Decimal::ONE;
        for _ in 0..100 {
            reserved = store
                .get_agent_role("reviewer")
                .await
                .unwrap()
                .unwrap()
                .reserved_usd;
            if reserved == Decimal::ZERO {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(reserved, Decimal::ZERO);
        // The provider may have spent the room it held: the cap stays closed.
        let record = store.get_agent_role("reviewer").await.unwrap().unwrap();
        assert_eq!(record.spent_usd, Decimal::ONE);
    }

    #[tokio::test]
    async fn a_reservation_released_before_the_provider_call_is_charged_nothing() {
        // The store recorded the reservation but the reply was lost (a worker
        // HTTP timeout, a 5xx after commit, a cancellation mid-request).
        let store = Arc::new(InMemoryStore::new());
        let reservation_id = Uuid::now_v7();
        let binding = AgentRoleBinding::new("reviewer", Some(RoleCostCap::per_month(Decimal::ONE)));
        let admission = store
            .reserve_agent_role_cost(RoleCostReservationRequest {
                reservation_id,
                role: binding.name.clone(),
                code_cap_usd: binding.cost_cap_usd,
                period: binding.period.as_str().to_string(),
                period_start: binding.period.start(Utc::now()),
                budget_usd: Some(Decimal::new(5, 1)),
                ttl_secs: ttl_secs(),
            })
            .await
            .unwrap();
        assert!(matches!(admission, RoleCostAdmission::Admitted { .. }));

        drop(ReservationGuard::pending(
            store.clone(),
            reservation_id,
            &binding,
        ));

        // The release runs on a spawned task.
        let mut reserved = Decimal::ONE;
        for _ in 0..100 {
            reserved = store
                .get_agent_role("reviewer")
                .await
                .unwrap()
                .unwrap()
                .reserved_usd;
            if reserved == Decimal::ZERO {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(reserved, Decimal::ZERO);
        let record = store.get_agent_role("reviewer").await.unwrap().unwrap();
        assert_eq!(record.spent_usd, Decimal::ZERO);
    }

    #[tokio::test]
    async fn a_failure_without_usage_is_charged_its_reservation() {
        let store = Arc::new(InMemoryStore::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let inner: Arc<dyn AgentProvider> = Arc::new(MockAgentProvider::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            Err(AgentError::Timeout {
                limit: Duration::from_secs(1),
            })
        }));
        let provider = RoleCostProvider::new(inner, store.clone());
        let config = bound(Some(Decimal::ONE), Some(0.6));
        provider.invoke(&config).await.expect_err("timed out");
        let record = store.get_agent_role("reviewer").await.unwrap().unwrap();
        assert_eq!(record.spent_usd, Decimal::new(6, 1));
        assert_eq!(record.reserved_usd, Decimal::ZERO);

        provider.invoke(&config).await.expect_err("timed out again");
        let err = provider.invoke(&config).await.expect_err("cap reached");
        assert!(matches!(err, AgentError::RoleCostCapReached { .. }));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_failure_raised_before_the_model_ran_is_charged_nothing() {
        let store = Arc::new(InMemoryStore::new());
        let inner: Arc<dyn AgentProvider> = Arc::new(MockAgentProvider::new(|_| {
            Err(AgentError::CapacityWait {
                kind: "claude".to_string(),
                wake_at: Utc::now(),
            })
        }));
        let provider = RoleCostProvider::new(inner, store.clone());
        // Without a budget the step holds all the room: charging it would
        // lock the role until the period ends.
        let config = bound(Some(Decimal::ONE), None);
        for _ in 0..3 {
            let err = provider.invoke(&config).await.expect_err("capacity wait");
            assert!(matches!(err, AgentError::CapacityWait { .. }));
        }
        let record = store.get_agent_role("reviewer").await.unwrap().unwrap();
        assert_eq!(record.spent_usd, Decimal::ZERO);
        assert_eq!(record.reserved_usd, Decimal::ZERO);
    }
}
