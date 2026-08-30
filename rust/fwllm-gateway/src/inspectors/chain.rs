use crate::error::ApiError;
use fwllm_core::config::InspectorsConfig;
use super::dlp::{DlpState, sanitize, deanonymize};
use super::injection::{scan, verdict};
use super::ml::MlInjectionInspector;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ChainState {
    pub dlp: DlpState,
}

pub struct InspectorChain {
    cfg: InspectorsConfig,
    ml: Option<MlInjectionInspector>,
    publish: Option<std::sync::Arc<dyn Fn(String, String, String) + Send + Sync>>,
}

impl InspectorChain {
    pub fn from_config(cfg: &InspectorsConfig) -> Result<Self, fwllm_core::config::ConfigError> {
        let ml = if cfg.injection.ml.enabled {
            let classifier = crate::inspectors::ml::try_load_classifier(&cfg.injection.ml.model_dir)
                .ok_or_else(|| fwllm_core::config::ConfigError::Validation(
                    "injection.ml is enabled but no model could be loaded - provide a valid model_dir with model.onnx/tokenizer.json".to_string()
                ))?;
            Some(MlInjectionInspector::new(classifier, cfg.injection.ml.threshold as f32, cfg.injection.block_severity_gte.clone(), cfg.injection.mode.clone()))
        } else { None };
        Ok(Self { cfg: cfg.clone(), ml, publish: None })
    }

    #[cfg(test)]
    pub fn from_config_with_classifier(cfg: &InspectorsConfig, classifier: Box<dyn super::ml::TextClassifier>) -> Self {
        let ml = if cfg.injection.ml.enabled {
            Some(MlInjectionInspector::new(classifier, cfg.injection.ml.threshold as f32, cfg.injection.block_severity_gte.clone(), cfg.injection.mode.clone()))
        } else { None };
        Self { cfg: cfg.clone(), ml, publish: None }
    }

    pub fn set_publish<F>(&mut self, f: F)
    where
        F: Fn(String, String, String) + Send + Sync + 'static,
    {
        self.publish = Some(std::sync::Arc::new(f));
    }

    /// Fallback for tests and non-strict contexts: never fails, logs warning.
    pub fn from_config_or_default(cfg: &InspectorsConfig) -> Self {
        Self::from_config(cfg).unwrap_or_else(|e| {
            tracing::warn!("ML classifier not loaded, continuing without it: {e}");
            Self { cfg: cfg.clone(), ml: None, publish: None }
        })
    }

    pub fn process_request(&self, payload: &mut serde_json::Value) -> Result<ChainState, ApiError> {
        self.process_request_with_client(payload, None)
    }

    pub fn process_request_with_client(
        &self,
        payload: &mut serde_json::Value,
        client_id: Option<&str>,
    ) -> Result<ChainState, ApiError> {
        // 1. injection (signatures)
        let mut all_findings: Vec<(&'static str, &'static str)> = Vec::new();
        if self.cfg.injection.mode != "off" {
            let messages = payload.get("messages").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            all_findings.extend(scan(&messages));
            if let Some(ml) = &self.ml {
                for m in &messages {
                    if let Some(content) = m.get("content").and_then(|v| v.as_str()) {
                        if let Some(finding) = ml.scan(content) {
                            all_findings.push(finding);
                        }
                    }
                }
            }
            if let Some((rule, severity)) = verdict(&all_findings, &self.cfg.injection.block_severity_gte) {
                if let Some(publish) = &self.publish {
                    publish(severity.to_string(), rule.to_string(), client_id.unwrap_or("").to_string());
                }
                if self.cfg.injection.mode == "block" {
                    return Err(ApiError::blocked(format!("prompt injection detected ({rule}, severity={severity})"), "injection"));
                }
            }
        }
        // 2. dlp
        let mut vault = HashMap::new();
        let mut scope = HashMap::new();
        if self.cfg.dlp.mode != "off" {
            if let Some(messages) = payload.get_mut("messages").and_then(|v| v.as_array_mut()) {
                for msg in messages {
                    if let Some(content) = msg.get("content").and_then(|v| v.as_str()).map(|s| s.to_string()) {
                        if self.cfg.dlp.mode == "block" {
                            let mut tmp_vault = HashMap::new();
                            let mut tmp_scope = HashMap::new();
                            let sanitized = sanitize(&content, &mut tmp_vault, &mut tmp_scope);
                            if sanitized != content {
                                return Err(ApiError::blocked("sensitive data detected (DLP block mode)".to_string(), "dlp"));
                            }
                        } else if self.cfg.dlp.mode == "mask" {
                            let new_content = sanitize(&content, &mut vault, &mut scope);
                            msg["content"] = serde_json::Value::String(new_content);
                        } else if self.cfg.dlp.mode == "log" {
                            let mut tmp_vault = HashMap::new();
                            let mut tmp_scope = HashMap::new();
                            let _ = sanitize(&content, &mut tmp_vault, &mut tmp_scope);
                        }
                    }
                }
            }
        }
        Ok(ChainState { dlp: DlpState { vault, scope } })
    }

    pub fn process_response(&self, text: &str, state: &ChainState) -> String {
        if self.cfg.dlp.mode == "off" {
            return text.to_string();
        }
        let policy = if self.cfg.dlp.restore_policy == "restore" { "restore" } else { "mask" };
        deanonymize(text, &state.dlp.vault, &state.dlp.scope, policy)
    }
}
