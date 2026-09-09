//! Alertmanager operations.
//!
//! These operations manage alerts and the Alertmanager configuration
//! within Mimir.

mod config;
mod query;

pub use config::{DeleteAlertmanagerConfig, SetAlertmanagerConfig};
pub use query::{GetAlertmanagerConfig, GetAlertmanagerConfigs, GetAlertmanagerStatus, GetAlerts};
