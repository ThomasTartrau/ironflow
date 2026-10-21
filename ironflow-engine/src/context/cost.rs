//! Live cost of a run tree whose sub-workflows run concurrently.
//!
//! A sequential chain checks the cost cap against what its ancestors spent
//! before it started: nothing else runs meanwhile. The children of a
//! [`workflows_with`](super::WorkflowContext::workflows_with) wave run at the
//! same time, so each of them, and every run below them, reads and writes one
//! shared total instead.

use std::sync::{Arc, Mutex, MutexGuard};

use rust_decimal::Decimal;

/// Cost of a run tree, shared by the contexts of a wave and their descendants.
#[derive(Debug, Clone)]
pub(crate) struct SharedCost(Arc<Mutex<CostLedger>>);

#[derive(Debug)]
struct CostLedger {
    /// Cost already reported by the steps of the tree.
    spent: Decimal,
    /// Declared budget of the agent steps admitted under the cap and not
    /// charged yet.
    reserved: Decimal,
}

impl SharedCost {
    /// A total starting from what the tree already spent.
    pub(crate) fn new(spent: Decimal) -> Self {
        Self(Arc::new(Mutex::new(CostLedger {
            spent,
            reserved: Decimal::ZERO,
        })))
    }

    /// Add the cost a step reported.
    pub(crate) fn charge(&self, cost: Decimal) {
        self.lock().spent += cost;
    }

    /// Cost reported so far by the whole tree.
    pub(crate) fn spent(&self) -> Decimal {
        self.lock().spent
    }

    /// Admit a step declaring `budget` when the cost spent and the budgets
    /// already reserved leave room for it under `limit`.
    ///
    /// The budget stays reserved until the returned [`BudgetReservation`] is
    /// dropped, so two concurrent steps never both claim the same room.
    ///
    /// # Errors
    ///
    /// Returns what the tree has spent plus reserved when `budget` does not fit.
    pub(crate) fn reserve(
        &self,
        budget: Decimal,
        limit: Decimal,
    ) -> Result<BudgetReservation, Decimal> {
        let mut ledger = self.lock();
        let committed = ledger.spent + ledger.reserved;
        if committed + budget > limit {
            return Err(committed);
        }
        ledger.reserved += budget;
        Ok(BudgetReservation {
            ledger: Some(self.clone()),
            amount: budget,
        })
    }

    /// Lock the ledger, ignoring poisoning: a panicking step must not turn
    /// every later cost check into a panic of its own.
    fn lock(&self) -> MutexGuard<'_, CostLedger> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Budget held for an admitted agent step, released when dropped.
///
/// Held until the step's cost is charged: from then on the cost itself
/// counts against the cap.
#[must_use = "the budget is released as soon as the reservation is dropped"]
pub(crate) struct BudgetReservation {
    ledger: Option<SharedCost>,
    amount: Decimal,
}

impl BudgetReservation {
    /// A reservation that holds nothing: the run shares no live total.
    pub(crate) fn none() -> Self {
        Self {
            ledger: None,
            amount: Decimal::ZERO,
        }
    }
}

impl Drop for BudgetReservation {
    fn drop(&mut self) {
        if let Some(ledger) = &self.ledger {
            ledger.lock().reserved -= self.amount;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reservation_holds_its_budget_until_dropped() {
        let cost = SharedCost::new(Decimal::new(5, 2));
        let limit = Decimal::new(20, 2);

        let first = cost
            .reserve(Decimal::new(10, 2), limit)
            .expect("0.05 + 0.10 fits under 0.20");
        assert_eq!(
            cost.reserve(Decimal::new(10, 2), limit).err(),
            Some(Decimal::new(15, 2)),
            "the second step sees the first one's budget"
        );

        drop(first);
        let _second = cost
            .reserve(Decimal::new(10, 2), limit)
            .expect("the budget is free again");
    }

    #[test]
    fn charged_cost_counts_against_the_next_reservation() {
        let cost = SharedCost::new(Decimal::ZERO);
        cost.charge(Decimal::new(15, 2));
        assert_eq!(cost.spent(), Decimal::new(15, 2));
        assert!(
            cost.reserve(Decimal::new(10, 2), Decimal::new(20, 2))
                .is_err()
        );
        let _exact = cost
            .reserve(Decimal::new(5, 2), Decimal::new(20, 2))
            .expect("a budget that reaches the cap exactly fits");
    }
}
