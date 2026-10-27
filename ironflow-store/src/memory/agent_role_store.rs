//! In-memory [`AgentRoleStore`] implementation.
//!
//! Every method runs under the single write lock of the store, which gives
//! the same atomicity as the row lock of the PostgreSQL implementation.

use std::collections::HashMap;

use chrono::{DateTime, TimeDelta, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::agent_role_store::AgentRoleStore;
use crate::entities::{
    AgentRoleRecord, Page, RoleCostAdmission, RoleCostReservationRequest, RoleCostSettlement,
    effective_cap,
};
use crate::error::StoreError;
use crate::memory::{AgentRoleReservation, InMemoryStore};
use crate::store::StoreFuture;

/// How long an expired reservation is kept before it is purged.
const EXPIRED_RESERVATION_RETENTION: TimeDelta = TimeDelta::days(1);

/// Reset the counter when `period_start` starts a newer period than the
/// stored one. An older `period_start` never resets, nor does a different
/// period alone: two worker versions declaring different periods for one role
/// must not wipe each other's spend.
fn roll_over(record: &mut AgentRoleRecord, period: &str, period_start: DateTime<Utc>) {
    if period_start > record.period_start {
        record.spent_usd = Decimal::ZERO;
    }
    record.period_start = record.period_start.max(period_start);
    record.period = period.to_string();
}

/// Room held by the live reservations of `role` at `now`, `except` one.
fn live_reserved(
    reservations: &HashMap<Uuid, AgentRoleReservation>,
    role: &str,
    now: DateTime<Utc>,
    except: Option<Uuid>,
) -> Decimal {
    reservations
        .iter()
        .filter(|(id, r)| r.role == role && r.expires_at > now && Some(**id) != except)
        .map(|(_, r)| r.amount_usd)
        .sum()
}

fn ttl(ttl_secs: u32) -> TimeDelta {
    TimeDelta::seconds(i64::from(ttl_secs))
}

impl AgentRoleStore for InMemoryStore {
    fn reserve_agent_role_cost(
        &self,
        request: RoleCostReservationRequest,
    ) -> StoreFuture<'_, RoleCostAdmission> {
        Box::pin(async move {
            let now = Utc::now();
            let mut guard = self.state.write().await;
            let state = &mut *guard;

            let record = state
                .agent_roles
                .entry(request.role.clone())
                .or_insert_with(|| AgentRoleRecord {
                    id: Uuid::now_v7(),
                    name: request.role.clone(),
                    code_cap_usd: request.code_cap_usd,
                    operator_cap_usd: None,
                    period: request.period.clone(),
                    period_start: request.period_start,
                    spent_usd: Decimal::ZERO,
                    reserved_usd: Decimal::ZERO,
                    created_at: now,
                    updated_at: now,
                });
            record.code_cap_usd = request.code_cap_usd;
            roll_over(record, &request.period, request.period_start);
            record.updated_at = now;
            let spent_usd = record.spent_usd;
            let cap_usd = effective_cap(request.code_cap_usd, record.operator_cap_usd);

            let reservations = &mut state.agent_role_reservations;
            if let Some(existing) = reservations.get(&request.reservation_id) {
                return Ok(RoleCostAdmission::Admitted {
                    reservation_id: request.reservation_id,
                    amount_usd: existing.amount_usd,
                    cap_usd,
                    spent_usd,
                    reserved_usd: live_reserved(
                        reservations,
                        &request.role,
                        now,
                        Some(request.reservation_id),
                    ),
                });
            }

            let purge_before = now - EXPIRED_RESERVATION_RETENTION;
            reservations.retain(|_, r| r.role != request.role || r.expires_at >= purge_before);
            let reserved_usd = live_reserved(reservations, &request.role, now, None);

            let amount_usd = match cap_usd {
                None => request.budget_usd.unwrap_or(Decimal::ZERO),
                Some(cap_usd) if spent_usd + reserved_usd >= cap_usd => {
                    return Ok(RoleCostAdmission::Refused {
                        cap_usd,
                        spent_usd,
                        reserved_usd,
                    });
                }
                Some(cap_usd) => request
                    .budget_usd
                    .unwrap_or(cap_usd - spent_usd - reserved_usd),
            };
            reservations.insert(
                request.reservation_id,
                AgentRoleReservation {
                    role: request.role,
                    amount_usd,
                    expires_at: now + ttl(request.ttl_secs),
                },
            );
            Ok(RoleCostAdmission::Admitted {
                reservation_id: request.reservation_id,
                amount_usd,
                cap_usd,
                spent_usd,
                reserved_usd,
            })
        })
    }

    fn renew_agent_role_reservation(&self, id: Uuid, ttl_secs: u32) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            if let Some(reservation) = state.agent_role_reservations.get_mut(&id) {
                reservation.expires_at = Utc::now() + ttl(ttl_secs);
            }
            Ok(())
        })
    }

    fn settle_agent_role_cost(&self, settlement: RoleCostSettlement) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut guard = self.state.write().await;
            let state = &mut *guard;
            let Some(reservation) = state
                .agent_role_reservations
                .remove(&settlement.reservation_id)
            else {
                return Ok(());
            };
            if let Some(record) = state.agent_roles.get_mut(&reservation.role) {
                roll_over(record, &settlement.period, settlement.period_start);
                record.spent_usd += settlement.cost_usd;
                record.updated_at = Utc::now();
            }
            Ok(())
        })
    }

    fn get_agent_role(&self, name: &str) -> StoreFuture<'_, Option<AgentRoleRecord>> {
        let name = name.to_string();
        Box::pin(async move {
            let state = self.state.read().await;
            let now = Utc::now();
            let reserved_usd = live_reserved(&state.agent_role_reservations, &name, now, None);
            Ok(state.agent_roles.get(&name).map(|record| AgentRoleRecord {
                reserved_usd,
                ..record.clone()
            }))
        })
    }

    fn list_agent_roles(&self, page: u32, per_page: u32) -> StoreFuture<'_, Page<AgentRoleRecord>> {
        Box::pin(async move {
            let state = self.state.read().await;
            let now = Utc::now();
            let mut items: Vec<AgentRoleRecord> = state
                .agent_roles
                .values()
                .map(|record| AgentRoleRecord {
                    reserved_usd: live_reserved(
                        &state.agent_role_reservations,
                        &record.name,
                        now,
                        None,
                    ),
                    ..record.clone()
                })
                .collect();
            items.sort_by(|a, b| a.name.cmp(&b.name));

            let total = items.len() as u64;
            let page = page.max(1);
            let per_page = per_page.clamp(1, 100);
            let offset = ((page - 1) * per_page) as usize;
            let items = items
                .into_iter()
                .skip(offset)
                .take(per_page as usize)
                .collect();

            Ok(Page {
                items,
                total,
                page,
                per_page,
            })
        })
    }

    fn set_agent_role_operator_cap(
        &self,
        name: &str,
        cap_usd: Option<Decimal>,
    ) -> StoreFuture<'_, AgentRoleRecord> {
        let name = name.to_string();
        Box::pin(async move {
            let mut guard = self.state.write().await;
            let state = &mut *guard;
            let Some(record) = state.agent_roles.get_mut(&name) else {
                return Err(StoreError::AgentRoleNotFound(name));
            };
            record.operator_cap_usd = cap_usd;
            record.updated_at = Utc::now();
            Ok(AgentRoleRecord {
                reserved_usd: live_reserved(
                    &state.agent_role_reservations,
                    &name,
                    Utc::now(),
                    None,
                ),
                ..record.clone()
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn month_start() -> DateTime<Utc> {
        "2026-10-01T00:00:00Z".parse().unwrap()
    }

    fn request(
        role: &str,
        cap: Option<Decimal>,
        budget: Option<Decimal>,
    ) -> RoleCostReservationRequest {
        RoleCostReservationRequest {
            reservation_id: Uuid::now_v7(),
            role: role.to_string(),
            code_cap_usd: cap,
            period: "month".to_string(),
            period_start: month_start(),
            budget_usd: budget,
            ttl_secs: 600,
        }
    }

    async fn settle(store: &InMemoryStore, id: Uuid, cost: Decimal) {
        store
            .settle_agent_role_cost(RoleCostSettlement {
                reservation_id: id,
                period: "month".to_string(),
                period_start: month_start(),
                cost_usd: cost,
            })
            .await
            .unwrap();
    }

    async fn spent(store: &InMemoryStore, name: &str) -> Decimal {
        store
            .get_agent_role(name)
            .await
            .unwrap()
            .expect("role exists")
            .spent_usd
    }

    #[tokio::test]
    async fn admits_until_the_cap_then_refuses() {
        let store = InMemoryStore::new();
        let first = request("reviewer", Some(Decimal::ONE), Some(Decimal::new(6, 1)));
        let admission = store.reserve_agent_role_cost(first.clone()).await.unwrap();
        assert!(matches!(admission, RoleCostAdmission::Admitted { .. }));
        settle(&store, first.reservation_id, Decimal::new(6, 1)).await;

        let second = request("reviewer", Some(Decimal::ONE), Some(Decimal::new(6, 1)));
        let admission = store.reserve_agent_role_cost(second.clone()).await.unwrap();
        assert!(matches!(admission, RoleCostAdmission::Admitted { .. }));
        settle(&store, second.reservation_id, Decimal::new(6, 1)).await;

        let third = request("reviewer", Some(Decimal::ONE), Some(Decimal::new(6, 1)));
        let admission = store.reserve_agent_role_cost(third).await.unwrap();
        assert_eq!(
            admission,
            RoleCostAdmission::Refused {
                cap_usd: Decimal::ONE,
                spent_usd: Decimal::new(12, 1),
                reserved_usd: Decimal::ZERO,
            }
        );
    }

    #[tokio::test]
    async fn zero_cap_refuses_every_step() {
        let store = InMemoryStore::new();
        let admission = store
            .reserve_agent_role_cost(request("off", Some(Decimal::ZERO), Some(Decimal::ONE)))
            .await
            .unwrap();
        assert!(matches!(admission, RoleCostAdmission::Refused { .. }));
    }

    #[tokio::test]
    async fn a_newer_period_resets_the_counter_and_an_older_one_never_does() {
        let store = InMemoryStore::new();
        let first = request("reviewer", Some(Decimal::ONE), Some(Decimal::ONE));
        store.reserve_agent_role_cost(first.clone()).await.unwrap();
        settle(&store, first.reservation_id, Decimal::ONE).await;

        // A worker whose clock is behind sends an older period: no reset.
        let mut late = request("reviewer", Some(Decimal::ONE), Some(Decimal::ONE));
        late.period_start = "2026-09-01T00:00:00Z".parse().unwrap();
        let admission = store.reserve_agent_role_cost(late).await.unwrap();
        assert!(matches!(admission, RoleCostAdmission::Refused { .. }));

        let mut next = request("reviewer", Some(Decimal::ONE), Some(Decimal::ONE));
        next.period_start = "2026-11-01T00:00:00Z".parse().unwrap();
        let admission = store.reserve_agent_role_cost(next).await.unwrap();
        assert!(matches!(
            admission,
            RoleCostAdmission::Admitted { spent_usd, .. } if spent_usd == Decimal::ZERO
        ));
    }

    #[tokio::test]
    async fn alternating_periods_never_wipe_the_counter() {
        let store = InMemoryStore::new();
        let day_start: DateTime<Utc> = "2026-10-27T00:00:00Z".parse().unwrap();
        // A per-day worker charges first: its day starts after the stored month.
        let mut daily = request("reviewer", Some(Decimal::ONE), Some(Decimal::new(6, 1)));
        daily.period = "day".to_string();
        daily.period_start = day_start;
        store.reserve_agent_role_cost(daily.clone()).await.unwrap();
        store
            .settle_agent_role_cost(RoleCostSettlement {
                reservation_id: daily.reservation_id,
                period: "day".to_string(),
                period_start: day_start,
                cost_usd: Decimal::new(6, 1),
            })
            .await
            .unwrap();

        // A per-month worker of an older version charges next: no reset.
        let monthly = request("reviewer", Some(Decimal::ONE), Some(Decimal::new(6, 1)));
        store
            .reserve_agent_role_cost(monthly.clone())
            .await
            .unwrap();
        settle(&store, monthly.reservation_id, Decimal::new(6, 1)).await;
        assert_eq!(spent(&store, "reviewer").await, Decimal::new(12, 1));

        // Back to the per-day worker: the spend of both is still counted.
        let mut daily = request("reviewer", Some(Decimal::ONE), Some(Decimal::new(6, 1)));
        daily.period = "day".to_string();
        daily.period_start = day_start;
        let admission = store.reserve_agent_role_cost(daily).await.unwrap();
        assert!(matches!(admission, RoleCostAdmission::Refused { .. }));
    }

    #[tokio::test]
    async fn reserve_and_settle_are_idempotent() {
        let store = InMemoryStore::new();
        let req = request("reviewer", Some(Decimal::TEN), Some(Decimal::ONE));
        let first = store.reserve_agent_role_cost(req.clone()).await.unwrap();
        let again = store.reserve_agent_role_cost(req.clone()).await.unwrap();
        assert!(matches!(
            again,
            RoleCostAdmission::Admitted { amount_usd, .. } if amount_usd == Decimal::ONE
        ));
        assert!(matches!(first, RoleCostAdmission::Admitted { .. }));
        let record = store.get_agent_role("reviewer").await.unwrap().unwrap();
        assert_eq!(record.reserved_usd, Decimal::ONE);

        settle(&store, req.reservation_id, Decimal::new(5, 1)).await;
        settle(&store, req.reservation_id, Decimal::new(5, 1)).await;
        assert_eq!(spent(&store, "reviewer").await, Decimal::new(5, 1));
    }

    #[tokio::test]
    async fn a_step_without_budget_holds_the_remaining_room() {
        let store = InMemoryStore::new();
        let unbudgeted = request("reviewer", Some(Decimal::TEN), None);
        let admission = store.reserve_agent_role_cost(unbudgeted).await.unwrap();
        assert!(matches!(
            admission,
            RoleCostAdmission::Admitted { amount_usd, .. } if amount_usd == Decimal::TEN
        ));
        let other = store
            .reserve_agent_role_cost(request("reviewer", Some(Decimal::TEN), Some(Decimal::ONE)))
            .await
            .unwrap();
        assert!(matches!(
            other,
            RoleCostAdmission::Refused { reserved_usd, .. } if reserved_usd == Decimal::TEN
        ));
    }

    #[tokio::test]
    async fn an_expired_reservation_no_longer_counts() {
        let store = InMemoryStore::new();
        let mut held = request("reviewer", Some(Decimal::ONE), None);
        held.ttl_secs = 0;
        store.reserve_agent_role_cost(held).await.unwrap();
        let admission = store
            .reserve_agent_role_cost(request("reviewer", Some(Decimal::ONE), None))
            .await
            .unwrap();
        assert!(matches!(
            admission,
            RoleCostAdmission::Admitted { reserved_usd, .. } if reserved_usd == Decimal::ZERO
        ));
    }

    #[tokio::test]
    async fn renewing_keeps_a_reservation_alive() {
        let store = InMemoryStore::new();
        let mut held = request("reviewer", Some(Decimal::ONE), None);
        held.ttl_secs = 0;
        store.reserve_agent_role_cost(held.clone()).await.unwrap();
        store
            .renew_agent_role_reservation(held.reservation_id, 600)
            .await
            .unwrap();
        let record = store.get_agent_role("reviewer").await.unwrap().unwrap();
        assert_eq!(record.reserved_usd, Decimal::ONE);
        // An unknown reservation is a no-op.
        store
            .renew_agent_role_reservation(Uuid::now_v7(), 600)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn operator_cap_lowers_but_cannot_raise_the_code_cap() {
        let store = InMemoryStore::new();
        let req = request("reviewer", Some(Decimal::TEN), Some(Decimal::new(15, 0)));
        store.reserve_agent_role_cost(req.clone()).await.unwrap();
        settle(&store, req.reservation_id, Decimal::new(15, 0)).await;

        let record = store
            .set_agent_role_operator_cap("reviewer", Some(Decimal::from(20)))
            .await
            .unwrap();
        assert_eq!(record.effective_cap_usd(), Some(Decimal::TEN));
        let admission = store
            .reserve_agent_role_cost(request("reviewer", Some(Decimal::TEN), Some(Decimal::ONE)))
            .await
            .unwrap();
        assert!(matches!(
            admission,
            RoleCostAdmission::Refused { cap_usd, .. } if cap_usd == Decimal::TEN
        ));

        store
            .set_agent_role_operator_cap("triage", None)
            .await
            .expect_err("never seen");
        let lowered = store
            .reserve_agent_role_cost(request("small", Some(Decimal::TEN), Some(Decimal::ONE)))
            .await
            .unwrap();
        assert!(matches!(lowered, RoleCostAdmission::Admitted { .. }));
        store
            .set_agent_role_operator_cap("small", Some(Decimal::ONE))
            .await
            .unwrap();
        let admission = store
            .reserve_agent_role_cost(request("small", Some(Decimal::TEN), Some(Decimal::ONE)))
            .await
            .unwrap();
        assert!(matches!(
            admission,
            RoleCostAdmission::Refused { cap_usd, .. } if cap_usd == Decimal::ONE
        ));
    }

    #[tokio::test]
    async fn operator_cap_on_an_unknown_role_is_not_found() {
        let store = InMemoryStore::new();
        let err = store
            .set_agent_role_operator_cap("ghost", Some(Decimal::ONE))
            .await
            .expect_err("unknown role");
        assert!(matches!(err, StoreError::AgentRoleNotFound(ref name) if name == "ghost"));
    }

    #[tokio::test]
    async fn list_is_ordered_by_name_and_paginated() {
        let store = InMemoryStore::new();
        for name in ["charlie", "alpha", "bravo"] {
            store
                .reserve_agent_role_cost(request(name, None, None))
                .await
                .unwrap();
        }
        let page = store.list_agent_roles(1, 2).await.unwrap();
        assert_eq!(page.total, 3);
        let names: Vec<_> = page.items.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["alpha", "bravo"]);
        let page = store.list_agent_roles(2, 2).await.unwrap();
        let names: Vec<_> = page.items.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["charlie"]);
        assert!(store.get_agent_role("delta").await.unwrap().is_none());
    }
}
