use axum::http::{HeaderMap, HeaderValue};
use fwllm_gateway::telemetry::extract_parent;
use opentelemetry::trace::TraceContextExt;
use opentelemetry_sdk::propagation::TraceContextPropagator;

fn install_propagator() {
    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
}

#[test]
fn extracts_valid_traceparent() {
    install_propagator();

    let mut headers = HeaderMap::new();
    headers.insert(
        "traceparent",
        HeaderValue::from_static(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        ),
    );

    let context = extract_parent(&headers);
    let span = context.span();
    let span_context = span.span_context();

    assert!(span_context.is_valid());
    assert_eq!(
        span_context.trace_id().to_string(),
        "4bf92f3577b34da6a3ce929d0e0e4736"
    );
}

#[test]
fn missing_traceparent_returns_empty_context() {
    install_propagator();

    let headers = HeaderMap::new();
    let context = extract_parent(&headers);
    let span = context.span();

    assert!(!span.span_context().is_valid());
}

#[test]
fn malformed_traceparent_is_ignored() {
    install_propagator();

    let mut headers = HeaderMap::new();
    headers.insert("traceparent", HeaderValue::from_static("not-a-traceparent"));

    let context = extract_parent(&headers);
    let span = context.span();

    assert!(!span.span_context().is_valid());
}
