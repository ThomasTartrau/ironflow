//! Agent role entities -- the cost counter of a role over its period.
//!
//! An agent role declared in code with a name is charged, step by step, to
//! one counter shared by every worker. A step first reserves room under the
//! cap ([`RoleCostReservationRequest`] -> [`RoleCostAdmission`]), then settles
//! the cost it reported ([`RoleCostSettlement`]).

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Request to reserve room for one agent step of a role.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_store::entities::RoleCostReservationRequest;
/// use rust_decimal::Decimal;
/// use uuid::Uuid;
///
/// let request = RoleCostReservationRequest {
///     reservation_id: Uuid::now_v7(),
///     role: "reviewer".to_string(),
///     code_cap_usd: Some(Decimal::TEN),
///     period: "month".to_string(),
///     period_start: Utc::now(),
///     budget_usd: Some(Decimal::ONE),
///     ttl_secs: 600,
/// };
/// assert_eq!(request.role, "reviewer");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleCostReservationRequest {
    /// ID of the reservation, chosen by the caller: reserving twice with the
    /// same ID returns the first admission.
    pub reservation_id: Uuid,
    /// Name of the role.
    pub role: String,
    /// Cap declared in code by the caller, enforced for this request.
    pub code_cap_usd: Option<Decimal>,
    /// Period of the counter: `"day"` or `"month"`.
    pub period: String,
    /// Start of the current period, as seen by the caller.
    pub period_start: DateTime<Utc>,
    /// `max_budget_usd` of the step; `None` reserves all the remaining room.
    pub budget_usd: Option<Decimal>,
    /// Lifetime of the reservation, in seconds, renewed while the step runs.
    pub ttl_secs: u32,
}

/// Outcome of a reservation request.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::RoleCostAdmission;
/// use rust_decimal::Decimal;
///
/// let refused = RoleCostAdmission::Refused {
///     cap_usd: Decimal::ONE,
///     spent_usd: Decimal::ONE,
///     reserved_usd: Decimal::ZERO,
/// };
/// assert!(matches!(refused, RoleCostAdmission::Refused { .. }));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RoleCostAdmission {
    /// The step may run; `amount_usd` is held until it settles.
    Admitted {
        /// ID of the reservation.
        reservation_id: Uuid,
        /// Room held by the reservation.
        amount_usd: Decimal,
        /// Effective cap, `None` when the role has none.
        cap_usd: Option<Decimal>,
        /// Cost settled in the current period, before this step.
        spent_usd: Decimal,
        /// Room held by the other running steps of the role.
        reserved_usd: Decimal,
    },
    /// The cap is reached: the step must not run.
    Refused {
        /// Effective cap.
        cap_usd: Decimal,
        /// Cost settled in the current period.
        spent_usd: Decimal,
        /// Room held by the running steps of the role.
        reserved_usd: Decimal,
    },
}

/// Cost of a finished agent step, releasing its reservation.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_store::entities::RoleCostSettlement;
/// use rust_decimal::Decimal;
/// use uuid::Uuid;
///
/// let settlement = RoleCostSettlement {
///     reservation_id: Uuid::now_v7(),
///     period: "day".to_string(),
///     period_start: Utc::now(),
///     cost_usd: Decimal::new(42, 2),
/// };
/// assert_eq!(settlement.cost_usd.to_string(), "0.42");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleCostSettlement {
    /// ID of the reservation to release.
    pub reservation_id: Uuid,
    /// Period of the counter: `"day"` or `"month"`.
    pub period: String,
    /// Start of the period the cost is counted in.
    pub period_start: DateTime<Utc>,
    /// Cost the step reported, in USD.
    pub cost_usd: Decimal,
}

/// The cost counter of an agent role.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_store::entities::AgentRoleRecord;
/// use rust_decimal::Decimal;
/// use uuid::Uuid;
///
/// let record = AgentRoleRecord {
///     id: Uuid::now_v7(),
///     name: "reviewer".to_string(),
///     code_cap_usd: Some(Decimal::TEN),
///     operator_cap_usd: Some(Decimal::ONE),
///     period: "month".to_string(),
///     period_start: Utc::now(),
///     spent_usd: Decimal::ZERO,
///     reserved_usd: Decimal::ZERO,
///     created_at: Utc::now(),
///     updated_at: Utc::now(),
/// };
/// assert_eq!(record.effective_cap_usd(), Some(Decimal::ONE));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRoleRecord {
    /// Unique ID (UUID v7).
    pub id: Uuid,
    /// Name of the role.
    pub name: String,
    /// Cap declared in code, as last seen by a reservation (display only:
    /// each reservation enforces the cap it carries).
    pub code_cap_usd: Option<Decimal>,
    /// Cap set by an operator; it can only lower the code cap.
    pub operator_cap_usd: Option<Decimal>,
    /// Period of the counter: `"day"` or `"month"`.
    pub period: String,
    /// Start of the period `spent_usd` is counted over.
    pub period_start: DateTime<Utc>,
    /// Cost settled over the period starting at `period_start`.
    pub spent_usd: Decimal,
    /// Room held by the live reservations of the role, computed on read.
    pub reserved_usd: Decimal,
    /// When the role was first seen.
    pub created_at: DateTime<Utc>,
    /// Last change of the counter or of a cap.
    pub updated_at: DateTime<Utc>,
}

impl AgentRoleRecord {
    /// The cap enforced for the role: the lower of the code and operator caps.
    ///
    /// # Examples
    ///
    /// See [`AgentRoleRecord`].
    pub fn effective_cap_usd(&self) -> Option<Decimal> {
        effective_cap(self.code_cap_usd, self.operator_cap_usd)
    }
}

/// The lower of two optional caps: an operator cap can lower the code cap,
/// never raise it, and either applies alone when the other is `None`.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::effective_cap;
/// use rust_decimal::Decimal;
///
/// assert_eq!(effective_cap(Some(Decimal::TEN), Some(Decimal::TWO)), Some(Decimal::TWO));
/// assert_eq!(effective_cap(Some(Decimal::TEN), Some(Decimal::ONE_HUNDRED)), Some(Decimal::TEN));
/// assert_eq!(effective_cap(None, Some(Decimal::ONE)), Some(Decimal::ONE));
/// assert_eq!(effective_cap(None, None), None);
/// ```
pub fn effective_cap(code: Option<Decimal>, operator: Option<Decimal>) -> Option<Decimal> {
    match (code, operator) {
        (Some(code), Some(operator)) => Some(code.min(operator)),
        (code, operator) => code.or(operator),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value, json, to_value};

    use super::*;

    #[test]
    fn operator_cap_only_lowers_the_code_cap() {
        let ten = Some(Decimal::TEN);
        let twenty = Some(Decimal::from(20));
        assert_eq!(effective_cap(ten, twenty), ten);
        assert_eq!(effective_cap(twenty, ten), ten);
        assert_eq!(effective_cap(ten, Some(Decimal::ZERO)), Some(Decimal::ZERO));
    }

    #[test]
    fn a_single_cap_applies_alone() {
        assert_eq!(effective_cap(Some(Decimal::ONE), None), Some(Decimal::ONE));
        assert_eq!(effective_cap(None, Some(Decimal::TWO)), Some(Decimal::TWO));
        assert_eq!(effective_cap(None, None), None);
    }

    #[test]
    fn admission_is_tagged_by_outcome() {
        let refused = RoleCostAdmission::Refused {
            cap_usd: Decimal::ONE,
            spent_usd: Decimal::ONE,
            reserved_usd: Decimal::ZERO,
        };
        let value = to_value(&refused).unwrap();
        assert_eq!(value["outcome"], json!("refused"));
        assert_eq!(from_value::<RoleCostAdmission>(value).unwrap(), refused);
    }
}
