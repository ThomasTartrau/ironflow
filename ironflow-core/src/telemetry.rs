//! OpenTelemetry integration for distributed tracing.
//!
//! Provides a one-call setup for exporting [`tracing`] spans as OpenTelemetry
//! traces via the OTLP HTTP/protobuf protocol. All existing
//! `#[tracing::instrument]` annotations in the engine automatically become
//! OTel spans once the subscriber is installed. Console logging (`fmt` layer)
//! is preserved alongside the OTel layer.
//!
//! # Architecture
//!
//! ```text
//! tracing::instrument spans
//!        |
//!        v
//!  tracing-subscriber (fmt layer + OTel layer)
//!        |                    |
//!        v                    v
//!    stdout/stderr    opentelemetry SDK (BatchSpanProcessor)
//!                            |
//!                            v
//!                    OTLP HTTP exporter -> Jaeger / Tempo / any OTLP collector
//! ```
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_core::telemetry::{TelemetryConfig, init_telemetry};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! let _guard = init_telemetry(TelemetryConfig {
//!     service_name: "ironflow-worker".to_string(),
//!     otlp_endpoint: "http://localhost:4318".to_string(),
//! })?;
//!
//! // All tracing spans are now exported as OTel traces.
//! // Console logging is preserved.
//! // When `_guard` is dropped, the exporter flushes and shuts down.
//! # Ok(())
//! # }
//! ```

use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::fmt as fmt_layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

/// Configuration for OpenTelemetry trace export.
///
/// # Examples
///
/// ```
/// use ironflow_core::telemetry::TelemetryConfig;
///
/// let config = TelemetryConfig {
///     service_name: "my-service".to_string(),
///     otlp_endpoint: "http://localhost:4318".to_string(),
/// };
/// assert_eq!(config.service_name, "my-service");
/// ```
pub struct TelemetryConfig {
    /// The service name reported to the collector (e.g. `"ironflow-api"`).
    pub service_name: String,
    /// OTLP HTTP endpoint (e.g. `"http://localhost:4318"`).
    pub otlp_endpoint: String,
}

/// Guard that shuts down the OpenTelemetry tracer provider on drop.
///
/// Keep this value alive for the duration of the application. When dropped,
/// it flushes pending spans and releases resources.
pub struct OtelGuard {
    provider: SdkTracerProvider,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Err(e) = self.provider.shutdown() {
            eprintln!("OpenTelemetry shutdown error: {e}");
        }
    }
}

/// Initialise the OpenTelemetry tracing pipeline.
///
/// Installs a global [`tracing`] subscriber that combines an [`EnvFilter`]
/// (reading `RUST_LOG`), a `fmt` layer for console output, and an
/// OpenTelemetry layer exporting traces via OTLP HTTP/protobuf.
///
/// Returns an [`OtelGuard`] whose [`Drop`] implementation flushes pending
/// spans and shuts down the tracer provider. The caller must keep this
/// guard alive (typically in `main`).
///
/// # Errors
///
/// Returns an error if the OTLP exporter or tracer provider fails to
/// initialise (e.g. invalid endpoint).
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::telemetry::{TelemetryConfig, init_telemetry};
///
/// # fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
/// let _guard = init_telemetry(TelemetryConfig {
///     service_name: "ironflow-worker".to_string(),
///     otlp_endpoint: "http://localhost:4318".to_string(),
/// })?;
/// # Ok(())
/// # }
/// ```
pub fn init_telemetry(
    config: TelemetryConfig,
) -> Result<OtelGuard, Box<dyn std::error::Error + Send + Sync>> {
    let exporter = SpanExporter::builder()
        .with_http()
        .with_endpoint(&config.otlp_endpoint)
        .build()?;

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(
            Resource::builder()
                .with_service_name(config.service_name.clone())
                .build(),
        )
        .build();

    let tracer = provider.tracer(config.service_name);
    let otel_layer = OpenTelemetryLayer::new(tracer);

    Registry::default()
        .with(EnvFilter::from_default_env())
        .with(fmt_layer::layer())
        .with(otel_layer)
        .try_init()?;

    Ok(OtelGuard { provider })
}
