//! OpenTelemetry prototype integration.
//!
//! Domain modules stay independent of OpenTelemetry. This module adapts
//! request/security domain events to tracing/OTLP.

mod context;
mod runtime;
mod security;

pub use context::{extract_parent, trace_chat_request};
pub use runtime::{TelemetryInitError, TelemetryRuntime};
pub use security::record_security_finding;
