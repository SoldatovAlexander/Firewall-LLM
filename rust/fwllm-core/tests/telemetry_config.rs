use fwllm_core::config::load_config_from_str;

const BASE: &str = r#"
server:
  host: 127.0.0.1
  port: 8080
providers:
  mock:
    type: openai_compat
    base_url: http://mock:8000/v1
ingress:
  enabled: false
"#;

#[test]
fn telemetry_is_disabled_by_default() {
    let cfg = load_config_from_str(BASE).expect("config should parse");
    assert!(!cfg.telemetry.enabled);
    assert_eq!(cfg.telemetry.service_name, "fwllm-gateway");
    assert_eq!(cfg.telemetry.exporter.protocol, "grpc");
    assert_eq!(cfg.telemetry.content.mode, "metadata_only");
}

#[test]
fn telemetry_explicit_config_parses() {
    let raw = format!(
        "{BASE}\ntelemetry:\n  enabled: true\n  service_name: fwllm-test\n  exporter:\n    protocol: grpc\n    endpoint: http://collector:4317\n    timeout_ms: 750\n  traces:\n    sampling_ratio: 0.5\n  content:\n    mode: metadata_only\n"
    );
    let cfg = load_config_from_str(&raw).expect("config should parse");
    assert!(cfg.telemetry.enabled);
    assert_eq!(cfg.telemetry.service_name, "fwllm-test");
    assert_eq!(cfg.telemetry.exporter.endpoint, "http://collector:4317");
    assert_eq!(cfg.telemetry.exporter.timeout_ms, 750);
    assert_eq!(cfg.telemetry.traces.sampling_ratio, 0.5);
}

#[test]
fn telemetry_rejects_unsupported_content_mode() {
    let raw = format!(
        "{BASE}\ntelemetry:\n  enabled: true\n  content:\n    mode: full_content\n"
    );
    let err = load_config_from_str(&raw).expect_err("full content must be rejected");
    assert!(err.to_string().contains("metadata_only"));
}
