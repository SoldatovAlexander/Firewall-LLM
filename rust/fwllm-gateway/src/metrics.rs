//! Prometheus business metrics (same names as the Python branch).

use prometheus::{
    register_histogram_vec, register_int_counter_vec, HistogramVec, IntCounterVec,
};
use std::sync::OnceLock;

struct Metrics {
    requests: IntCounterVec,
    tokens: IntCounterVec,
    duration: HistogramVec,
    audit_errors: prometheus::IntCounter,
}

static METRICS: OnceLock<Metrics> = OnceLock::new();

fn metrics() -> &'static Metrics {
    METRICS.get_or_init(|| Metrics {
        requests: register_int_counter_vec!(
            "fw_requests_total",
            "Total chat completions processed",
            &["client", "provider", "model", "code"]
        )
        .unwrap(),
        tokens: register_int_counter_vec!(
            "fw_tokens_total",
            "Tokens processed",
            &["client", "provider", "model", "direction"]
        )
        .unwrap(),
        duration: register_histogram_vec!(
            "fw_request_duration_seconds",
            "Upstream request duration",
            &["provider", "model"]
        )
        .unwrap(),
        audit_errors: prometheus::register_int_counter!(
            "fw_audit_errors_total",
            "Audit storage write failures"
        )
        .unwrap(),
    })
}

/// R12: audit storage failure is a visible metric with a consistent policy
/// (log + count; the request itself is unaffected).
pub fn audit_error() {
    metrics().audit_errors.inc();
}

#[allow(clippy::too_many_arguments)]
pub fn observe_request(
    client: &str,
    provider: &str,
    model: &str,
    code: &str,
    duration_seconds: f64,
    prompt_tokens: u64,
    completion_tokens: u64,
) {
    let m = metrics();
    m.requests
        .with_label_values(&[client, provider, model, code])
        .inc();
    if prompt_tokens > 0 {
        m.tokens
            .with_label_values(&[client, provider, model, "prompt"])
            .inc_by(prompt_tokens);
    }
    if completion_tokens > 0 {
        m.tokens
            .with_label_values(&[client, provider, model, "completion"])
            .inc_by(completion_tokens);
    }
    m.duration
        .with_label_values(&[provider, model])
        .observe(duration_seconds);
}

pub fn render_metrics() -> String {
    use prometheus::Encoder;
    let encoder = prometheus::TextEncoder::new();
    let mut buffer = Vec::new();
    encoder
        .encode(&prometheus::default_registry().gather(), &mut buffer)
        .ok();
    String::from_utf8(buffer).unwrap_or_default()
}
