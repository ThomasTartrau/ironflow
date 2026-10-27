//! Agent role request and response DTOs.

use std::str::FromStr;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize};

use ironflow_core::agent_role::CostPeriod;
use ironflow_store::entities::AgentRoleRecord;

use crate::error::ApiError;

/// Cost consumed by an agent role over its current period, and its cap.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_api::entities::AgentRoleResponse;
/// use ironflow_store::entities::AgentRoleRecord;
/// use rust_decimal::Decimal;
/// use uuid::Uuid;
///
/// # fn example() -> Result<(), ironflow_api::error::ApiError> {
/// let now = Utc::now();
/// let record = AgentRoleRecord {
///     id: Uuid::now_v7(),
///     name: "reviewer".to_string(),
///     code_cap_usd: Some(Decimal::TEN),
///     operator_cap_usd: None,
///     period: "month".to_string(),
///     period_start: now,
///     spent_usd: Decimal::TWO,
///     reserved_usd: Decimal::ONE,
///     created_at: now,
///     updated_at: now,
/// };
/// let response = AgentRoleResponse::from_record(record, now)?;
/// assert_eq!(response.remaining_usd, Some(Decimal::new(7, 0)));
/// # Ok(())
/// # }
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentRoleResponse {
    /// Name of the role, as declared by `AgentRole::name`.
    pub name: String,
    /// Period of the counter: `day` or `month` (calendar, UTC).
    pub period: String,
    /// Start of the current period.
    pub period_start: DateTime<Utc>,
    /// When the counter resets.
    pub resets_at: DateTime<Utc>,
    /// Cap declared in code, as last seen by a step of the role.
    pub code_cap_usd: Option<Decimal>,
    /// Cap set by an operator; it can only lower the code cap.
    pub operator_cap_usd: Option<Decimal>,
    /// Cap enforced: the lower of the code and operator caps.
    pub cap_usd: Option<Decimal>,
    /// Cost settled in the current period.
    pub spent_usd: Decimal,
    /// Cost held by the steps of the role running now.
    pub reserved_usd: Decimal,
    /// Room left under the cap, never negative. `null` without a cap.
    pub remaining_usd: Option<Decimal>,
    /// Last change of the counter or of a cap.
    pub updated_at: DateTime<Utc>,
}

impl AgentRoleResponse {
    /// Build the response at `now`. A counter whose period is over reads as
    /// zero over the current period: the store only resets it on the next
    /// step of the role.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::Internal`] if the stored period is unknown.
    ///
    /// # Examples
    ///
    /// See [`AgentRoleResponse`].
    pub fn from_record(record: AgentRoleRecord, now: DateTime<Utc>) -> Result<Self, ApiError> {
        let period = CostPeriod::from_str(&record.period).map_err(|e| {
            ApiError::Internal(format!(
                "agent role '{}' has an unknown period '{}': {e}",
                record.name, record.period
            ))
        })?;
        let current = period.start(now);
        let (period_start, spent_usd) = if record.period_start < current {
            (current, Decimal::ZERO)
        } else {
            (record.period_start, record.spent_usd)
        };
        let cap_usd = record.effective_cap_usd();
        let remaining_usd =
            cap_usd.map(|cap| (cap - spent_usd - record.reserved_usd).max(Decimal::ZERO));
        Ok(Self {
            name: record.name,
            period: period.as_str().to_string(),
            period_start,
            resets_at: period.next_start(now),
            code_cap_usd: record.code_cap_usd,
            operator_cap_usd: record.operator_cap_usd,
            cap_usd,
            spent_usd,
            reserved_usd: record.reserved_usd,
            remaining_usd,
            updated_at: record.updated_at,
        })
    }
}

/// Query parameters for listing agent roles.
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams, utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
pub struct ListAgentRolesQuery {
    /// Page number (1-based, defaults to 1).
    pub page: Option<u32>,
    /// Items per page (defaults to 50, max 100).
    pub per_page: Option<u32>,
}

/// Set or clear the operator cap of an agent role.
///
/// The field is required and unknown fields are refused: only an explicit
/// `null` clears the cap, so `{}` or a misspelled key cannot raise the
/// enforced cap back to the code cap.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateAgentRoleCostCapRequest {
    /// Cap in USD over the role's period, at least 0; `null` clears it. It can
    /// only lower the cap declared in code.
    #[serde(deserialize_with = "deserialize_required_option")]
    #[cfg_attr(feature = "openapi", schema(required = true))]
    pub operator_cap_usd: Option<Decimal>,
}

/// Deserialize an `Option` without serde's "missing means `None`" fallback:
/// with a `deserialize_with` and no `default`, a missing key is an error.
fn deserialize_required_option<'de, D>(deserializer: D) -> Result<Option<Decimal>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<Decimal>::deserialize(deserializer)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::from_str;
    use uuid::Uuid;

    use super::*;

    fn record(period_start: DateTime<Utc>) -> AgentRoleRecord {
        AgentRoleRecord {
            id: Uuid::now_v7(),
            name: "reviewer".to_string(),
            code_cap_usd: Some(Decimal::TEN),
            operator_cap_usd: Some(Decimal::from(20)),
            period: "month".to_string(),
            period_start,
            spent_usd: Decimal::from(4),
            reserved_usd: Decimal::ONE,
            created_at: period_start,
            updated_at: period_start,
        }
    }

    fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).single().unwrap()
    }

    #[test]
    fn reports_spent_reserved_and_remaining_under_the_lower_cap() {
        let response = AgentRoleResponse::from_record(record(at(2026, 10, 1)), at(2026, 10, 27))
            .expect("valid record");
        assert_eq!(response.cap_usd, Some(Decimal::TEN));
        assert_eq!(response.spent_usd, Decimal::from(4));
        assert_eq!(response.reserved_usd, Decimal::ONE);
        assert_eq!(response.remaining_usd, Some(Decimal::from(5)));
        assert_eq!(response.resets_at, at(2026, 11, 1));
    }

    #[test]
    fn a_stale_period_reads_as_zero() {
        let response = AgentRoleResponse::from_record(record(at(2026, 9, 1)), at(2026, 10, 27))
            .expect("valid record");
        assert_eq!(response.spent_usd, Decimal::ZERO);
        assert_eq!(response.period_start, at(2026, 10, 1));
        assert_eq!(response.remaining_usd, Some(Decimal::from(9)));
    }

    #[test]
    fn remaining_floors_at_zero_and_is_null_without_cap() {
        let mut over = record(at(2026, 10, 1));
        over.spent_usd = Decimal::from(15);
        let response =
            AgentRoleResponse::from_record(over, at(2026, 10, 27)).expect("valid record");
        assert_eq!(response.remaining_usd, Some(Decimal::ZERO));

        let mut uncapped = record(at(2026, 10, 1));
        uncapped.code_cap_usd = None;
        uncapped.operator_cap_usd = None;
        let response =
            AgentRoleResponse::from_record(uncapped, at(2026, 10, 27)).expect("valid record");
        assert_eq!(response.remaining_usd, None);
        assert_eq!(response.cap_usd, None);
    }

    #[test]
    fn cap_request_requires_the_field_and_refuses_unknown_ones() {
        let set: UpdateAgentRoleCostCapRequest =
            from_str(r#"{"operator_cap_usd": 5}"#).expect("cap");
        assert_eq!(set.operator_cap_usd, Some(Decimal::from(5)));
        let clear: UpdateAgentRoleCostCapRequest =
            from_str(r#"{"operator_cap_usd": null}"#).expect("explicit null");
        assert_eq!(clear.operator_cap_usd, None);

        assert!(from_str::<UpdateAgentRoleCostCapRequest>("{}").is_err());
        assert!(from_str::<UpdateAgentRoleCostCapRequest>(r#"{"operator_cap": 5}"#).is_err());
    }

    #[test]
    fn an_unknown_period_is_an_internal_error() {
        let mut broken = record(at(2026, 10, 1));
        broken.period = "week".to_string();
        let err = AgentRoleResponse::from_record(broken, at(2026, 10, 27)).expect_err("unknown");
        assert!(matches!(err, ApiError::Internal(ref m) if m.contains("week")));
    }
}
