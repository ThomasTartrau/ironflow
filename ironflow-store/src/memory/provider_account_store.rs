//! In-memory [`ProviderAccountStore`] implementation.

use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::entities::{
    NewProviderAccount, NewProviderAccountObservation, Page, ProviderAccount,
    ProviderAccountCandidate, ProviderAccountUpdate, ProviderAccountUsagePoint,
    ProviderAccountWindow, StepStatus,
};
use crate::error::StoreError;
use crate::memory::{InMemoryStore, State};
use crate::provider_account_store::ProviderAccountStore;
use crate::store::StoreFuture;

fn by_priority_then_name(a: &ProviderAccount, b: &ProviderAccount) -> Ordering {
    a.priority
        .cmp(&b.priority)
        .then_with(|| a.name.cmp(&b.name))
}

fn windows_of(state: &State, ids: &[Uuid]) -> Vec<ProviderAccountWindow> {
    let mut windows: Vec<ProviderAccountWindow> = state
        .provider_account_windows
        .values()
        .filter(|w| ids.contains(&w.account_id))
        .cloned()
        .collect();
    windows.sort_by(|a, b| {
        a.account_id
            .cmp(&b.account_id)
            .then_with(|| a.window.cmp(&b.window))
            .then_with(|| a.model_scope.cmp(&b.model_scope))
    });
    windows
}

/// Running steps per account, counting only runs whose worker lease is live.
fn running_steps(state: &State, now: DateTime<Utc>) -> HashMap<Uuid, u32> {
    let mut counts = HashMap::new();
    for step in state.steps.values() {
        let Some(account_id) = step.account_id else {
            continue;
        };
        if step.status.state != StepStatus::Running {
            continue;
        }
        let leased = state
            .runs
            .get(&step.run_id)
            .and_then(|run| run.lease_expires_at)
            .is_some_and(|expires| expires > now);
        if leased {
            *counts.entry(account_id).or_insert(0) += 1;
        }
    }
    counts
}

impl ProviderAccountStore for InMemoryStore {
    fn create_provider_account(&self, req: NewProviderAccount) -> StoreFuture<'_, ProviderAccount> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            if state.provider_accounts.values().any(|a| a.name == req.name) {
                return Err(StoreError::DuplicateProviderAccount(req.name));
            }
            let now = Utc::now();
            let account = ProviderAccount {
                id: req.id,
                name: req.name,
                display_name: req.display_name,
                kind: req.kind,
                secret_key: req.secret_key,
                enabled: req.enabled,
                priority: req.priority,
                tags: req.tags,
                max_concurrency: req.max_concurrency,
                alert_threshold: req.alert_threshold,
                expires_at: req.expires_at,
                plan: req.plan,
                auth_failed_at: None,
                created_by: req.created_by,
                created_at: now,
                updated_at: now,
            };
            state.provider_accounts.insert(account.id, account.clone());
            Ok(account)
        })
    }

    fn get_provider_account(&self, id: Uuid) -> StoreFuture<'_, Option<ProviderAccount>> {
        Box::pin(async move {
            let state = self.state.read().await;
            Ok(state.provider_accounts.get(&id).cloned())
        })
    }

    fn find_provider_account_by_name(
        &self,
        name: &str,
    ) -> StoreFuture<'_, Option<ProviderAccount>> {
        let name = name.to_string();
        Box::pin(async move {
            let state = self.state.read().await;
            Ok(state
                .provider_accounts
                .values()
                .find(|a| a.name == name)
                .cloned())
        })
    }

    fn list_provider_accounts(
        &self,
        kind: Option<String>,
        page: u32,
        per_page: u32,
    ) -> StoreFuture<'_, Page<ProviderAccount>> {
        Box::pin(async move {
            let state = self.state.read().await;
            let mut all: Vec<ProviderAccount> = state
                .provider_accounts
                .values()
                .filter(|a| kind.as_ref().is_none_or(|k| &a.kind == k))
                .cloned()
                .collect();
            all.sort_by(by_priority_then_name);
            let total = all.len() as u64;
            let start = (page.saturating_sub(1) as usize) * (per_page as usize);
            let items = all
                .into_iter()
                .skip(start)
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

    fn update_provider_account(
        &self,
        id: Uuid,
        update: ProviderAccountUpdate,
    ) -> StoreFuture<'_, ProviderAccount> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            let account = state
                .provider_accounts
                .get_mut(&id)
                .ok_or(StoreError::ProviderAccountNotFound(id))?;
            if let Some(display_name) = update.display_name {
                account.display_name = display_name;
            }
            if let Some(enabled) = update.enabled {
                account.enabled = enabled;
            }
            if let Some(priority) = update.priority {
                account.priority = priority;
            }
            if let Some(tags) = update.tags {
                account.tags = tags;
            }
            if let Some(max_concurrency) = update.max_concurrency {
                account.max_concurrency = max_concurrency;
            }
            if let Some(alert_threshold) = update.alert_threshold {
                account.alert_threshold = alert_threshold;
            }
            if let Some(expires_at) = update.expires_at {
                account.expires_at = expires_at;
            }
            if let Some(plan) = update.plan {
                account.plan = plan;
            }
            if let Some(auth_failed_at) = update.auth_failed_at {
                account.auth_failed_at = auth_failed_at;
            }
            account.updated_at = Utc::now();
            Ok(account.clone())
        })
    }

    fn delete_provider_account(&self, id: Uuid) -> StoreFuture<'_, bool> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            if state.provider_accounts.remove(&id).is_none() {
                return Ok(false);
            }
            state
                .provider_account_windows
                .retain(|(account_id, _, _), _| *account_id != id);
            state.provider_account_usage.retain(|u| u.account_id != id);
            for step in state.steps.values_mut() {
                if step.account_id == Some(id) {
                    step.account_id = None;
                }
            }
            Ok(true)
        })
    }

    fn list_provider_account_windows(
        &self,
        ids: Vec<Uuid>,
    ) -> StoreFuture<'_, Vec<ProviderAccountWindow>> {
        Box::pin(async move {
            let state = self.state.read().await;
            Ok(windows_of(&state, &ids))
        })
    }

    fn list_provider_account_usage(
        &self,
        id: Uuid,
        since: DateTime<Utc>,
    ) -> StoreFuture<'_, Vec<ProviderAccountUsagePoint>> {
        Box::pin(async move {
            let state = self.state.read().await;
            let mut points: Vec<ProviderAccountUsagePoint> = state
                .provider_account_usage
                .iter()
                .filter(|u| u.account_id == id && u.observed_at >= since)
                .cloned()
                .collect();
            points.sort_by_key(|u| u.observed_at);
            Ok(points)
        })
    }

    fn record_provider_account_observation(
        &self,
        id: Uuid,
        observation: NewProviderAccountObservation,
    ) -> StoreFuture<'_, Vec<ProviderAccountWindow>> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            if !state.provider_accounts.contains_key(&id) {
                return Err(StoreError::ProviderAccountNotFound(id));
            }
            let recorded = !observation.windows.is_empty();
            for window in observation.windows {
                let key = (
                    id,
                    window.window.clone(),
                    window.model_scope.clone().unwrap_or_default(),
                );
                let newer_stored = state
                    .provider_account_windows
                    .get(&key)
                    .is_some_and(|stored| stored.observed_at > window.observed_at);
                if !newer_stored {
                    state.provider_account_windows.insert(
                        key,
                        ProviderAccountWindow {
                            account_id: id,
                            window: window.window.clone(),
                            utilization: window.utilization,
                            resets_at: window.resets_at,
                            status: window.status,
                            model_scope: window.model_scope.clone(),
                            observed_at: window.observed_at,
                        },
                    );
                }
                state
                    .provider_account_usage
                    .push(ProviderAccountUsagePoint {
                        id: Uuid::now_v7(),
                        account_id: id,
                        window: window.window,
                        utilization: window.utilization,
                        resets_at: window.resets_at,
                        status: window.status,
                        model_scope: window.model_scope,
                        observed_at: window.observed_at,
                    });
            }
            let now = Utc::now();
            if let Some(account) = state.provider_accounts.get_mut(&id) {
                if observation.auth_failed {
                    account.auth_failed_at = Some(now);
                    account.updated_at = now;
                } else if recorded {
                    account.auth_failed_at = None;
                }
            }
            Ok(windows_of(&state, &[id]))
        })
    }

    fn purge_provider_account_usage(&self, before: DateTime<Utc>) -> StoreFuture<'_, u64> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            let len_before = state.provider_account_usage.len();
            state
                .provider_account_usage
                .retain(|u| u.observed_at >= before);
            Ok((len_before - state.provider_account_usage.len()) as u64)
        })
    }

    fn list_provider_account_candidates(
        &self,
        kind: String,
    ) -> StoreFuture<'_, Vec<ProviderAccountCandidate>> {
        Box::pin(async move {
            let state = self.state.read().await;
            let now = Utc::now();
            let mut accounts: Vec<ProviderAccount> = state
                .provider_accounts
                .values()
                .filter(|a| {
                    a.kind == kind && a.enabled && a.auth_failed_at.is_none() && a.expires_at > now
                })
                .cloned()
                .collect();
            accounts.sort_by(by_priority_then_name);
            let running = running_steps(&state, now);
            Ok(accounts
                .into_iter()
                .map(|account| {
                    let windows = windows_of(&state, &[account.id]);
                    let running_steps = running.get(&account.id).copied().unwrap_or(0);
                    ProviderAccountCandidate {
                        account,
                        windows,
                        running_steps,
                    }
                })
                .collect())
        })
    }
}
