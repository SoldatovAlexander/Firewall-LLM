//! OpenTelemetry prototype integration.
//!
//! This module is intentionally infrastructure-only: domain modules do not
//! depend on OpenTelemetry. Request spans are added in later prototype PRs.

mod runtime;

pub use runtime::{TelemetryInitError, TelemetryRuntime};
