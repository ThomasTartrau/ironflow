//! Series and label discovery operations.
//!
//! These operations query the Prometheus-compatible metadata endpoints
//! for series, labels, label values, metric metadata, and active series.

mod active;
mod labels;
mod metadata;
mod search;

pub use active::GetActiveSeries;
pub use labels::{GetLabelValues, GetLabels};
pub use metadata::GetMetadata;
pub use search::GetSeries;
