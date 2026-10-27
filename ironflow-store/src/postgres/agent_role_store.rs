//! PostgreSQL [`AgentRoleStore`] implementation.
//!
//! A reservation upserts the role row first: the upsert holds the row lock
//! until the transaction commits, which serializes the reservers of one role
//! across every worker. Every expiry is computed with the database clock.

use rust_decimal::Decimal;
use sqlx::postgres::PgRow;
use sqlx::{Error as SqlxError, Row};
use uuid::Uuid;

use crate::agent_role_store::AgentRoleStore;
use crate::entities::{
    AgentRoleRecord, Page, RoleCostAdmission, RoleCostReservationRequest, RoleCostSettlement,
    effective_cap,
};
use crate::error::StoreError;
use crate::store::StoreFuture;

use super::PostgresStore;

/// Select of a role with the room held by its live reservations, in
/// [`row_to_role`] order. Callers append a `WHERE` / `ORDER BY` clause.
const ROLE_SELECT: &str = "SELECT ar.id, ar.name, ar.code_cap_usd, ar.operator_cap_usd, \
     ar.period, ar.period_start, ar.spent_usd, live.reserved_usd, ar.created_at, ar.updated_at \
     FROM ironflow.agent_roles ar \
     LEFT JOIN LATERAL ( \
         SELECT COALESCE(SUM(r.amount_usd), 0) AS reserved_usd \
         FROM ironflow.agent_role_reservations r \
         WHERE r.role_id = ar.id AND r.expires_at > NOW() \
     ) live ON TRUE";

fn db_err(e: SqlxError) -> StoreError {
    StoreError::Database(e.to_string())
}

fn row_to_role(row: &PgRow) -> AgentRoleRecord {
    AgentRoleRecord {
        id: row.get("id"),
        name: row.get("name"),
        code_cap_usd: row.get("code_cap_usd"),
        operator_cap_usd: row.get("operator_cap_usd"),
        period: row.get("period"),
        period_start: row.get("period_start"),
        spent_usd: row.get("spent_usd"),
        reserved_usd: row.get("reserved_usd"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

impl PostgresStore {
    async fn fetch_agent_role(&self, name: &str) -> Result<Option<AgentRoleRecord>, StoreError> {
        let sql = format!("{ROLE_SELECT} WHERE ar.name = $1");
        let row = sqlx::query(&sql)
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(row_to_role))
    }
}

impl AgentRoleStore for PostgresStore {
    fn reserve_agent_role_cost(
        &self,
        request: RoleCostReservationRequest,
    ) -> StoreFuture<'_, RoleCostAdmission> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await.map_err(db_err)?;

            let role = sqlx::query(
                r#"
                INSERT INTO ironflow.agent_roles (id, name, code_cap_usd, period, period_start)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (name) DO UPDATE SET
                    code_cap_usd = EXCLUDED.code_cap_usd,
                    spent_usd = CASE
                        WHEN EXCLUDED.period_start > agent_roles.period_start THEN 0
                        ELSE agent_roles.spent_usd
                    END,
                    period_start = GREATEST(agent_roles.period_start, EXCLUDED.period_start),
                    period = EXCLUDED.period,
                    updated_at = NOW()
                RETURNING id, operator_cap_usd, spent_usd
                "#,
            )
            .bind(Uuid::now_v7())
            .bind(&request.role)
            .bind(request.code_cap_usd)
            .bind(&request.period)
            .bind(request.period_start)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?;
            let role_id: Uuid = role.get("id");
            let operator_cap_usd: Option<Decimal> = role.get("operator_cap_usd");
            let spent_usd: Decimal = role.get("spent_usd");
            let cap_usd = effective_cap(request.code_cap_usd, operator_cap_usd);

            let existing: Option<Decimal> = sqlx::query_scalar(
                "SELECT amount_usd FROM ironflow.agent_role_reservations WHERE id = $1",
            )
            .bind(request.reservation_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;

            sqlx::query(
                "DELETE FROM ironflow.agent_role_reservations \
                 WHERE role_id = $1 AND expires_at < NOW() - INTERVAL '1 day'",
            )
            .bind(role_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;

            let reserved_usd: Decimal = sqlx::query_scalar(
                "SELECT COALESCE(SUM(amount_usd), 0) FROM ironflow.agent_role_reservations \
                 WHERE role_id = $1 AND expires_at > NOW() AND id <> $2",
            )
            .bind(role_id)
            .bind(request.reservation_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?;

            if let Some(amount_usd) = existing {
                tx.commit().await.map_err(db_err)?;
                return Ok(RoleCostAdmission::Admitted {
                    reservation_id: request.reservation_id,
                    amount_usd,
                    cap_usd,
                    spent_usd,
                    reserved_usd,
                });
            }

            let amount_usd = match cap_usd {
                None => request.budget_usd.unwrap_or(Decimal::ZERO),
                Some(cap_usd) if spent_usd + reserved_usd >= cap_usd => {
                    // Commit the upsert: the role is visible to operators even
                    // when its first step is refused.
                    tx.commit().await.map_err(db_err)?;
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

            sqlx::query(
                "INSERT INTO ironflow.agent_role_reservations (id, role_id, amount_usd, expires_at) \
                 VALUES ($1, $2, $3, NOW() + make_interval(secs => $4))",
            )
            .bind(request.reservation_id)
            .bind(role_id)
            .bind(amount_usd)
            .bind(f64::from(request.ttl_secs))
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
            tx.commit().await.map_err(db_err)?;

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
            sqlx::query(
                "UPDATE ironflow.agent_role_reservations \
                 SET expires_at = NOW() + make_interval(secs => $2), updated_at = NOW() \
                 WHERE id = $1",
            )
            .bind(id)
            .bind(f64::from(ttl_secs))
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
            Ok(())
        })
    }

    fn settle_agent_role_cost(&self, settlement: RoleCostSettlement) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await.map_err(db_err)?;
            let role_id: Option<Uuid> = sqlx::query_scalar(
                "DELETE FROM ironflow.agent_role_reservations WHERE id = $1 RETURNING role_id",
            )
            .bind(settlement.reservation_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;

            if let Some(role_id) = role_id {
                sqlx::query(
                    r#"
                    UPDATE ironflow.agent_roles SET
                        spent_usd = CASE
                            WHEN $2 > period_start THEN $4
                            ELSE spent_usd + $4
                        END,
                        period_start = GREATEST(period_start, $2),
                        period = $3,
                        updated_at = NOW()
                    WHERE id = $1
                    "#,
                )
                .bind(role_id)
                .bind(settlement.period_start)
                .bind(&settlement.period)
                .bind(settlement.cost_usd)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
            }
            tx.commit().await.map_err(db_err)?;
            Ok(())
        })
    }

    fn get_agent_role(&self, name: &str) -> StoreFuture<'_, Option<AgentRoleRecord>> {
        let name = name.to_string();
        Box::pin(async move { self.fetch_agent_role(&name).await })
    }

    fn list_agent_roles(&self, page: u32, per_page: u32) -> StoreFuture<'_, Page<AgentRoleRecord>> {
        Box::pin(async move {
            let page = page.max(1);
            let per_page = per_page.clamp(1, 100);

            let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ironflow.agent_roles")
                .fetch_one(&self.pool)
                .await
                .map_err(db_err)?;

            let sql = format!("{ROLE_SELECT} ORDER BY ar.name LIMIT $1 OFFSET $2");
            let rows = sqlx::query(&sql)
                .bind(i64::from(per_page))
                .bind(i64::from(page - 1) * i64::from(per_page))
                .fetch_all(&self.pool)
                .await
                .map_err(db_err)?;

            Ok(Page {
                items: rows.iter().map(row_to_role).collect(),
                total: total as u64,
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
            let updated = sqlx::query(
                "UPDATE ironflow.agent_roles SET operator_cap_usd = $2, updated_at = NOW() \
                 WHERE name = $1",
            )
            .bind(&name)
            .bind(cap_usd)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
            if updated.rows_affected() == 0 {
                return Err(StoreError::AgentRoleNotFound(name));
            }
            self.fetch_agent_role(&name)
                .await?
                .ok_or(StoreError::AgentRoleNotFound(name))
        })
    }
}
