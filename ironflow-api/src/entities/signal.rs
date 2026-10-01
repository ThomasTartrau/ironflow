//! Signal request and response DTOs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use ironflow_engine::signal::{SignalDelivery, SignalRejected, SignalResumed};
use ironflow_store::entities::Signal;

/// Send signal request body.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
pub struct SendSignalRequest {
    /// Signal name, e.g. `"ci.pipeline_finished"` (1-255 characters).
    pub name: String,
    /// Occurrence key, e.g. a commit SHA (1-255 characters).
    pub key: String,
    /// JSON payload, validated against the schema of each waiting step.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub payload: Value,
    /// Deduplication ID (1-255 characters): a second signal with the same ID
    /// is neither stored nor delivered.
    pub idempotency_id: Option<String>,
}

/// A run resumed by a signal.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct ResumedRunResponse {
    /// Run whose waiting step the signal resolved.
    pub run_id: Uuid,
    /// The resolved signal step.
    pub step_id: Uuid,
}

impl From<SignalResumed> for ResumedRunResponse {
    fn from(r: SignalResumed) -> Self {
        Self {
            run_id: r.run_id,
            step_id: r.step_id,
        }
    }
}

/// A waiting run whose payload schema the signal did not match.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct RejectedRunResponse {
    /// Run that keeps waiting.
    pub run_id: Uuid,
    /// The signal step that keeps waiting.
    pub step_id: Uuid,
    /// Why the payload was refused.
    pub error: String,
}

impl From<SignalRejected> for RejectedRunResponse {
    fn from(r: SignalRejected) -> Self {
        Self {
            run_id: r.run_id,
            step_id: r.step_id,
            error: r.error,
        }
    }
}

/// Result of sending a signal.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct SignalDeliveryResponse {
    /// ID of the stored signal (the pre-existing one on a duplicate).
    pub signal_id: Uuid,
    /// `true` when the idempotency ID was already used: nothing was delivered.
    pub duplicate: bool,
    /// Runs the signal resumed.
    pub resumed: Vec<ResumedRunResponse>,
    /// Waiting runs that rejected the payload and keep waiting.
    pub rejected: Vec<RejectedRunResponse>,
}

impl From<SignalDelivery> for SignalDeliveryResponse {
    fn from(d: SignalDelivery) -> Self {
        Self {
            signal_id: d.signal_id,
            duplicate: d.duplicate,
            resumed: d
                .resumed
                .into_iter()
                .map(ResumedRunResponse::from)
                .collect(),
            rejected: d
                .rejected
                .into_iter()
                .map(RejectedRunResponse::from)
                .collect(),
        }
    }
}

/// A stored signal.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct SignalResponse {
    /// Signal ID.
    pub id: Uuid,
    /// Signal name.
    pub name: String,
    /// Occurrence key.
    pub key: String,
    /// JSON payload.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub payload: Value,
    /// Deduplication ID given by the sender.
    pub idempotency_id: Option<String>,
    /// When the signal was received.
    pub received_at: DateTime<Utc>,
}

impl From<Signal> for SignalResponse {
    fn from(s: Signal) -> Self {
        Self {
            id: s.id,
            name: s.name,
            key: s.key,
            payload: s.payload,
            idempotency_id: s.idempotency_id,
            received_at: s.received_at,
        }
    }
}

/// Query parameters for listing signals.
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams, utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
pub struct ListSignalsQuery {
    /// Only signals with this exact name.
    pub name: Option<String>,
    /// Only signals with this exact key.
    pub key: Option<String>,
    /// Page number (1-based, defaults to 1).
    pub page: Option<u32>,
    /// Items per page (defaults to 20, max 100).
    pub per_page: Option<u32>,
}
