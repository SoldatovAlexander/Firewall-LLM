use crate::error::ApiError;
use fwllm_core::config::InspectorsConfig;
use super::dlp::{DlpState, sanitize, deanonymize};
use super::injection::{scan, verdict};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ChainState {
    pub dlp: DlpState,
}

pub struct InspectorChain {
    cfg: InspectorsConfig,
}

impl InspectorChain {
    pub fn from_config(cfg: &InspectorsConfig) -> Self {
        Self { cfg: cfg.clone() }
    }

    pub fn process_request(&self, payload: &mut serde_json::Value) -> Result<ChainState, ApiError> {
        // 1. injection
        if self.cfg.injection.mode != "off" {
            let messages = payload.get("messages").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let findings = scan(&messages);
            if let Some((rule, severity)) = verdict(&findings, &self.cfg.injection.block_severity_gte) {
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
                            // check if any PII would be found
                            let mut tmp_vault = HashMap::new();
                            let mut tmp_scope = HashMap::new();
                            let sanitized = sanitize(&content, &mut tmp_vault, &mut tmp_scope);
                            if sanitized != content {
                                return Err(ApiError::blocked("sensitive data detected (DLP block mode)".to_string(), "dlp"));
                            }
                        } else if self.cfg.dlp.mode == "mask" {
                            let new_content = sanitize(&content, &mut vault, &mut scope);
                            msg["content"] = serde_json::Value::String(new_content);
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
