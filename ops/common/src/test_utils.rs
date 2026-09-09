//! Test utilities for ops crates.
//!
//! Available when the `test-utils` feature is enabled.

use std::collections::HashMap;

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{SecretResolver, SecretValue};

/// An in-memory [`SecretResolver`] backed by a [`HashMap`].
///
/// Useful for testing `from_context` methods without a real secret store.
///
/// # Examples
///
/// ```
/// use ironflow_ops_common::MapSecretResolver;
/// use ironflow_core::operation::{OperationContext, SecretResolver};
/// use std::collections::HashMap;
/// use std::sync::Arc;
///
/// let mut secrets = HashMap::new();
/// secrets.insert("api_url".into(), "http://localhost:8080".into());
/// let resolver = MapSecretResolver::new(secrets);
/// let ctx = OperationContext::new(Arc::new(resolver));
/// ```
pub struct MapSecretResolver(HashMap<String, String>);

impl MapSecretResolver {
    /// Create a new resolver from a map of key-value pairs.
    pub fn new(secrets: HashMap<String, String>) -> Self {
        Self(secrets)
    }
}

#[async_trait]
impl SecretResolver for MapSecretResolver {
    async fn get(&self, key: &str) -> Result<Option<SecretValue>, OperationError> {
        Ok(self.0.get(key).map(|v| SecretValue { value: v.clone() }))
    }
}
