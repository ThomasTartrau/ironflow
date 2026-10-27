#![cfg(feature = "store-postgres")]

//! Integration tests for the agent role cost counters on PostgreSQL.
//!
//! The guarantee that matters (concurrent reservers of one role never exceed
//! its cap by more than one step) lives in the row lock of the upsert, so it
//! needs a real database (`DATABASE_URL`).
//!
//! Run them with:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store --features store-postgres --test postgres_agent_roles -- --ignored
//! ```

use std::env::var;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use ironflow_store::agent_role_store::AgentRoleStore;
use ironflow_store::entities::{RoleCostAdmission, RoleCostReservationRequest, RoleCostSettlement};
use ironflow_store::error::StoreError;
use ironflow_store::postgres::{PoolConfig, PostgresStore};
use rust_decimal::Decimal;
use tokio::task::JoinSet;
use uuid::Uuid;

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    let config = PoolConfig {
        acquire_timeout: Duration::from_secs(60),
        connect_timeout: Duration::from_secs(60),
        ..PoolConfig::default()
    };
    PostgresStore::with_config(&url, config)
        .await
        .expect("failed to connect to PostgreSQL")
}

/// A role name no other test uses.
fn unique_role(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::now_v7().simple())
}

fn month(month: u32) -> DateTime<Utc> {
    format!("2026-{month:02}-01T00:00:00Z").parse().unwrap()
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
        period_start: month(10),
        budget_usd: budget,
        ttl_secs: 600,
    }
}

fn settlement(id: Uuid, cost: Decimal, period_start: DateTime<Utc>) -> RoleCostSettlement {
    RoleCostSettlement {
        reservation_id: id,
        period: "month".to_string(),
        period_start,
        cost_usd: cost,
    }
}

#[tokio::test]
#[ignore]
async fn concurrent_reservations_never_exceed_the_cap_by_more_than_one_step() {
    let store = Arc::new(get_store().await);
    let role = unique_role("concurrent");
    let mut tasks = JoinSet::new();
    for _ in 0..20 {
        let store = Arc::clone(&store);
        let req = request(&role, Some(Decimal::from(5)), Some(Decimal::ONE));
        tasks.spawn(async move { store.reserve_agent_role_cost(req).await });
    }
    let mut admitted = 0;
    while let Some(result) = tasks.join_next().await {
        if let RoleCostAdmission::Admitted { .. } = result.unwrap().unwrap() {
            admitted += 1;
        }
    }
    assert_eq!(admitted, 5);
    let record = store.get_agent_role(&role).await.unwrap().unwrap();
    assert_eq!(record.reserved_usd, Decimal::from(5));
}

#[tokio::test]
#[ignore]
async fn settle_adds_the_cost_once_and_refuses_at_the_cap() {
    let store = get_store().await;
    let role = unique_role("settle");
    let req = request(&role, Some(Decimal::ONE), Some(Decimal::ONE));
    let admission = store.reserve_agent_role_cost(req.clone()).await.unwrap();
    assert!(matches!(admission, RoleCostAdmission::Admitted { .. }));
    // Reserving again with the same ID returns the first admission.
    let again = store.reserve_agent_role_cost(req.clone()).await.unwrap();
    assert!(matches!(
        again,
        RoleCostAdmission::Admitted { amount_usd, .. } if amount_usd == Decimal::ONE
    ));

    let cost = Decimal::new(12, 1);
    store
        .settle_agent_role_cost(settlement(req.reservation_id, cost, month(10)))
        .await
        .unwrap();
    store
        .settle_agent_role_cost(settlement(req.reservation_id, cost, month(10)))
        .await
        .unwrap();
    let record = store.get_agent_role(&role).await.unwrap().unwrap();
    assert_eq!(record.spent_usd, cost);
    assert_eq!(record.reserved_usd, Decimal::ZERO);

    let refused = store
        .reserve_agent_role_cost(request(&role, Some(Decimal::ONE), Some(Decimal::ONE)))
        .await
        .unwrap();
    assert_eq!(
        refused,
        RoleCostAdmission::Refused {
            cap_usd: Decimal::ONE,
            spent_usd: cost,
            reserved_usd: Decimal::ZERO,
        }
    );
}

#[tokio::test]
#[ignore]
async fn a_newer_period_resets_the_counter_and_an_older_one_never_does() {
    let store = get_store().await;
    let role = unique_role("rollover");
    let req = request(&role, Some(Decimal::ONE), Some(Decimal::ONE));
    store.reserve_agent_role_cost(req.clone()).await.unwrap();
    store
        .settle_agent_role_cost(settlement(req.reservation_id, Decimal::ONE, month(10)))
        .await
        .unwrap();

    let mut late = request(&role, Some(Decimal::ONE), Some(Decimal::ONE));
    late.period_start = month(9);
    let admission = store.reserve_agent_role_cost(late).await.unwrap();
    assert!(matches!(admission, RoleCostAdmission::Refused { .. }));

    let mut next = request(&role, Some(Decimal::ONE), Some(Decimal::ONE));
    next.period_start = month(11);
    let admission = store.reserve_agent_role_cost(next).await.unwrap();
    assert!(matches!(
        admission,
        RoleCostAdmission::Admitted { spent_usd, .. } if spent_usd == Decimal::ZERO
    ));
    let record = store.get_agent_role(&role).await.unwrap().unwrap();
    assert_eq!(record.period_start, month(11));
}

#[tokio::test]
#[ignore]
async fn a_step_without_budget_holds_the_remaining_room_until_it_expires() {
    let store = get_store().await;
    let role = unique_role("unbudgeted");
    let mut held = request(&role, Some(Decimal::TEN), None);
    held.ttl_secs = 600;
    let admission = store.reserve_agent_role_cost(held.clone()).await.unwrap();
    assert!(matches!(
        admission,
        RoleCostAdmission::Admitted { amount_usd, .. } if amount_usd == Decimal::TEN
    ));
    let refused = store
        .reserve_agent_role_cost(request(&role, Some(Decimal::TEN), Some(Decimal::ONE)))
        .await
        .unwrap();
    assert!(matches!(refused, RoleCostAdmission::Refused { .. }));

    // A zero lease makes it expire at once.
    store
        .renew_agent_role_reservation(held.reservation_id, 0)
        .await
        .unwrap();
    let admitted = store
        .reserve_agent_role_cost(request(&role, Some(Decimal::TEN), Some(Decimal::ONE)))
        .await
        .unwrap();
    assert!(matches!(admitted, RoleCostAdmission::Admitted { .. }));
}

#[tokio::test]
#[ignore]
async fn operator_cap_only_lowers_the_code_cap() {
    let store = get_store().await;
    let role = unique_role("operator");
    let req = request(&role, Some(Decimal::TEN), Some(Decimal::from(15)));
    store.reserve_agent_role_cost(req.clone()).await.unwrap();
    store
        .settle_agent_role_cost(settlement(req.reservation_id, Decimal::from(15), month(10)))
        .await
        .unwrap();

    let record = store
        .set_agent_role_operator_cap(&role, Some(Decimal::from(20)))
        .await
        .unwrap();
    assert_eq!(record.operator_cap_usd, Some(Decimal::from(20)));
    assert_eq!(record.effective_cap_usd(), Some(Decimal::TEN));
    let refused = store
        .reserve_agent_role_cost(request(&role, Some(Decimal::TEN), Some(Decimal::ONE)))
        .await
        .unwrap();
    assert!(matches!(
        refused,
        RoleCostAdmission::Refused { cap_usd, .. } if cap_usd == Decimal::TEN
    ));

    let err = store
        .set_agent_role_operator_cap(&unique_role("ghost"), None)
        .await
        .expect_err("never seen");
    assert!(matches!(err, StoreError::AgentRoleNotFound(_)));
}

#[tokio::test]
#[ignore]
async fn list_reports_a_seen_role() {
    let store = get_store().await;
    let role = unique_role("listed");
    store
        .reserve_agent_role_cost(request(&role, None, Some(Decimal::ONE)))
        .await
        .unwrap();
    let record = store.get_agent_role(&role).await.unwrap().unwrap();
    assert_eq!(record.reserved_usd, Decimal::ONE);
    let page = store.list_agent_roles(1, 100).await.unwrap();
    assert!(page.total >= 1);
    assert!(page.items.len() <= 100);
}

#[tokio::test]
#[ignore]
async fn caps_and_budgets_above_a_million_are_stored() {
    let store = get_store().await;
    let role = unique_role("large");
    let cap = Decimal::from(5_000_000);
    let budget = Decimal::from(2_000_000);
    let req = request(&role, Some(cap), Some(budget));
    let admission = store.reserve_agent_role_cost(req.clone()).await.unwrap();
    assert!(matches!(
        admission,
        RoleCostAdmission::Admitted { amount_usd, .. } if amount_usd == budget
    ));
    store
        .settle_agent_role_cost(settlement(req.reservation_id, budget, month(10)))
        .await
        .unwrap();
    let record = store
        .set_agent_role_operator_cap(&role, Some(Decimal::from(3_000_000)))
        .await
        .unwrap();
    assert_eq!(record.spent_usd, budget);
    assert_eq!(record.code_cap_usd, Some(cap));
    assert_eq!(record.operator_cap_usd, Some(Decimal::from(3_000_000)));
}
