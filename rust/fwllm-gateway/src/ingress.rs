//! Ingress proxy: token issuance and agent registry.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, serde::Serialize)]
pub struct TokenEntry {
    pub agent_id: String,
    pub token: String,
    pub expires_at: f64,
}

#[derive(Default)]
pub struct IngressRegistry {
    tokens: RwLock<HashMap<String, TokenEntry>>, // token -> entry
    agents: RwLock<HashMap<String, AgentConn>>,  // agent_id -> conn info
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentConn {
    pub agent_id: String,
    pub connected_at: f64,
    pub last_seen: f64,
}

impl IngressRegistry {
    pub async fn issue_token(&self, agent_id: String, ttl_hours: u64) -> TokenEntry {
        let token = generate_token();
        let expires_at = now_ts() + (ttl_hours as f64) * 3600.0;
        let entry = TokenEntry { agent_id: agent_id.clone(), token: token.clone(), expires_at };
        self.tokens.write().await.insert(token, entry.clone());
        entry
    }

    pub async fn list_tokens(&self) -> Vec<TokenEntry> {
        self.tokens.read().await.values().cloned().collect()
    }

    pub async fn validate_token(&self, token: &str) -> Option<TokenEntry> {
        let entry = self.tokens.read().await.get(token).cloned()?;
        if entry.expires_at < now_ts() { None } else { Some(entry) }
    }

    pub async fn register_agent(&self, agent_id: String) {
        let now = now_ts();
        self.agents.write().await.insert(agent_id.clone(), AgentConn { agent_id, connected_at: now, last_seen: now });
    }

    pub async fn list_agents(&self) -> Vec<AgentConn> {
        self.agents.read().await.values().cloned().collect()
    }
}

fn generate_token() -> String {
    use rand::distributions::{Alphanumeric, DistString};
    Alphanumeric.sample_string(&mut rand::thread_rng(), 43)
}

fn now_ts() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

pub fn shared_registry() -> Arc<IngressRegistry> {
    Arc::new(IngressRegistry::default())
}

/// Header masking for tunnel forwarding (called both on gateway and agent).
pub fn mask_for_tunnel(
    mut headers: std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, String> {
    let to_remove = [
        "via",
        "x-forwarded-for",
        "x-forwarded-proto",
        "x-forwarded-host",
        "x-real-ip",
        "x-forwarded-port",
        "cf-connecting-ip",
        "cf-ray",
        "server",
        "x-powered-by",
    ];
    for key in to_remove {
        headers.remove(key);
        headers.remove(&key.to_uppercase());
        // case-insensitive removal
        headers.retain(|k, _| k.to_ascii_lowercase() != key);
    }
    let has_ua = headers.keys().any(|k| k.to_ascii_lowercase() == "user-agent");
    if !has_ua {
        headers.insert("User-Agent".to_string(), "Firewall-LLM-Agent/0.1".to_string());
    }
    headers
}
