//! The auth proxy tokens of one step, revoked whatever ends the step.

use std::mem::take;
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::{debug, warn};

use crate::auth_proxy::{AuthProxyClient, short_token_id};

/// Longest a step that ends waits for the revocation of its tokens. Past it,
/// the revocation goes on in the background and the step ends.
const REVOKE_WAIT: Duration = Duration::from_secs(5);

/// The auth proxy tokens issued for one step (Claude and proxied secrets).
///
/// Each token is added as soon as the proxy issues it. The revocation always
/// runs in a task of its own, so dropping the step never cuts it short:
///
/// * a step that ends calls [`revoke`](Self::revoke), which waits at most
///   [`REVOKE_WAIT`] for it;
/// * a step dropped first (run cancelled, lease lost, run or workflow timed
///   out, a parallel branch aborted) starts it from [`Drop`].
///
/// Without a runtime to run on, the token expiry stays the backstop. So it
/// is for a step dropped while the proxy is issuing a token: its id is not
/// known yet, but the token never reaches a pod.
pub(super) struct StepTokens {
    client: Option<AuthProxyClient>,
    ids: Vec<String>,
}

impl StepTokens {
    /// Tokens issued through `client`, `None` when no proxy is set.
    pub(super) fn new(client: Option<AuthProxyClient>) -> Self {
        Self {
            client,
            ids: Vec::new(),
        }
    }

    /// The admin client of the proxy, `None` when no proxy is set.
    pub(super) fn client(&self) -> Option<&AuthProxyClient> {
        self.client.as_ref()
    }

    /// Track a token the proxy just issued.
    pub(super) fn push(&mut self, id: String) {
        self.ids.push(id);
    }

    /// Revoke every token, best effort: their expiry is the backstop. Waits
    /// at most [`REVOKE_WAIT`]; the revocation goes on past it.
    pub(super) async fn revoke(mut self) {
        let Some(task) = self.spawn_revocation() else {
            return;
        };
        match timeout(REVOKE_WAIT, task).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => warn!(error = %e, "auth proxy token revocation task failed"),
            Err(_) => warn!(
                wait = ?REVOKE_WAIT,
                "auth proxy token revocation still running; the step ends without it"
            ),
        }
    }

    /// Start the revocation of the tokens not revoked yet, `None` when there
    /// is none or no runtime to run it on.
    fn spawn_revocation(&mut self) -> Option<JoinHandle<()>> {
        if self.ids.is_empty() {
            return None;
        }
        let Ok(handle) = Handle::try_current() else {
            warn!(
                tokens = self.ids.len(),
                "auth proxy tokens not revoked: no runtime; they expire at expires-at"
            );
            return None;
        };
        let ids = take(&mut self.ids);
        let client = self.client.clone();
        Some(handle.spawn(async move { revoke_all(client.as_ref(), &ids).await }))
    }
}

impl Drop for StepTokens {
    fn drop(&mut self) {
        // Detached: the revocation outlives the dropped step.
        drop(self.spawn_revocation());
    }
}

async fn revoke_all(client: Option<&AuthProxyClient>, ids: &[String]) {
    let Some(client) = client else {
        return;
    };
    for id in ids {
        let short = short_token_id(id);
        match client.revoke(id).await {
            Ok(()) => debug!(token = %short, "auth proxy token revoked"),
            Err(e) => warn!(
                token = %short,
                error = %e,
                "auth proxy token revocation failed; it expires at expires-at"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_outside_a_runtime_does_not_panic() {
        let mut tokens = StepTokens::new(None);
        tokens.push("0123456789abcdef".to_string());
        drop(tokens);
    }

    #[tokio::test]
    async fn revoke_without_a_proxy_returns_at_once() {
        let mut tokens = StepTokens::new(None);
        tokens.push("0123456789abcdef".to_string());
        timeout(Duration::from_secs(1), tokens.revoke())
            .await
            .expect("revoke waited without a proxy");
    }
}
