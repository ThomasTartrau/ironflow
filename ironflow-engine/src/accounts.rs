//! Provider Account selection and injection for agent steps.
//!
//! [`AccountAwareProvider`] wraps the worker's [`AgentProvider`]. For each
//! agent invocation on a provider that can inject a Provider Account
//! credential for the invocation ([`AgentProvider::account_kind_for`]), it:
//!
//! 1. lists the candidate accounts of that kind from the store, keeping the
//!    one named by [`AgentConfig::account_name`] or those tagged with
//!    [`AgentConfig::account_pool`],
//! 2. picks one with an [`AccountStrategy`],
//! 3. reads its credential and runs the invocation under it,
//! 4. records the usage windows observed during the invocation and, when a
//!    rate-limited window rejected it, tries the next account (never with
//!    `account_name`).
//!
//! When no targeted account can take the step, it decides between
//! [`AgentError::CapacityWait`] (the engine puts the run to sleep until the
//! earliest reset, or 60 seconds when only `max_concurrency` blocks) and
//! [`AgentError::NoCapacity`] (the wait would exceed the step's
//! [`AgentConfig::max_capacity_wait`], [`DEFAULT_MAX_CAPACITY_WAIT`] by
//! default). It never runs the agent in that case.
//!
//! With no account of the kind in the store, the invocation runs with the
//! worker environment; a rate-limit rejection of that token is handled the
//! same way. A provider that cannot inject an account runs unchanged.
//!
//! # Examples
//!
//! ```no_run
//! use std::sync::Arc;
//! use ironflow_core::account_strategy::Priority;
//! use ironflow_core::providers::claude::ClaudeCodeProvider;
//! use ironflow_engine::accounts::AccountAwareProvider;
//! use ironflow_store::memory::InMemoryStore;
//!
//! let provider = AccountAwareProvider::new(
//!     Arc::new(ClaudeCodeProvider::new()),
//!     Arc::new(InMemoryStore::new()),
//! )
//! .with_strategy(Arc::new(Priority));
//! # let _ = provider;
//! ```

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use ironflow_core::account::{
    AccountKind, AccountSession, AccountWindow, ClaudeSubscriptionKind, RateLimitRecorder,
    WindowStatus,
};
use ironflow_core::account_strategy::{
    AccountCandidate, AccountStrategy, LeastUtilized, select_account,
};
use ironflow_core::error::AgentError;
use ironflow_core::provider::{
    AgentConfig, AgentOutput, AgentProvider, InvokeFuture, LogSink, ReleaseFuture,
};
use ironflow_store::entities::{
    AccountWindowStatus, NewAccountWindow, NewProviderAccountObservation, ProviderAccount,
    ProviderAccountCandidate, ProviderAccountWindow,
};
use ironflow_store::store::Store;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Convert a stored window into the core representation.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_engine::accounts::window_from_store;
/// use ironflow_store::entities::{AccountWindowStatus, ProviderAccountWindow};
/// use uuid::Uuid;
///
/// let window = window_from_store(&ProviderAccountWindow {
///     account_id: Uuid::now_v7(),
///     window: "five_hour".to_string(),
///     utilization: 0.4,
///     resets_at: None,
///     status: AccountWindowStatus::Allowed,
///     model_scope: None,
///     observed_at: Utc::now(),
/// });
/// assert_eq!(window.window, "five_hour");
/// ```
pub fn window_from_store(window: &ProviderAccountWindow) -> AccountWindow {
    AccountWindow {
        window: window.window.clone(),
        utilization: window.utilization,
        resets_at: window.resets_at,
        status: match window.status {
            AccountWindowStatus::Allowed => WindowStatus::Allowed,
            AccountWindowStatus::AllowedWarning => WindowStatus::AllowedWarning,
            AccountWindowStatus::Rejected => WindowStatus::Rejected,
        },
        model_scope: window.model_scope.clone(),
        observed_at: window.observed_at,
    }
}

/// Convert an observed core window into the store representation.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_core::account::{AccountWindow, WindowStatus};
/// use ironflow_engine::accounts::window_to_store;
/// use ironflow_store::entities::AccountWindowStatus;
///
/// let window = window_to_store(AccountWindow {
///     window: "seven_day".to_string(),
///     utilization: 1.0,
///     resets_at: None,
///     status: WindowStatus::Rejected,
///     model_scope: Some("opus".to_string()),
///     observed_at: Utc::now(),
/// });
/// assert_eq!(window.status, AccountWindowStatus::Rejected);
/// ```
pub fn window_to_store(window: AccountWindow) -> NewAccountWindow {
    NewAccountWindow {
        window: window.window,
        utilization: window.utilization,
        resets_at: window.resets_at,
        status: match window.status {
            WindowStatus::Allowed => AccountWindowStatus::Allowed,
            WindowStatus::AllowedWarning => AccountWindowStatus::AllowedWarning,
            WindowStatus::Rejected => AccountWindowStatus::Rejected,
        },
        model_scope: window.model_scope,
        observed_at: window.observed_at,
    }
}

fn to_core_candidate(candidate: &ProviderAccountCandidate) -> AccountCandidate {
    AccountCandidate {
        id: candidate.account.id.to_string(),
        name: candidate.account.name.clone(),
        priority: candidate.account.priority,
        max_concurrency: candidate.account.max_concurrency,
        running_steps: candidate.running_steps,
        windows: candidate.windows.iter().map(window_from_store).collect(),
    }
}

fn resolution_error(message: String) -> AgentError {
    AgentError::ProcessFailed {
        exit_code: -1,
        stderr: message,
    }
}

/// Longest time an agent step waits for provider capacity when neither the
/// step ([`AgentConfig::max_capacity_wait`]) nor the worker
/// ([`AccountAwareProvider::with_max_capacity_wait`]) sets one: 6 hours.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use ironflow_engine::accounts::DEFAULT_MAX_CAPACITY_WAIT;
///
/// assert_eq!(DEFAULT_MAX_CAPACITY_WAIT, Duration::from_secs(6 * 3600));
/// ```
pub const DEFAULT_MAX_CAPACITY_WAIT: Duration = Duration::from_secs(6 * 3600);

/// How long a step waits before trying again when every targeted account is
/// only blocked by its `max_concurrency`: running steps free no reset time.
const CONCURRENCY_RETRY: Duration = Duration::from_secs(60);

/// One account tried by a step, logged when the step fails over.
#[derive(Debug)]
struct AccountAttempt {
    account_id: Uuid,
    outcome: &'static str,
}

fn to_time_delta(duration: Duration) -> TimeDelta {
    TimeDelta::from_std(duration).unwrap_or(TimeDelta::MAX)
}

/// Decide between waiting and failing once no targeted account can take the
/// step.
///
/// `since` is when the step first parked on a capacity wait: the whole wait,
/// across wake-ups, stays within `wait`.
fn capacity_decision(
    kind: &str,
    next_reset: Option<DateTime<Utc>>,
    concurrency_blocked: bool,
    wait: Duration,
    since: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> AgentError {
    let kind = kind.to_string();
    if wait.is_zero() {
        return AgentError::NoCapacity { kind, next_reset };
    }
    let budget = to_time_delta(wait);
    let since = since.unwrap_or(now);
    match next_reset {
        Some(reset) if reset - since <= budget => AgentError::CapacityWait {
            kind,
            wake_at: reset,
        },
        Some(_) => AgentError::NoCapacity { kind, next_reset },
        None if concurrency_blocked => {
            let wake_at = now + to_time_delta(CONCURRENCY_RETRY);
            if wake_at - since <= budget {
                AgentError::CapacityWait { kind, wake_at }
            } else {
                AgentError::NoCapacity {
                    kind,
                    next_reset: None,
                }
            }
        }
        None => AgentError::NoCapacity {
            kind,
            next_reset: None,
        },
    }
}

/// Earliest reset of a rejected window constraining `model`.
fn earliest_reset<'a>(
    windows: impl Iterator<Item = &'a AccountWindow>,
    model: &str,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    windows
        .filter(|w| w.applies_to(model) && w.is_exhausted(now))
        .filter_map(|w| w.resets_at)
        .min()
}

/// Whether the step targets `account`: by name, by pool tag, or any.
fn is_targeted(config: &AgentConfig, account: &ProviderAccount) -> bool {
    if let Some(name) = &config.account_name {
        return &account.name == name;
    }
    if let Some(pool) = &config.account_pool {
        return account.tags.iter().any(|tag| tag == pool);
    }
    true
}

/// An [`AgentProvider`] that runs each invocation under a Provider Account.
///
/// See the [module documentation](self).
pub struct AccountAwareProvider {
    inner: Arc<dyn AgentProvider>,
    store: Arc<dyn Store>,
    strategy: Arc<dyn AccountStrategy>,
    kinds: HashMap<&'static str, Arc<dyn AccountKind>>,
    max_capacity_wait: Duration,
}

impl fmt::Debug for AccountAwareProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccountAwareProvider")
            .field("strategy", &self.strategy.name())
            .field("kinds", &self.kinds.keys().collect::<Vec<_>>())
            .field("max_capacity_wait", &self.max_capacity_wait)
            .finish_non_exhaustive()
    }
}

impl AccountAwareProvider {
    /// Wrap `inner`, reading accounts from `store`, with the
    /// [`LeastUtilized`] strategy, the [`ClaudeSubscriptionKind`] kind and a
    /// capacity wait of [`DEFAULT_MAX_CAPACITY_WAIT`].
    ///
    /// # Examples
    ///
    /// See the [module documentation](self).
    pub fn new(inner: Arc<dyn AgentProvider>, store: Arc<dyn Store>) -> Self {
        let claude: Arc<dyn AccountKind> = Arc::new(ClaudeSubscriptionKind::new());
        Self {
            inner,
            store,
            strategy: Arc::new(LeastUtilized),
            kinds: HashMap::from([(claude.id(), claude)]),
            max_capacity_wait: DEFAULT_MAX_CAPACITY_WAIT,
        }
    }

    /// Replace the account selection strategy.
    ///
    /// # Examples
    ///
    /// See the [module documentation](self).
    pub fn with_strategy(mut self, strategy: Arc<dyn AccountStrategy>) -> Self {
        self.strategy = strategy;
        self
    }

    /// Longest time a step waits for provider capacity when it sets no
    /// [`AgentConfig::max_capacity_wait`] of its own.
    ///
    /// `Duration::ZERO` fails such steps at once with
    /// [`AgentError::NoCapacity`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use std::time::Duration;
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    /// use ironflow_engine::accounts::AccountAwareProvider;
    /// use ironflow_store::memory::InMemoryStore;
    ///
    /// let provider = AccountAwareProvider::new(
    ///     Arc::new(ClaudeCodeProvider::new()),
    ///     Arc::new(InMemoryStore::new()),
    /// )
    /// .with_max_capacity_wait(Duration::from_secs(3600));
    /// # let _ = provider;
    /// ```
    pub fn with_max_capacity_wait(mut self, wait: Duration) -> Self {
        self.max_capacity_wait = wait;
        self
    }

    /// Register an additional account kind (or replace one with the same id).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use ironflow_core::account::ClaudeSubscriptionKind;
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    /// use ironflow_engine::accounts::AccountAwareProvider;
    /// use ironflow_store::memory::InMemoryStore;
    ///
    /// let provider = AccountAwareProvider::new(
    ///     Arc::new(ClaudeCodeProvider::new()),
    ///     Arc::new(InMemoryStore::new()),
    /// )
    /// .with_kind(Arc::new(ClaudeSubscriptionKind::with_api_base("http://proxy:8080")));
    /// # let _ = provider;
    /// ```
    pub fn with_kind(mut self, kind: Arc<dyn AccountKind>) -> Self {
        self.kinds.insert(kind.id(), kind);
        self
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
        let Some(kind_id) = self.inner.account_kind_for(config) else {
            return self.invoke_inner(config, sink).await;
        };
        let Some(kind) = self.kinds.get(kind_id).cloned() else {
            debug!(
                kind = kind_id,
                "no account kind registered, using worker environment"
            );
            return self.invoke_inner(config, sink).await;
        };
        let wait = config.max_capacity_wait.unwrap_or(self.max_capacity_wait);

        // Accounts already tried by this invocation: each one is tried at most
        // once, so the failover ends after the last targeted account.
        let mut attempts: Vec<AccountAttempt> = Vec::new();
        loop {
            let candidates = self
                .store
                .list_provider_account_candidates(kind_id.to_string())
                .await
                .map_err(|e| {
                    resolution_error(format!("provider account resolution failed: {e}"))
                })?;
            if candidates.is_empty()
                && config.account_name.is_none()
                && config.account_pool.is_none()
            {
                debug!(
                    kind = kind_id,
                    "no provider account for kind, using worker environment"
                );
                return self.run_with_worker_env(kind_id, config, sink, wait).await;
            }

            let targeted: Vec<&ProviderAccountCandidate> = candidates
                .iter()
                .filter(|c| is_targeted(config, &c.account))
                .collect();
            if targeted.is_empty() {
                if let Some(name) = &config.account_name {
                    return Err(AgentError::AccountNotFound { name: name.clone() });
                }
                return Err(AgentError::NoCapacity {
                    kind: kind_id.to_string(),
                    next_reset: None,
                });
            }

            let all: Vec<_> = targeted.iter().map(|c| to_core_candidate(c)).collect();
            let untried: Vec<AccountCandidate> = all
                .iter()
                .filter(|c| !attempts.iter().any(|a| a.account_id.to_string() == c.id))
                .cloned()
                .collect();
            let now = Utc::now();
            let Some(selected) =
                select_account(self.strategy.as_ref(), &untried, &config.model, now)
            else {
                let next_reset = earliest_reset(
                    all.iter().flat_map(|c| c.windows.iter()),
                    &config.model,
                    now,
                );
                let concurrency_blocked = all.iter().any(|c| {
                    c.max_concurrency.is_some_and(|max| c.running_steps >= max)
                        && !is_rejected(&c.windows, &config.model, now)
                });
                let decision = capacity_decision(
                    kind_id,
                    next_reset,
                    concurrency_blocked,
                    wait,
                    config.capacity_wait_since,
                    now,
                );
                info!(
                    kind = kind_id,
                    attempts = ?attempts,
                    decision = %decision,
                    "no targeted provider account has capacity"
                );
                return Err(decision);
            };
            let account: &ProviderAccount = &targeted
                .iter()
                .find(|c| c.account.id.to_string() == selected.id)
                .ok_or_else(|| resolution_error("selected provider account vanished".to_string()))?
                .account;

            let result = self
                .run_with_account(kind.as_ref(), account, config, sink.clone())
                .await?;
            match result {
                AccountRun::Done(result) => return result,
                AccountRun::Rejected => {
                    let attempt = AccountAttempt {
                        account_id: account.id,
                        outcome: "rate limited",
                    };
                    warn!(
                        account = %account.name,
                        account_id = %attempt.account_id,
                        outcome = attempt.outcome,
                        "provider account rate limited mid-step, trying the next account"
                    );
                    attempts.push(attempt);
                }
            }
        }
    }

    /// Run the invocation under `account` and record what it observed.
    ///
    /// The outer `Err` is a resolution failure (missing credential); a
    /// rejection by a rate-limited window comes back as
    /// [`AccountRun::Rejected`].
    async fn run_with_account(
        &self,
        kind: &dyn AccountKind,
        account: &ProviderAccount,
        config: &AgentConfig,
        sink: Option<Arc<dyn LogSink>>,
    ) -> Result<AccountRun, AgentError> {
        let missing = || {
            resolution_error(format!(
                "credential of provider account '{}' is missing",
                account.name
            ))
        };
        let secret = match self.store.get_secret(&account.secret_key).await {
            Ok(Some(secret)) => secret,
            Ok(None) => return Err(missing()),
            Err(e) => {
                warn!(account = %account.name, error = %e, "failed to read provider account credential");
                return Err(missing());
            }
        };

        info!(
            account = %account.name,
            strategy = self.strategy.name(),
            model = %config.model,
            "selected provider account"
        );

        let recorder = RateLimitRecorder::default();
        // Rate-limit events only appear in stream-json, hence verbose.
        let account_config = config
            .clone()
            .verbose(true)
            .account_session(AccountSession::new(
                kind.credential(&secret.value),
                recorder.clone(),
            ));

        let result = self.invoke_inner(&account_config, sink).await;

        let windows = recorder.take();
        let now = Utc::now();
        let rejected = result.is_err() && is_rejected(&windows, &config.model, now);
        let auth_failed = matches!(
            result,
            Err(AgentError::Api {
                status: Some(401 | 403),
                ..
            })
        );
        if !windows.is_empty() || auth_failed {
            let observation = NewProviderAccountObservation {
                windows: windows.into_iter().map(window_to_store).collect(),
                auth_failed,
            };
            if let Err(e) = self
                .store
                .record_provider_account_observation(account.id, observation)
                .await
            {
                warn!(account = %account.name, error = %e, "failed to record provider account usage");
            }
        }

        if rejected {
            return Ok(AccountRun::Rejected);
        }
        Ok(AccountRun::Done(result.map(|mut output| {
            output.account_id = Some(account.id.to_string());
            output
        })))
    }

    /// Run the invocation with the worker environment credential.
    ///
    /// A failure along with a rejected window for the model means the worker
    /// token is rate limited: the step waits for its reset like a pool whose
    /// accounts are all limited.
    async fn run_with_worker_env(
        &self,
        kind_id: &str,
        config: &AgentConfig,
        sink: Option<Arc<dyn LogSink>>,
        wait: Duration,
    ) -> Result<AgentOutput, AgentError> {
        let recorder = RateLimitRecorder::default();
        // Rate-limit events only appear in stream-json, hence verbose.
        let env_config = config
            .clone()
            .verbose(true)
            .rate_limit_recorder(recorder.clone());
        let result = self.invoke_inner(&env_config, sink).await;
        if result.is_ok() {
            return result;
        }

        let windows = recorder.take();
        let now = Utc::now();
        if !is_rejected(&windows, &config.model, now) {
            return result;
        }
        let next_reset = earliest_reset(windows.iter(), &config.model, now);
        let decision = capacity_decision(
            kind_id,
            next_reset,
            false,
            wait,
            config.capacity_wait_since,
            now,
        );
        info!(
            kind = kind_id,
            decision = %decision,
            "worker environment credential rate limited"
        );
        Err(decision)
    }
}

/// What a run under one account ended with.
#[allow(clippy::large_enum_variant)]
enum AccountRun {
    /// The invocation finished, successfully or not, without a rate-limit
    /// rejection.
    Done(Result<AgentOutput, AgentError>),
    /// A window constraining the model rejected the invocation.
    Rejected,
}

/// Whether one of `windows` rejects requests for `model` at `now`.
fn is_rejected(windows: &[AccountWindow], model: &str, now: DateTime<Utc>) -> bool {
    windows
        .iter()
        .any(|w| w.applies_to(model) && w.is_exhausted(now))
}

impl AgentProvider for AccountAwareProvider {
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
    use std::sync::Mutex;

    use ironflow_core::account_strategy::Priority;
    use ironflow_core::providers::router::{ProviderMatcher, ProviderRouter};
    use ironflow_store::crypto::KeyRing;
    use ironflow_store::entities::{
        NewProviderAccount, NewRun, ProviderKind, RunStatus, RunUpdate, TriggerKind,
        provider_account_secret_key,
    };
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::provider_account_store::ProviderAccountStore;
    use ironflow_store::secret_store::SecretStore;
    use ironflow_store::store::RunStore;
    use serde_json::json;

    use super::*;

    const TOKEN: &str = "sk-ant-oat01-test-token-abcdefghijklmnopqrstuvwxyz";

    /// What the test provider does once invoked.
    #[derive(Clone, Copy)]
    enum Outcome {
        Succeed,
        FailApi(u16),
        /// Report a rejected window resetting at `resets_at`, then fail with
        /// a 429, like a rate-limited Claude transport.
        Reject {
            resets_at: DateTime<Utc>,
        },
        /// Reject the first invocation, succeed afterwards.
        RejectOnce {
            resets_at: DateTime<Utc>,
        },
    }

    /// A provider that behaves like the Claude transports: it reads the
    /// injected credential and reports a rate-limit window to the recorder.
    struct RecordingProvider {
        kind: Option<&'static str>,
        outcome: Outcome,
        seen: Mutex<Vec<(Option<String>, bool)>>,
    }

    impl RecordingProvider {
        fn new(kind: Option<&'static str>, outcome: Outcome) -> Self {
            Self {
                kind,
                outcome,
                seen: Mutex::new(Vec::new()),
            }
        }

        fn seen(&self) -> Vec<(Option<String>, bool)> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl AgentProvider for RecordingProvider {
        fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
            Box::pin(async move {
                let credential = config
                    .account
                    .as_ref()
                    .map(|s| s.credential().expose().to_string());
                let previous = {
                    let mut seen = self.seen.lock().unwrap();
                    seen.push((credential, config.verbose));
                    seen.len() - 1
                };
                let rejection = match self.outcome {
                    Outcome::Reject { resets_at } => Some(resets_at),
                    Outcome::RejectOnce { resets_at } if previous == 0 => Some(resets_at),
                    _ => None,
                };
                if let Some(resets_at) = rejection {
                    let recorder = config
                        .account
                        .as_ref()
                        .map(AccountSession::recorder)
                        .or(config.rate_limits.as_ref())
                        .expect("AccountAwareProvider always sets a recorder");
                    recorder.record(AccountWindow {
                        window: "five_hour".to_string(),
                        utilization: 1.0,
                        resets_at: Some(resets_at),
                        status: WindowStatus::Rejected,
                        model_scope: None,
                        observed_at: Utc::now(),
                    });
                    return Err(AgentError::Api {
                        status: Some(429),
                        code: None,
                        message: "rate limited".to_string(),
                    });
                }
                if let Some(session) = &config.account {
                    session.recorder().record(AccountWindow {
                        window: "five_hour".to_string(),
                        utilization: 0.42,
                        resets_at: Some(Utc::now() + TimeDelta::hours(2)),
                        status: WindowStatus::Allowed,
                        model_scope: None,
                        observed_at: Utc::now(),
                    });
                }
                match self.outcome {
                    Outcome::Succeed | Outcome::Reject { .. } | Outcome::RejectOnce { .. } => {
                        Ok(AgentOutput::new(json!("done")))
                    }
                    Outcome::FailApi(status) => Err(AgentError::Api {
                        status: Some(status),
                        code: None,
                        message: "API Error".to_string(),
                    }),
                }
            })
        }

        fn account_kind(&self) -> Option<&'static str> {
            self.kind
        }
    }

    fn store_with_key() -> InMemoryStore {
        let mut store = InMemoryStore::new();
        let spec = format!("1:{}", "aa".repeat(32));
        store.set_key_ring(KeyRing::from_spec(&spec, Some(1)).unwrap());
        store
    }

    async fn add_account(store: &InMemoryStore, name: &str, priority: i32) -> ProviderAccount {
        add_account_with(store, name, priority, &[], None).await
    }

    async fn add_account_with(
        store: &InMemoryStore,
        name: &str,
        priority: i32,
        tags: &[&str],
        max_concurrency: Option<u32>,
    ) -> ProviderAccount {
        let id = Uuid::now_v7();
        let secret_key = provider_account_secret_key(id);
        store
            .set_secret(&secret_key, &format!("{TOKEN}-{name}"))
            .await
            .unwrap();
        store
            .create_provider_account(NewProviderAccount {
                id,
                name: name.to_string(),
                display_name: name.to_string(),
                kind: ClaudeSubscriptionKind::ID.to_string(),
                secret_key,
                enabled: true,
                priority,
                tags: tags.iter().map(|t| t.to_string()).collect(),
                max_concurrency,
                alert_threshold: 0.8,
                expires_at: Utc::now() + TimeDelta::days(30),
                plan: None,
                created_by: None,
            })
            .await
            .unwrap()
    }

    fn wrap(inner: Arc<RecordingProvider>, store: &Arc<InMemoryStore>) -> AccountAwareProvider {
        let store: Arc<dyn Store> = store.clone();
        AccountAwareProvider::new(inner, store)
    }

    #[tokio::test]
    async fn account_aware_provider_injects_selected_account() {
        let store = Arc::new(store_with_key());
        add_account(&store, "busy", 10).await;
        let busy = store
            .find_provider_account_by_name("busy")
            .await
            .unwrap()
            .unwrap();
        store
            .record_provider_account_observation(
                busy.id,
                NewProviderAccountObservation {
                    windows: vec![NewAccountWindow {
                        window: "five_hour".to_string(),
                        utilization: 0.9,
                        resets_at: Some(Utc::now() + TimeDelta::hours(1)),
                        status: AccountWindowStatus::Allowed,
                        model_scope: None,
                        observed_at: Utc::now(),
                    }],
                    auth_failed: false,
                },
            )
            .await
            .unwrap();
        add_account(&store, "idle", 20).await;

        let inner = Arc::new(RecordingProvider::new(
            Some(ClaudeSubscriptionKind::ID),
            Outcome::Succeed,
        ));
        let provider = wrap(inner.clone(), &store);
        provider.invoke(&AgentConfig::new("hello")).await.unwrap();

        let seen = inner.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0.as_deref(), Some(format!("{TOKEN}-idle").as_str()));
        assert!(seen[0].1, "verbose must be forced for rate_limit_event");
    }

    #[tokio::test]
    async fn account_is_injected_behind_a_router() {
        let store = Arc::new(store_with_key());
        let account = add_account(&store, "perso", 10).await;
        let claude = Arc::new(RecordingProvider::new(
            Some(ClaudeSubscriptionKind::ID),
            Outcome::Succeed,
        ));
        let router = ProviderRouter::new(claude.clone());
        let dyn_store: Arc<dyn Store> = store.clone();
        let provider = AccountAwareProvider::new(Arc::new(router), dyn_store);

        let output = provider
            .invoke(&AgentConfig::new("p").model("sonnet"))
            .await
            .unwrap();

        assert_eq!(output.account_id, Some(account.id.to_string()));
        let seen = claude.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].0.as_deref(),
            Some(format!("{TOKEN}-perso").as_str())
        );
    }

    #[tokio::test]
    async fn mixed_router_injects_an_account_only_on_claude_routes() {
        let store = Arc::new(store_with_key());
        let account = add_account(&store, "perso", 10).await;
        let claude = Arc::new(RecordingProvider::new(
            Some(ClaudeSubscriptionKind::ID),
            Outcome::Succeed,
        ));
        let http = Arc::new(RecordingProvider::new(None, Outcome::Succeed));
        let router = ProviderRouter::new(claude.clone())
            .route(ProviderMatcher::ModelPrefix("gpt-".into()), http.clone());
        let dyn_store: Arc<dyn Store> = store.clone();
        let provider = AccountAwareProvider::new(Arc::new(router), dyn_store);

        let claude_output = provider
            .invoke(&AgentConfig::new("p").model("sonnet"))
            .await
            .unwrap();
        assert_eq!(claude_output.account_id, Some(account.id.to_string()));
        assert!(claude.seen()[0].0.is_some());

        let http_output = provider
            .invoke(&AgentConfig::new("p").model("gpt-5"))
            .await
            .unwrap();
        assert_eq!(http_output.account_id, None);
        assert_eq!(http.seen(), vec![(None, false)]);
    }

    #[tokio::test]
    async fn account_aware_provider_passthrough_without_accounts() {
        let store = Arc::new(store_with_key());
        let inner = Arc::new(RecordingProvider::new(
            Some(ClaudeSubscriptionKind::ID),
            Outcome::Succeed,
        ));
        let provider = wrap(inner.clone(), &store);
        let output = provider.invoke(&AgentConfig::new("hello")).await.unwrap();
        assert_eq!(output.account_id, None);
        // Verbose is forced so a rate-limit rejection of the worker token is seen.
        assert_eq!(inner.seen(), vec![(None, true)]);
    }

    #[tokio::test]
    async fn account_aware_provider_passthrough_for_kindless_provider() {
        let store = Arc::new(store_with_key());
        add_account(&store, "perso", 10).await;
        let inner = Arc::new(RecordingProvider::new(None, Outcome::Succeed));
        let provider = wrap(inner.clone(), &store);
        let output = provider.invoke(&AgentConfig::new("hello")).await.unwrap();
        assert_eq!(output.account_id, None);
        assert_eq!(inner.seen(), vec![(None, false)]);
        assert_eq!(provider.account_kind(), None);
    }

    #[tokio::test]
    async fn account_aware_provider_records_windows_on_error() {
        let store = Arc::new(store_with_key());
        let account = add_account(&store, "perso", 10).await;
        let inner = Arc::new(RecordingProvider::new(
            Some(ClaudeSubscriptionKind::ID),
            Outcome::FailApi(500),
        ));
        let provider = wrap(inner, &store);
        let err = provider
            .invoke(&AgentConfig::new("hello"))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            AgentError::Api {
                status: Some(500),
                ..
            }
        ));

        let windows = store
            .list_provider_account_windows(vec![account.id])
            .await
            .unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].window, "five_hour");
        let stored = store
            .get_provider_account(account.id)
            .await
            .unwrap()
            .unwrap();
        assert!(stored.auth_failed_at.is_none());
    }

    #[tokio::test]
    async fn account_aware_provider_marks_auth_failed_on_401() {
        let store = Arc::new(store_with_key());
        let account = add_account(&store, "perso", 10).await;
        let inner = Arc::new(RecordingProvider::new(
            Some(ClaudeSubscriptionKind::ID),
            Outcome::FailApi(401),
        ));
        let provider = wrap(inner, &store);
        provider
            .invoke(&AgentConfig::new("hello"))
            .await
            .unwrap_err();

        let stored = store
            .get_provider_account(account.id)
            .await
            .unwrap()
            .unwrap();
        assert!(stored.auth_failed_at.is_some());
        let candidates = store
            .list_provider_account_candidates(ClaudeSubscriptionKind::ID.to_string())
            .await
            .unwrap();
        assert!(candidates.is_empty(), "a rejected token is not a candidate");
    }

    async fn exhaust(store: &InMemoryStore, account: &ProviderAccount, resets_at: DateTime<Utc>) {
        store
            .record_provider_account_observation(
                account.id,
                NewProviderAccountObservation {
                    windows: vec![NewAccountWindow {
                        window: "five_hour".to_string(),
                        utilization: 1.0,
                        resets_at: Some(resets_at),
                        status: AccountWindowStatus::Rejected,
                        model_scope: None,
                        observed_at: Utc::now(),
                    }],
                    auth_failed: false,
                },
            )
            .await
            .unwrap();
    }

    fn succeeding() -> Arc<RecordingProvider> {
        Arc::new(RecordingProvider::new(
            Some(ClaudeSubscriptionKind::ID),
            Outcome::Succeed,
        ))
    }

    #[tokio::test]
    async fn pool_exhausted_sleeps_until_next_reset() {
        let store = Arc::new(store_with_key());
        let soon = Utc::now() + TimeDelta::hours(1);
        let later = Utc::now() + TimeDelta::hours(2);
        let first = add_account(&store, "first", 10).await;
        exhaust(&store, &first, later).await;
        let second = add_account(&store, "second", 20).await;
        exhaust(&store, &second, soon).await;
        let inner = succeeding();
        let provider = wrap(inner.clone(), &store);

        let err = provider
            .invoke(&AgentConfig::new("hello"))
            .await
            .unwrap_err();

        let AgentError::CapacityWait { kind, wake_at } = err else {
            panic!("expected CapacityWait, got {err}");
        };
        assert_eq!(kind, ClaudeSubscriptionKind::ID);
        assert_eq!(wake_at, soon, "wakes at the earliest reset of the pool");
        assert!(inner.seen().is_empty(), "the agent must not run");
    }

    #[tokio::test]
    async fn zero_capacity_wait_fails_fast_with_no_capacity() {
        let store = Arc::new(store_with_key());
        let reset = Utc::now() + TimeDelta::hours(1);
        let account = add_account(&store, "perso", 10).await;
        exhaust(&store, &account, reset).await;
        let inner = succeeding();
        let provider = wrap(inner.clone(), &store);

        let err = provider
            .invoke(&AgentConfig::new("hello").max_capacity_wait(Duration::ZERO))
            .await
            .unwrap_err();

        let AgentError::NoCapacity { kind, next_reset } = err else {
            panic!("expected NoCapacity, got {err}");
        };
        assert_eq!(kind, ClaudeSubscriptionKind::ID);
        assert_eq!(next_reset, Some(reset));
        assert!(inner.seen().is_empty(), "the agent must not run");
    }

    #[tokio::test]
    async fn worker_max_capacity_wait_applies_when_the_step_sets_none() {
        let store = Arc::new(store_with_key());
        let reset = Utc::now() + TimeDelta::hours(1);
        let account = add_account(&store, "perso", 10).await;
        exhaust(&store, &account, reset).await;
        let provider = wrap(succeeding(), &store).with_max_capacity_wait(Duration::ZERO);

        let err = provider
            .invoke(&AgentConfig::new("hello"))
            .await
            .unwrap_err();
        assert!(matches!(err, AgentError::NoCapacity { .. }), "got {err}");

        let err = provider
            .invoke(&AgentConfig::new("hello").max_capacity_wait(Duration::from_secs(7200)))
            .await
            .unwrap_err();
        assert!(
            matches!(err, AgentError::CapacityWait { .. }),
            "the step wait overrides the worker default, got {err}"
        );
    }

    #[tokio::test]
    async fn reset_beyond_capacity_wait_fails_with_no_capacity() {
        let store = Arc::new(store_with_key());
        let reset = Utc::now() + TimeDelta::hours(8);
        let account = add_account(&store, "perso", 10).await;
        exhaust(&store, &account, reset).await;
        let inner = succeeding();
        let provider = wrap(inner.clone(), &store);

        let err = provider
            .invoke(&AgentConfig::new("hello"))
            .await
            .unwrap_err();

        let AgentError::NoCapacity { next_reset, .. } = err else {
            panic!("expected NoCapacity, got {err}");
        };
        assert_eq!(next_reset, Some(reset));
        assert!(inner.seen().is_empty(), "the agent must not run");
    }

    #[tokio::test]
    async fn rejection_mid_step_fails_over_to_next_account() {
        let store = Arc::new(store_with_key());
        let reset = Utc::now() + TimeDelta::hours(1);
        let first = add_account(&store, "first", 10).await;
        let second = add_account(&store, "second", 20).await;
        let inner = Arc::new(RecordingProvider::new(
            Some(ClaudeSubscriptionKind::ID),
            Outcome::RejectOnce { resets_at: reset },
        ));
        let provider = wrap(inner.clone(), &store).with_strategy(Arc::new(Priority));

        let output = provider.invoke(&AgentConfig::new("hello")).await.unwrap();

        assert_eq!(output.account_id, Some(second.id.to_string()));
        let credentials: Vec<Option<String>> = inner.seen().into_iter().map(|s| s.0).collect();
        assert_eq!(
            credentials,
            vec![
                Some(format!("{TOKEN}-first")),
                Some(format!("{TOKEN}-second")),
            ]
        );
        let windows = store
            .list_provider_account_windows(vec![first.id])
            .await
            .unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].status, AccountWindowStatus::Rejected);
        assert_eq!(windows[0].resets_at, Some(reset));
    }

    #[tokio::test]
    async fn rejection_on_every_account_sleeps_until_next_reset() {
        let store = Arc::new(store_with_key());
        let reset = Utc::now() + TimeDelta::hours(1);
        add_account(&store, "first", 10).await;
        add_account(&store, "second", 20).await;
        let inner = Arc::new(RecordingProvider::new(
            Some(ClaudeSubscriptionKind::ID),
            Outcome::Reject { resets_at: reset },
        ));
        let provider = wrap(inner.clone(), &store);

        let err = provider
            .invoke(&AgentConfig::new("hello"))
            .await
            .unwrap_err();

        let AgentError::CapacityWait { wake_at, .. } = err else {
            panic!("expected CapacityWait, got {err}");
        };
        assert_eq!(wake_at, reset);
        assert_eq!(inner.seen().len(), 2, "each account is tried once");
    }

    #[tokio::test]
    async fn named_account_waits_without_failover() {
        let store = Arc::new(store_with_key());
        let reset = Utc::now() + TimeDelta::hours(1);
        add_account(&store, "first", 10).await;
        add_account(&store, "second", 20).await;
        let inner = Arc::new(RecordingProvider::new(
            Some(ClaudeSubscriptionKind::ID),
            Outcome::Reject { resets_at: reset },
        ));
        let provider = wrap(inner.clone(), &store);

        let err = provider
            .invoke(&AgentConfig::new("hello").account("second"))
            .await
            .unwrap_err();

        let AgentError::CapacityWait { wake_at, .. } = err else {
            panic!("expected CapacityWait, got {err}");
        };
        assert_eq!(wake_at, reset);
        let credentials: Vec<Option<String>> = inner.seen().into_iter().map(|s| s.0).collect();
        assert_eq!(credentials, vec![Some(format!("{TOKEN}-second"))]);
    }

    #[tokio::test]
    async fn named_account_runs_under_that_account_only() {
        let store = Arc::new(store_with_key());
        add_account(&store, "first", 10).await;
        let second = add_account(&store, "second", 20).await;
        let inner = succeeding();
        let provider = wrap(inner.clone(), &store);

        let output = provider
            .invoke(&AgentConfig::new("hello").account("second"))
            .await
            .unwrap();

        assert_eq!(output.account_id, Some(second.id.to_string()));
    }

    #[tokio::test]
    async fn unknown_account_name_fails_with_account_not_found() {
        let store = Arc::new(store_with_key());
        add_account(&store, "perso", 10).await;
        let inner = succeeding();
        let provider = wrap(inner.clone(), &store);

        let err = provider
            .invoke(&AgentConfig::new("hello").account("missing"))
            .await
            .unwrap_err();

        let AgentError::AccountNotFound { name } = err else {
            panic!("expected AccountNotFound, got {err}");
        };
        assert_eq!(name, "missing");
        assert!(inner.seen().is_empty(), "the agent must not run");
    }

    #[tokio::test]
    async fn account_name_without_any_account_never_falls_back_to_worker_env() {
        let store = Arc::new(store_with_key());
        let inner = succeeding();
        let provider = wrap(inner.clone(), &store);

        let err = provider
            .invoke(&AgentConfig::new("hello").account("perso"))
            .await
            .unwrap_err();

        assert!(
            matches!(err, AgentError::AccountNotFound { .. }),
            "got {err}"
        );
        assert!(inner.seen().is_empty(), "the worker token must not be used");
    }

    #[tokio::test]
    async fn account_pool_only_selects_tagged_accounts() {
        let store = Arc::new(store_with_key());
        add_account_with(&store, "untagged", 1, &[], None).await;
        let tagged = add_account_with(&store, "tagged", 50, &["batch"], None).await;
        let inner = succeeding();
        let provider = wrap(inner.clone(), &store).with_strategy(Arc::new(Priority));

        let output = provider
            .invoke(&AgentConfig::new("hello").account_pool("batch"))
            .await
            .unwrap();

        assert_eq!(output.account_id, Some(tagged.id.to_string()));
    }

    #[tokio::test]
    async fn unknown_account_pool_fails_with_no_capacity() {
        let store = Arc::new(store_with_key());
        add_account_with(&store, "tagged", 10, &["batch"], None).await;
        let inner = succeeding();
        let provider = wrap(inner.clone(), &store);

        let err = provider
            .invoke(&AgentConfig::new("hello").account_pool("nightly"))
            .await
            .unwrap_err();

        let AgentError::NoCapacity { kind, next_reset } = err else {
            panic!("expected NoCapacity, got {err}");
        };
        assert_eq!(kind, ClaudeSubscriptionKind::ID);
        assert_eq!(next_reset, None);
        assert!(inner.seen().is_empty(), "the agent must not run");
    }

    #[tokio::test]
    async fn saturated_account_retries_after_a_minute() {
        let store = Arc::new(store_with_key());
        // `max_concurrency: 0` is saturated with no running step.
        add_account_with(&store, "busy", 10, &[], Some(0)).await;
        let inner = succeeding();
        let provider = wrap(inner.clone(), &store);
        let before = Utc::now();

        let err = provider
            .invoke(&AgentConfig::new("hello"))
            .await
            .unwrap_err();

        let AgentError::CapacityWait { wake_at, .. } = err else {
            panic!("expected CapacityWait, got {err}");
        };
        assert!(wake_at >= before + TimeDelta::seconds(60));
        assert!(wake_at <= Utc::now() + TimeDelta::seconds(60));
        assert!(inner.seen().is_empty(), "the agent must not run");
    }

    #[tokio::test]
    async fn saturated_account_stops_waiting_past_the_cumulative_bound() {
        let store = Arc::new(store_with_key());
        add_account_with(&store, "busy", 10, &[], Some(0)).await;
        let provider = wrap(succeeding(), &store);
        let config = AgentConfig::new("hello")
            .max_capacity_wait(Duration::from_secs(3600))
            .capacity_wait_since(Utc::now() - TimeDelta::minutes(59) - TimeDelta::seconds(30));

        let err = provider.invoke(&config).await.unwrap_err();

        let AgentError::NoCapacity { next_reset, .. } = err else {
            panic!("expected NoCapacity, got {err}");
        };
        assert_eq!(next_reset, None);
    }

    #[tokio::test]
    async fn worker_env_rejection_sleeps_until_resets_at() {
        let store = Arc::new(store_with_key());
        let reset = Utc::now() + TimeDelta::hours(1);
        let inner = Arc::new(RecordingProvider::new(
            Some(ClaudeSubscriptionKind::ID),
            Outcome::Reject { resets_at: reset },
        ));
        let provider = wrap(inner.clone(), &store);

        let err = provider
            .invoke(&AgentConfig::new("hello"))
            .await
            .unwrap_err();

        let AgentError::CapacityWait { kind, wake_at } = err else {
            panic!("expected CapacityWait, got {err}");
        };
        assert_eq!(kind, ClaudeSubscriptionKind::ID);
        assert_eq!(wake_at, reset);
        assert_eq!(inner.seen(), vec![(None, true)]);
    }

    #[tokio::test]
    async fn worker_env_failure_without_rejection_is_returned_unchanged() {
        let store = Arc::new(store_with_key());
        let inner = Arc::new(RecordingProvider::new(
            Some(ClaudeSubscriptionKind::ID),
            Outcome::FailApi(500),
        ));
        let provider = wrap(inner, &store);

        let err = provider
            .invoke(&AgentConfig::new("hello"))
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            AgentError::Api {
                status: Some(500),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn adding_account_wakes_capacity_sleepers() {
        let store = store_with_key();
        let far = Utc::now() + TimeDelta::hours(3);
        let mut runs = Vec::new();
        for kind in [ClaudeSubscriptionKind::ID, "other_kind"] {
            let run = store
                .create_run(NewRun {
                    created_by: None,
                    workflow_name: "capacity".to_string(),
                    trigger: TriggerKind::Manual,
                    payload: json!({}),
                    max_retries: 0,
                    handler_version: None,
                    labels: HashMap::new(),
                    scheduled_at: None,
                    idempotency_key: None,
                    concurrency_key: None,
                    priority: 0,
                    concurrency_limits: Vec::new(),
                    max_cost_usd: None,
                    worker_tags: Vec::new(),
                })
                .await
                .unwrap()
                .into_run();
            store
                .update_run_status(run.id, RunStatus::Running)
                .await
                .unwrap();
            store
                .update_run(
                    run.id,
                    RunUpdate {
                        status: Some(RunStatus::Sleeping),
                        scheduled_at: Some(far),
                        capacity_wait_kind: Some(ProviderKind::from(kind)),
                        ..RunUpdate::default()
                    },
                )
                .await
                .unwrap();
            runs.push(run.id);
        }

        add_account(&store, "fresh", 10).await;
        let now = Utc::now();

        let woken = store.get_run(runs[0]).await.unwrap().unwrap();
        assert_eq!(woken.status.state, RunStatus::Sleeping);
        assert!(woken.scheduled_at.is_some_and(|at| at <= now));
        let untouched = store.get_run(runs[1]).await.unwrap().unwrap();
        assert_eq!(untouched.scheduled_at, Some(far));
        assert_eq!(
            untouched.capacity_wait_kind,
            Some(ProviderKind::from("other_kind"))
        );
    }

    #[test]
    fn capacity_decision_zero_wait_fails_fast() {
        let now = Utc::now();
        let reset = now + TimeDelta::minutes(5);
        let err = capacity_decision("claude", Some(reset), true, Duration::ZERO, None, now);
        assert!(matches!(
            err,
            AgentError::NoCapacity { next_reset: Some(at), .. } if at == reset
        ));
    }

    #[test]
    fn capacity_decision_waits_for_a_reset_within_the_bound() {
        let now = Utc::now();
        let reset = now + TimeDelta::hours(1);
        let err = capacity_decision(
            "claude",
            Some(reset),
            false,
            DEFAULT_MAX_CAPACITY_WAIT,
            None,
            now,
        );
        assert!(matches!(
            err,
            AgentError::CapacityWait { wake_at, .. } if wake_at == reset
        ));
    }

    #[test]
    fn capacity_decision_counts_the_time_already_waited() {
        let now = Utc::now();
        let reset = now + TimeDelta::hours(1);
        let since = now - TimeDelta::hours(5) - TimeDelta::minutes(30);
        let err = capacity_decision(
            "claude",
            Some(reset),
            false,
            DEFAULT_MAX_CAPACITY_WAIT,
            Some(since),
            now,
        );
        assert!(matches!(
            err,
            AgentError::NoCapacity { next_reset: Some(at), .. } if at == reset
        ));
    }

    #[test]
    fn capacity_decision_fails_for_a_reset_beyond_the_bound() {
        let now = Utc::now();
        let reset = now + TimeDelta::hours(7);
        let err = capacity_decision(
            "claude",
            Some(reset),
            false,
            DEFAULT_MAX_CAPACITY_WAIT,
            None,
            now,
        );
        assert!(matches!(
            err,
            AgentError::NoCapacity { next_reset: Some(at), .. } if at == reset
        ));
    }

    #[test]
    fn capacity_decision_retries_concurrency_after_a_minute() {
        let now = Utc::now();
        let err = capacity_decision("claude", None, true, DEFAULT_MAX_CAPACITY_WAIT, None, now);
        assert!(matches!(
            err,
            AgentError::CapacityWait { wake_at, .. } if wake_at == now + TimeDelta::seconds(60)
        ));
    }

    #[test]
    fn capacity_decision_bounds_the_concurrency_wait() {
        let now = Utc::now();
        let wait = Duration::from_secs(600);
        let within = capacity_decision(
            "claude",
            None,
            true,
            wait,
            Some(now - TimeDelta::minutes(9)),
            now,
        );
        assert!(matches!(within, AgentError::CapacityWait { .. }));

        let beyond = capacity_decision(
            "claude",
            None,
            true,
            wait,
            Some(now - TimeDelta::minutes(9) - TimeDelta::seconds(1)),
            now,
        );
        assert!(matches!(
            beyond,
            AgentError::NoCapacity {
                next_reset: None,
                ..
            }
        ));
    }

    #[test]
    fn capacity_decision_without_reset_or_concurrency_fails() {
        let now = Utc::now();
        let err = capacity_decision("claude", None, false, DEFAULT_MAX_CAPACITY_WAIT, None, now);
        assert!(matches!(
            err,
            AgentError::NoCapacity {
                next_reset: None,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn account_aware_provider_fails_when_credential_missing() {
        let store = Arc::new(store_with_key());
        let account = add_account(&store, "perso", 10).await;
        store.delete_secret(&account.secret_key).await.unwrap();
        let inner = Arc::new(RecordingProvider::new(
            Some(ClaudeSubscriptionKind::ID),
            Outcome::Succeed,
        ));
        let provider = wrap(inner, &store);
        let err = provider
            .invoke(&AgentConfig::new("hello"))
            .await
            .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("credential of provider account 'perso' is missing"));
        assert!(!message.contains(TOKEN));
    }

    #[tokio::test]
    async fn account_aware_provider_sets_output_account_id() {
        let store = Arc::new(store_with_key());
        let account = add_account(&store, "perso", 10).await;
        let inner = Arc::new(RecordingProvider::new(
            Some(ClaudeSubscriptionKind::ID),
            Outcome::Succeed,
        ));
        let provider = wrap(inner, &store);
        let output = provider.invoke(&AgentConfig::new("hello")).await.unwrap();
        assert_eq!(output.account_id, Some(account.id.to_string()));

        let windows = store
            .list_provider_account_windows(vec![account.id])
            .await
            .unwrap();
        assert_eq!(windows.len(), 1);
        assert!((windows[0].utilization - 0.42).abs() < 1e-9);
    }
}
