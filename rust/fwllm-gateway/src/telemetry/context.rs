//! W3C Trace Context extraction and request-span middleware.

use axum::{
    body::Body,
    http::{HeaderMap, Request},
    middleware::Next,
    response::Response,
};
use opentelemetry::{global, propagation::Extractor, Context};
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;

struct HeaderExtractor<'a>(&'a HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|key| key.as_str()).collect()
    }
}

/// Extract an upstream W3C trace context from HTTP headers.
///
/// Missing or malformed headers are handled by the propagator as an empty
/// context, so the request span becomes a new root span.
pub fn extract_parent(headers: &HeaderMap) -> Context {
    global::get_text_map_propagator(|propagator| propagator.extract(&HeaderExtractor(headers)))
}

/// Wrap the chat-completions route in a stable gateway request span.
///
/// Authentication and request parsing remain domain concerns in the handler.
/// Once those succeed, the handler records safe request metadata on this span.
pub async fn trace_chat_request(request: Request<Body>, next: Next) -> Response {
    let parent = extract_parent(request.headers());
    let span = tracing::info_span!(
        "fwllm.request",
        otel.name = "fwllm.request",
        otel.kind = "server",
        "fwllm.telemetry.schema_version" = "prototype-v1",
        "fwllm.request.id" = tracing::field::Empty,
        "gen_ai.request.model" = tracing::field::Empty,
        "fwllm.stream" = tracing::field::Empty,
    );
    let _ = span.set_parent(parent);

    next.run(request).instrument(span).await
}
