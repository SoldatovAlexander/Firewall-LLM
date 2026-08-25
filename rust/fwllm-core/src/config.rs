//! Configuration loading: YAML file + FWLLM_* env overrides + api_key resolution.
//!
//! Mirrors the Python implementation and shares the same config schema.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse config: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("invalid config: {0}")]
    Validation(String),
}

fn default_port() -> u16 {
    8080
}

fn default_timeout() -> f64 {
    120.0
}

fn default_redis_url() -> String {
    "redis://localhost:6379/0".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_timeout", rename = "request_timeout_seconds")]
    pub request_timeout_seconds: f64,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            request_timeout_seconds: default_timeout(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    #[serde(rename = "type", default = "default_provider_type")]
    pub provider_type: String,
    pub base_url: String,
    #[serde(rename = "api_key_env")]
    pub api_key_env: Option<String>,
    /// Resolved from the environment; excluded from serialization.
    #[serde(skip_serializing)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
}

fn default_provider_type() -> String {
    "openai_compat".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Quotas {
    #[serde(rename = "client_tokens_per_day")]
    pub client_tokens_per_day: Option<i64>,
    #[serde(rename = "client_requests_per_day")]
    pub client_requests_per_day: Option<i64>,
    #[serde(rename = "provider_tokens_per_day")]
    pub provider_tokens_per_day: Option<i64>,
}

/// Threshold comparisons for routing rules.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Threshold {
    pub gt: Option<f64>,
    pub gte: Option<f64>,
    pub lt: Option<f64>,
    pub lte: Option<f64>,
}

impl Threshold {
    pub fn matches(&self, value: f64) -> bool {
        if let Some(gt) = self.gt {
            if !(value > gt) {
                return false;
            }
        }
        if let Some(gte) = self.gte {
            if !(value >= gte) {
                return false;
            }
        }
        if let Some(lt) = self.lt {
            if !(value < lt) {
                return false;
            }
        }
        if let Some(lte) = self.lte {
            if !(value <= lte) {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RuleCondition {
    pub provider: Option<String>,
    #[serde(rename = "provider_tokens_today")]
    pub provider_tokens_today: Option<Threshold>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RuleAction {
    #[serde(rename = "next_in_chain", default)]
    pub next_in_chain: bool,
    #[serde(rename = "switch_to")]
    pub switch_to: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RoutingRule {
    pub name: String,
    pub when: RuleCondition,
    pub action: RuleAction,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct AttackFailoverConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_count")]
    pub count: usize,
    #[serde(default = "default_window")]
    pub window_seconds: u64,
    #[serde(rename = "min_severity", default = "default_min_severity")]
    pub min_severity: String,
    #[serde(rename = "switch_to")]
    pub switch_to: Option<String>,
    #[serde(rename = "block_source", default = "default_true")]
    pub block_source: bool,
    #[serde(rename = "block_ttl_seconds", default = "default_block_ttl")]
    pub block_ttl_seconds: u64,
    #[serde(rename = "cooldown_seconds", default = "default_cooldown")]
    pub cooldown_seconds: u64,
}

fn default_count() -> usize {
    3
}
fn default_window() -> u64 {
    300
}
fn default_min_severity() -> String {
    "high".to_string()
}
fn default_true() -> bool {
    true
}
fn default_block_ttl() -> u64 {
    600
}
fn default_cooldown() -> u64 {
    300
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RoutingConfig {
    #[serde(rename = "default_chain", default)]
    pub default_chain: Vec<String>,
    #[serde(rename = "model_mapping", default)]
    pub model_mapping: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    pub rules: Vec<RoutingRule>,
    #[serde(rename = "attack_failover", default)]
    pub attack_failover: AttackFailoverConfig,
    #[serde(rename = "state_store", default = "default_state_store")]
    pub state_store: String,
}

fn default_state_store() -> String {
    "memory".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct EgressConfig {
    #[serde(default = "default_egress_mode")]
    pub mode: String,
    #[serde(rename = "proxy_url")]
    pub proxy_url: Option<String>,
}

fn default_egress_mode() -> String {
    "direct".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DlpConfig {
    #[serde(default = "default_dlp_mode")]
    pub mode: String,
    #[serde(rename = "restore_policy", default = "default_restore_policy")]
    pub restore_policy: String,
    #[serde(default = "default_profile")]
    pub profile: String,
}

fn default_dlp_mode() -> String {
    "mask".to_string()
}
fn default_restore_policy() -> String {
    "mask".to_string()
}
fn default_profile() -> String {
    "ru_152".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MlModelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(rename = "model_dir", default)]
    pub model_dir: String,
    #[serde(default = "default_ml_threshold")]
    pub threshold: f64,
}

impl Default for MlModelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model_dir: String::new(),
            threshold: 0.6,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct InjectionConfig {
    #[serde(default = "default_injection_mode")]
    pub mode: String,
    #[serde(
        rename = "block_severity_gte",
        default = "default_block_severity_gte"
    )]
    pub block_severity_gte: String,
    #[serde(default)]
    pub ml: MlModelConfig,
}

fn default_injection_mode() -> String {
    "block".to_string()
}
fn default_block_severity_gte() -> String {
    "high".to_string()
}
fn default_ml_threshold() -> f64 {
    0.6
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct InspectorsConfig {
    #[serde(default)]
    pub dlp: DlpConfig,
    #[serde(default)]
    pub injection: InjectionConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuditConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(rename = "db_path", default = "default_db_path")]
    pub db_path: String,
    #[serde(rename = "dlp_redact", default = "default_true")]
    pub dlp_redact: bool,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            db_path: default_db_path(),
            dlp_redact: true,
        }
    }
}

fn default_db_path() -> String {
    "audit.db".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default = "default_redis_url")]
    pub redis_url: String,
    pub providers: BTreeMap<String, ProviderConfig>,
    #[serde(default)]
    pub clients: BTreeMap<String, String>,
    #[serde(default)]
    pub quotas: Quotas,
    #[serde(default)]
    pub routing: RoutingConfig,
    #[serde(default)]
    pub egress: EgressConfig,
    #[serde(default)]
    pub inspectors: InspectorsConfig,
    #[serde(default)]
    pub audit: AuditConfig,
}

/// Load and validate configuration from a YAML file.
pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
    let raw = std::fs::read_to_string(path).map_err(ConfigError::Io)?;
    load_config_from_str(&raw)
}

/// Parse configuration from a YAML string (applies the same env overrides).
pub fn load_config_from_str(raw: &str) -> Result<Config, ConfigError> {
    let mut value: serde_yaml::Value =
        serde_yaml::from_str(raw).map_err(ConfigError::Parse)?;

    // env overrides (mirror Python behavior)
    if let Ok(port) = std::env::var("FWLLM_SERVER__PORT") {
        if let Some(server) = value.get_mut("server").and_then(|s| s.as_mapping_mut()) {
            if let Ok(p) = port.parse::<u64>() {
                server.insert(
                    serde_yaml::Value::from("port"),
                    serde_yaml::Value::from(p),
                );
            }
        } else {
            let mut m = serde_yaml::Mapping::new();
            if let Ok(p) = port.parse::<u64>() {
                m.insert(serde_yaml::Value::from("port"), serde_yaml::Value::from(p));
            }
            value.as_mapping_mut()
                .map(|mm| mm.insert(serde_yaml::Value::from("server"), serde_yaml::Value::Mapping(m)));
        }
    }
    if let Ok(redis) = std::env::var("FWLLM_REDIS_URL") {
        if let Some(mm) = value.as_mapping_mut() {
            mm.insert(
                serde_yaml::Value::from("redis_url"),
                serde_yaml::Value::from(redis),
            );
        }
    }
    if let Ok(tokens) = std::env::var("FWLLM_CLIENT_TOKENS") {
        let mut clients = serde_yaml::Mapping::new();
        for pair in tokens.split(',') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let (token, label) = match pair.split_once(':') {
                Some((t, l)) => (t, l),
                None => (pair, ""),
            };
            clients.insert(
                serde_yaml::Value::from(token),
                serde_yaml::Value::from(if label.is_empty() { token } else { label }),
            );
        }
        if let Some(mm) = value.as_mapping_mut() {
            mm.insert(
                serde_yaml::Value::from("clients"),
                serde_yaml::Value::Mapping(clients),
            );
        }
    }

    let mut cfg: Config = serde_yaml::from_value(value)?;

    // resolve api keys from environment
    for (name, provider) in cfg.providers.iter_mut() {
        if let Some(env_var) = &provider.api_key_env {
            let value = std::env::var(env_var).map_err(|_| {
                ConfigError::Validation(format!(
                    "environment variable '{env_var}' (api_key_env of provider '{name}') is not set"
                ))
            })?;
            provider.api_key = Some(value);
        }
    }

    Ok(cfg)
}
