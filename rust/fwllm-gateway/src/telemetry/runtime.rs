//! OpenTelemetry runtime bootstrap for the prototype.

use fwllm_core::config::TelemetryConfig;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    propagation::TraceContextPropagator,
    trace::SdkTracerProvider,
    Resource,
};
use std::time::Duration;
use tracing_subscriber::{
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
};

#[derive(Debug, thiserror::Error)]
pub enum TelemetryInitError {
    #[error("failed to build OTLP span exporter: {0}")]
    Exporter(String),
    #[error("failed to install tracing subscriber: {0}")]
    Subscriber(String),
}

/// Keeps the SDK tracer provider alive for the lifetime of the process.
///
/// When telemetry is disabled the provider is None and the gateway uses the
/// existing fmt/env-filter subscriber only.
pub struct TelemetryRuntime {
    provider: Option<SdkTracerProvider>,
}

impl TelemetryRuntime {
    pub fn init(cfg: &TelemetryConfig) -> Result<Self, TelemetryInitError> {
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info"));
        let fmt_layer = tracing_subscriber::fmt::layer();

        if !cfg.enabled {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt_layer)
                .try_init()
                .map_err(|e| TelemetryInitError::Subscriber(e.to_string()))?;
            return Ok(Self { provider: None });
        }

        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(cfg.exporter.endpoint.clone())
            .with_timeout(Duration::from_millis(cfg.exporter.timeout_ms))
            .build()
            .map_err(|e| TelemetryInitError::Exporter(e.to_string()))?;

        let resource = Resource::builder()
            .with_service_name(cfg.service_name.clone())
            .build();

        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_resource(resource)
            .build();

        let tracer = provider.tracer("fwllm-gateway");
        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

        tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .with(otel_layer)
            .try_init()
            .map_err(|e| TelemetryInitError::Subscriber(e.to_string()))?;

        Ok(Self {
            provider: Some(provider),
        })
    }

    pub fn force_flush(&self) {
        if let Some(provider) = &self.provider {
            if let Err(err) = provider.force_flush() {
                eprintln!("OpenTelemetry force_flush failed: {err}");
            }
        }
    }
}

impl Drop for TelemetryRuntime {
    fn drop(&mut self) {
        if let Some(provider) = &self.provider {
            if let Err(err) = provider.shutdown() {
                eprintln!("OpenTelemetry shutdown failed: {err}");
            }
        }
    }
}
