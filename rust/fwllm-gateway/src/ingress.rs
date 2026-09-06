//! Ingress proxy: token issuance and agent registry.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, RwLock};

#[derive(Debug, Clone, serde::Serialize)]
pub struct TokenEntry {
    pub agent_id: String,
    pub token: String,
    pub expires_at: f64,
}

#[derive(Debug)]
pub struct ProxyRequest {
    pub id: String,
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub responder: oneshot::Sender<ProxyResponse>,
}

#[derive(Debug, Clone)]
pub struct ProxyResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
}

/// 0.1.1 tunnel hardening: bounded per-agent queue — bursts beyond this
/// fail fast with "agent overloaded" instead of growing memory without
/// bound (previous: unbounded channel).
pub const TUNNEL_QUEUE_DEPTH: usize = 16;
/// Heartbeat: gateway pings every interval; an agent silent longer than
/// the timeout is dropped and unregistered (stale-record cleanup).
pub const HEARTBEAT_INTERVAL_SECS: u64 = 30;
pub const HEARTBEAT_TIMEOUT_SECS: u64 = 90;

#[derive(Default)]
pub struct IngressRegistry {
    tokens: RwLock<HashMap<String, TokenEntry>>, // token -> entry
    agents: RwLock<HashMap<String, AgentConn>>,  // agent_id -> conn info
    tunnels: RwLock<HashMap<String, mpsc::Sender<ProxyRequest>>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentConn {
    pub agent_id: String,
    pub connected_at: f64,
    pub last_seen: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TokenSummary {
    pub agent_id: String,
    pub prefix: String,
    pub expires_at: f64,
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

    pub async fn list_token_summaries(&self) -> Vec<TokenSummary> {
        self.tokens
            .read()
            .await
            .values()
            .map(|e| TokenSummary {
                agent_id: e.agent_id.clone(),
                prefix: e.token.chars().take(6).collect(),
                expires_at: e.expires_at,
            })
            .collect()
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

    pub async fn register_tunnel(
        &self,
        agent_id: String,
        sender: mpsc::Sender<ProxyRequest>,
    ) {
        self.tunnels.write().await.insert(agent_id.clone(), sender);
        self.register_agent(agent_id).await;
    }

    /// 0.1.1: drop a dead agent's record AND its tunnel sender, so the
    /// agents list never lies and the next forward fails fast with
    /// "no tunnel" instead of hitting a dead channel.
    pub async fn unregister_agent(&self, agent_id: &str) {
        self.agents.write().await.remove(agent_id);
        self.tunnels.write().await.remove(agent_id);
    }

    /// 0.1.1: heartbeat pong path — refreshes last_seen.
    pub async fn touch_agent(&self, agent_id: &str) {
        if let Some(conn) = self.agents.write().await.get_mut(agent_id) {
            conn.last_seen = now_ts();
        }
    }

    /// 0.1.1: heartbeat helper — true when the agent is missing or silent
    /// longer than max_age.
    pub async fn is_stale(&self, agent_id: &str, max_age_secs: f64) -> bool {
        let now = now_ts();
        match self.agents.read().await.get(agent_id) {
            Some(conn) => now - conn.last_seen > max_age_secs,
            None => true,
        }
    }

    /// 0.1.1: sweep agents (and their tunnels) silent longer than max_age.
    /// `now` is a parameter so tests need no clock tricks.
    pub async fn sweep_stale(&self, now: f64, max_age_secs: f64) -> Vec<String> {
        let stale: Vec<String> = self
            .agents
            .read()
            .await
            .iter()
            .filter(|(_, c)| now - c.last_seen > max_age_secs)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &stale {
            self.unregister_agent(id).await;
        }
        stale
    }



    pub async fn forward(
        &self,
        agent_id: &str,
        method: String,
        url: String,
        headers: HashMap<String, String>,
        body: Option<String>,
    ) -> Result<ProxyResponse, String> {
        let sender = self
            .tunnels
            .read()
            .await
            .get(agent_id)
            .cloned()
            .ok_or_else(|| format!("no tunnel for agent {agent_id}"))?;
        let (tx, rx) = oneshot::channel();
        let req = ProxyRequest {
            id: format!("{}", rand::random::<u64>()),
            method,
            url,
            headers: mask_for_tunnel(headers),
            body,
            responder: tx,
        };
        // 0.1.1: bounded queue with honest backpressure — a full agent
        // queue fails fast instead of growing memory without bound.
        sender
            .try_send(req)
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => "agent overloaded".to_string(),
                mpsc::error::TrySendError::Closed(_) => "agent disconnected".to_string(),
            })?;
        tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_| "tunnel timeout".to_string())?
            .map_err(|_| "agent dropped".to_string())
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
    let has_ua = headers.keys().any(|k| k.eq_ignore_ascii_case("user-agent"));
    if !has_ua {
        headers.insert("User-Agent".to_string(), "Firewall-LLM-Agent/0.1".to_string());
    }
    headers
}
