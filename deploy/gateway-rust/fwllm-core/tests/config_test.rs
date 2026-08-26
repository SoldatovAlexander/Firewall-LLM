use fwllm_core::config::{load_config, ConfigError};
use std::io::Write;
use std::sync::Mutex;

/// cargo runs tests in parallel threads sharing the process environment;
/// serialize every test that touches FWLLM_* variables.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn write_cfg(dir: &std::path::Path, content: &str) -> std::path::PathBuf {
    let path = dir.join("fwllm.yaml");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path
}

/// Each test uses its own api_key_env variable: cargo runs tests in parallel
/// threads that share the process environment.
fn minimal_cfg(key_var: &str) -> String {
    format!(
        r#"
providers:
  openrouter:
    type: openrouter
    base_url: https://openrouter.ai/api/v1
    api_key_env: {key_var}
"#
    )
}

#[test]
fn loads_minimal_config_with_defaults() {
    let _env = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let key_var = "TEST_KEY_MINIMAL";
    std::env::set_var(key_var, "sk-test");
    let cfg = load_config(&write_cfg(dir.path(), &minimal_cfg(key_var))).unwrap();
    assert_eq!(cfg.server.port, 8080);
    assert_eq!(cfg.server.request_timeout_seconds, 120.0);
    assert_eq!(cfg.redis_url, "redis://localhost:6379/0");
    let p = cfg.providers.get("openrouter").unwrap();
    assert_eq!(p.base_url, "https://openrouter.ai/api/v1");
    assert_eq!(p.api_key.as_deref(), Some("sk-test"));
    assert_eq!(p.provider_type, "openrouter");
}

#[test]
fn missing_file_is_error() {
    let _env = ENV_LOCK.lock().unwrap();
    let err = load_config(std::path::Path::new("/nonexistent/fwllm.yaml")).unwrap_err();
    assert!(matches!(err, ConfigError::Io(_)));
}

#[test]
fn invalid_yaml_is_error() {
    let _env = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let err = load_config(&write_cfg(dir.path(), "server: [unclosed\n")).unwrap_err();
    assert!(matches!(err, ConfigError::Parse(_)));
}

#[test]
fn missing_provider_base_url_is_error() {
    let _env = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_cfg(
        dir.path(),
        "providers:\n  x:\n    api_key_env: KEY_X_MISSING\n",
    );
    std::env::remove_var("KEY_X_MISSING");
    let err = load_config(&cfg).unwrap_err();
    // serde surfaces it as Parse(missing field), Python wraps into Validation -
    // either way the offending field must be named
    let msg = err.to_string();
    assert!(msg.contains("base_url"), "{msg}");
}

#[test]
fn missing_api_key_env_var_is_error() {
    let _env = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::env::remove_var("DEFINITELY_NOT_SET_12345");
    let cfg = write_cfg(
        dir.path(),
        "providers:\n  x:\n    base_url: https://x.example/v1\n    api_key_env: DEFINITELY_NOT_SET_12345\n",
    );
    let err = load_config(&cfg).unwrap_err();
    match err {
        ConfigError::Validation(msg) => {
            assert!(msg.contains("DEFINITELY_NOT_SET_12345"), "{msg}")
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn env_overrides_apply() {
    let _env = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let key_var = "TEST_KEY_OVERRIDE";
    std::env::set_var(key_var, "sk-1");
    std::env::set_var("FWLLM_SERVER__PORT", "9090");
    std::env::set_var("FWLLM_REDIS_URL", "redis://cache:6379/1");
    let cfg = load_config(&write_cfg(dir.path(), &minimal_cfg(key_var))).unwrap();
    assert_eq!(cfg.server.port, 9090);
    assert_eq!(cfg.redis_url, "redis://cache:6379/1");
    std::env::remove_var("FWLLM_SERVER__PORT");
    std::env::remove_var("FWLLM_REDIS_URL");
}

#[test]
fn client_tokens_from_env() {
    let _env = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let key_var = "TEST_KEY_CLIENTS";
    std::env::set_var(key_var, "sk-1");
    std::env::set_var("FWLLM_CLIENT_TOKENS", "tok1:alice,tok2:bob");
    let cfg = load_config(&write_cfg(dir.path(), &minimal_cfg(key_var))).unwrap();
    assert_eq!(cfg.clients.get("tok1").map(String::as_str), Some("alice"));
    assert_eq!(cfg.clients.get("tok2").map(String::as_str), Some("bob"));
    std::env::remove_var("FWLLM_CLIENT_TOKENS");
}

#[test]
fn routing_and_attack_failover_parse() {
    let _env = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let key_var = "TEST_KEY_ROUTING";
    std::env::set_var(key_var, "sk-1");
    let cfg_src = format!(
        r#"{}
routing:
  default_chain: [primary, backup]
  model_mapping:
    gpt-4o:
      primary: "gpt-4o-2024"
      backup: "llama3.3:70b"
  attack_failover:
    enabled: true
    count: 5
    window_seconds: 300
    min_severity: high
    switch_to: backup
"#,
        minimal_cfg(key_var)
    );
    let cfg = load_config(&write_cfg(dir.path(), &cfg_src)).unwrap();
    let r = &cfg.routing;
    assert_eq!(r.default_chain, vec!["primary", "backup"]);
    assert_eq!(
        r.model_mapping.get("gpt-4o").and_then(|m| m.get("primary")),
        Some(&"gpt-4o-2024".to_string())
    );
    let af = &r.attack_failover;
    assert!(af.enabled);
    assert_eq!(af.count, 5);
    assert_eq!(af.switch_to.as_deref(), Some("backup"));
}
