//! Data source operations.
//!
//! Provides CRUD, lookup, and query operations for Grafana data sources
//! via the `/api/datasources/` endpoints.

mod crud;
mod lookup;

use serde::{Deserialize, Serialize};

pub use crud::{
    DataSourceCreate, DataSourceDelete, DataSourceList, DataSourceQuery, DataSourceUpdate,
};
pub use lookup::{DataSourceGetById, DataSourceGetByName, DataSourceGetByUid};

/// Data source metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSourceOutput {
    /// Data source numeric ID.
    pub id: Option<u64>,
    /// Data source UID.
    pub uid: Option<String>,
    /// Data source name.
    pub name: Option<String>,
    /// Data source type (e.g. `"prometheus"`, `"loki"`).
    #[serde(rename = "type")]
    pub type_name: Option<String>,
    /// Data source URL.
    pub url: Option<String>,
    /// Access mode (`"proxy"` or `"direct"`).
    pub access: Option<String>,
}
