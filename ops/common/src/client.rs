//! Generic HTTP API client with multi-auth support.

use std::fmt;

use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;
use reqwest::Client;
use reqwest::RequestBuilder;

/// Authentication mode for [`HttpApiClient`].
///
/// # Examples
///
/// ```
/// use ironflow_ops_common::Auth;
///
/// let none = Auth::None;
/// let bearer = Auth::Bearer("token".into());
/// let basic = Auth::Basic { user: "admin".into(), password: "secret".into() };
/// ```
#[derive(Clone)]
pub enum Auth {
    /// No authentication.
    None,
    /// Bearer token authentication.
    Bearer(String),
    /// HTTP Basic authentication.
    Basic {
        /// Username.
        user: String,
        /// Password.
        password: String,
    },
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Auth::None => write!(f, "None"),
            Auth::Bearer(_) => write!(f, "Bearer(<redacted>)"),
            Auth::Basic { user, .. } => {
                write!(f, "Basic {{ user: {user:?}, password: <redacted> }}")
            }
        }
    }
}

/// A generic HTTP API client with configurable authentication.
///
/// Wraps a base URL, an [`Auth`] mode, and a [`reqwest::Client`], providing
/// convenience methods to build authenticated HTTP requests.
///
/// Product-specific ops crates (Grafana, Loki, Tempo) wrap this in a thin
/// newtype that provides product-specific `from_context` methods and
/// re-exports the request-building methods.
///
/// # Construction
///
/// - [`new`](HttpApiClient::new) for explicit parameters.
/// - [`with_bearer_token`](HttpApiClient::with_bearer_token) and
///   [`with_basic_auth`](HttpApiClient::with_basic_auth) to set auth.
/// - [`from_context`](HttpApiClient::from_context) to read credentials from
///   the workflow's secret store with configurable key names.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_common::HttpApiClient;
/// use reqwest::Client;
///
/// let client = HttpApiClient::new("http://api.example.com", Client::new())
///     .with_bearer_token("my-token");
///
/// // Build an authenticated GET request
/// let req = client.get("/api/v1/resource");
/// ```
#[derive(Clone)]
pub struct HttpApiClient {
    base_url: String,
    auth: Auth,
    http: Client,
}

impl fmt::Debug for HttpApiClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpApiClient")
            .field("base_url", &self.base_url)
            .field("auth", &self.auth)
            .field("http", &"[reqwest::Client]")
            .finish()
    }
}

impl HttpApiClient {
    /// Build a client from explicit parameters, without a secret store.
    ///
    /// The trailing slash on `base_url` is stripped.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_ops_common::HttpApiClient;
    /// use reqwest::Client;
    ///
    /// let client = HttpApiClient::new("http://api.example.com/", Client::new());
    /// assert_eq!(client.base_url(), "http://api.example.com");
    /// ```
    pub fn new(base_url: &str, http: Client) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            auth: Auth::None,
            http,
        }
    }

    /// Build a client from an [`OperationContext`]'s secret store.
    ///
    /// Reads secrets by configurable key names:
    /// - `url_key` (required) -- the base URL.
    /// - `token_key` (optional) -- a Bearer token.
    /// - `basic_auth_key` (optional) -- basic auth credentials as `user:password`.
    ///
    /// If neither token nor basic auth is set, requests are sent without
    /// authentication.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the secret store fails, if the
    /// URL secret is missing or empty, or if the basic auth value does not
    /// contain a colon separator.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_common::HttpApiClient;
    /// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    /// let client = HttpApiClient::from_context(
    ///     &ctx,
    ///     "tempo_url",
    ///     "tempo_token",
    ///     "tempo_basic_auth",
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_context(
        ctx: &OperationContext,
        url_key: &str,
        token_key: &str,
        basic_auth_key: &str,
    ) -> Result<Self, OperationError> {
        let url_secret = ctx.secrets().get(url_key).await?;
        let base_url = url_secret
            .filter(|s| !s.value.is_empty())
            .map(|s| s.value)
            .ok_or_else(|| OperationError::Secret {
                message: format!("missing or empty secret: {url_key}"),
            })?;

        let base_url = base_url.trim_end_matches('/').to_owned();

        let token = ctx.secrets().get(token_key).await?;
        let basic = ctx.secrets().get(basic_auth_key).await?;

        let auth = if let Some(t) = token.filter(|s| !s.value.is_empty()) {
            Auth::Bearer(t.value)
        } else if let Some(b) = basic.filter(|s| !s.value.is_empty()) {
            let (user, password) =
                b.value
                    .split_once(':')
                    .ok_or_else(|| OperationError::Secret {
                        message: format!("{basic_auth_key} must be in 'user:password' format"),
                    })?;
            Auth::Basic {
                user: user.to_owned(),
                password: password.to_owned(),
            }
        } else {
            Auth::None
        };

        Ok(Self {
            base_url,
            auth,
            http: ctx.http_client().clone(),
        })
    }

    /// Set Bearer token authentication.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_ops_common::HttpApiClient;
    /// use reqwest::Client;
    ///
    /// let client = HttpApiClient::new("http://api.example.com", Client::new())
    ///     .with_bearer_token("my-token");
    /// ```
    #[must_use]
    pub fn with_bearer_token(mut self, token: &str) -> Self {
        self.auth = Auth::Bearer(token.to_owned());
        self
    }

    /// Set basic authentication.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_ops_common::HttpApiClient;
    /// use reqwest::Client;
    ///
    /// let client = HttpApiClient::new("http://api.example.com", Client::new())
    ///     .with_basic_auth("user", "password");
    /// ```
    #[must_use]
    pub fn with_basic_auth(mut self, user: &str, password: &str) -> Self {
        self.auth = Auth::Basic {
            user: user.to_owned(),
            password: password.to_owned(),
        };
        self
    }

    /// The base URL (without trailing slash).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The underlying HTTP client.
    pub fn http_client(&self) -> &Client {
        &self.http
    }

    /// The current authentication mode.
    pub fn auth(&self) -> &Auth {
        &self.auth
    }

    /// Build a full URL by appending `path` to the base URL.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_ops_common::HttpApiClient;
    /// use reqwest::Client;
    ///
    /// let client = HttpApiClient::new("http://api.example.com", Client::new());
    /// assert_eq!(client.url("/api/v1/status"), "http://api.example.com/api/v1/status");
    /// ```
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// Build a GET request to the given path, with authentication applied.
    pub fn get(&self, path: &str) -> RequestBuilder {
        self.authenticate(self.http.get(self.url(path)))
    }

    /// Build a POST request to the given path, with authentication applied.
    pub fn post(&self, path: &str) -> RequestBuilder {
        self.authenticate(self.http.post(self.url(path)))
    }

    /// Build a PUT request to the given path, with authentication applied.
    pub fn put(&self, path: &str) -> RequestBuilder {
        self.authenticate(self.http.put(self.url(path)))
    }

    /// Build a PATCH request to the given path, with authentication applied.
    pub fn patch(&self, path: &str) -> RequestBuilder {
        self.authenticate(self.http.patch(self.url(path)))
    }

    /// Build a DELETE request to the given path, with authentication applied.
    pub fn delete(&self, path: &str) -> RequestBuilder {
        self.authenticate(self.http.delete(self.url(path)))
    }

    fn authenticate(&self, builder: RequestBuilder) -> RequestBuilder {
        match &self.auth {
            Auth::None => builder,
            Auth::Bearer(token) => builder.bearer_auth(token),
            Auth::Basic { user, password } => builder.basic_auth(user, Some(password)),
        }
    }
}
