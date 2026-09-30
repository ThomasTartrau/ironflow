//! Provider Account selection and injection for agent steps.
//!
//! [`AccountAwareProvider`] wraps the worker's [`AgentProvider`]. For each
//! agent invocation on a provider that can inject a Provider Account
//! credential for the invocation ([`AgentProvider::account_kind_for`]), it:
//!
//! 1. lists the candidate accounts of that kind from the store,
//! 2. picks one with an [`AccountStrategy`],
//! 3. reads its credential and runs the invocation under it,
//! 4. records the usage windows observed during the invocation.
//!
//! With no account of the kind in the store, or a provider that cannot
//! inject one, the invocation runs unchanged with the worker environment.
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

use chrono::Utc;
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

/// An [`AgentProvider`] that runs each invocation under a Provider Account.
///
/// See the [module documentation](self).
pub struct AccountAwareProvider {
    inner: Arc<dyn AgentProvider>,
    store: Arc<dyn Store>,
    strategy: Arc<dyn AccountStrategy>,
    kinds: HashMap<&'static str, Arc<dyn AccountKind>>,
}

impl fmt::Debug for AccountAwareProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccountAwareProvider")
            .field("strategy", &self.strategy.name())
            .field("kinds", &self.kinds.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl AccountAwareProvider {
    /// Wrap `inner`, reading accounts from `store`, with the
    /// [`LeastUtilized`] strategy and the [`ClaudeSubscriptionKind`] kind.
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

        let candidates = self
            .store
            .list_provider_account_candidates(kind_id.to_string())
            .await
            .map_err(|e| resolution_error(format!("provider account resolution failed: {e}")))?;
        if candidates.is_empty() {
            debug!(
                kind = kind_id,
                "no provider account for kind, using worker environment"
            );
            return self.invoke_inner(config, sink).await;
        }

        let core_candidates: Vec<AccountCandidate> =
            candidates.iter().map(to_core_candidate).collect();
        let now = Utc::now();
        let Some(selected) =
            select_account(self.strategy.as_ref(), &core_candidates, &config.model, now)
        else {
            let next_reset = core_candidates
                .iter()
                .flat_map(|c| c.windows.iter())
                .filter(|w| w.applies_to(&config.model) && w.is_exhausted(now))
                .filter_map(|w| w.resets_at)
                .min()
                .map_or_else(|| "unknown".to_string(), |at| at.to_rfc3339());
            return Err(resolution_error(format!(
                "no provider account available for {kind_id}: all limited or at max_concurrency (next reset {next_reset})"
            )));
        };
        let account: &ProviderAccount = &candidates
            .iter()
            .find(|c| c.account.id.to_string() == selected.id)
            .ok_or_else(|| resolution_error("selected provider account vanished".to_string()))?
            .account;

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

        result.map(|mut output| {
            output.account_id = Some(account.id.to_string());
            output
        })
    }
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

    use chrono::TimeDelta;
    use ironflow_core::providers::router::{ProviderMatcher, ProviderRouter};
    use ironflow_store::crypto::KeyRing;
    use ironflow_store::entities::{NewProviderAccount, provider_account_secret_key};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::provider_account_store::ProviderAccountStore;
    use ironflow_store::secret_store::SecretStore;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    const TOKEN: &str = "sk-ant-oat01-test-token-abcdefghijklmnopqrstuvwxyz";

    /// What the test provider does once invoked.
    #[derive(Clone, Copy)]
    enum Outcome {
        Succeed,
        FailApi(u16),
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
                self.seen.lock().unwrap().push((credential, config.verbose));
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
                    Outcome::Succeed => Ok(AgentOutput::new(json!("done"))),
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
                tags: Vec::new(),
                max_concurrency: None,
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
        assert_eq!(inner.seen(), vec![(None, false)]);
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

    #[tokio::test]
    async fn account_aware_provider_fails_when_all_exhausted() {
        let store = Arc::new(store_with_key());
        let account = add_account(&store, "perso", 10).await;
        let reset = Utc::now() + TimeDelta::hours(1);
        store
            .record_provider_account_observation(
                account.id,
                NewProviderAccountObservation {
                    windows: vec![NewAccountWindow {
                        window: "five_hour".to_string(),
                        utilization: 1.0,
                        resets_at: Some(reset),
                        status: AccountWindowStatus::Rejected,
                        model_scope: None,
                        observed_at: Utc::now(),
                    }],
                    auth_failed: false,
                },
            )
            .await
            .unwrap();
        let inner = Arc::new(RecordingProvider::new(
            Some(ClaudeSubscriptionKind::ID),
            Outcome::Succeed,
        ));
        let provider = wrap(inner.clone(), &store);
        let err = provider
            .invoke(&AgentConfig::new("hello"))
            .await
            .unwrap_err();
        let AgentError::ProcessFailed { stderr, .. } = err else {
            panic!("expected ProcessFailed");
        };
        assert!(stderr.contains("no provider account available"));
        assert!(stderr.contains(&reset.to_rfc3339()));
        assert!(inner.seen().is_empty(), "the agent must not run");
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
