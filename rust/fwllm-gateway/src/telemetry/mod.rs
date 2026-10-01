//! OpenTelemetry prototype integration.
//!
//! This module is intentionally infrastructure-only: domain modules do not
//! depend on OpenTelemetry. Request spans are added in later prototype PRs.

mod context;
mod runtime;

pub use context::{extract_parent, trace_chat_request};
pub use runtime::{TelemetryInitError, TelemetryRuntime};
