//! TDD Red: tunnel egress provider forwards via agent with header masking.

use fwllm_gateway::providers::{Provider, ProviderError};
use serde_json::{json, Value};
use std::pin::Pin;
use std::future::Future;

// fake tunnel registry + provider will be injected; here we test masking directly
use fwllm_gateway::ingress::mask_for_tunnel;

#[test]
fn tunnel_masks_via_and_forwarded_headers() {
    let mut headers = std::collections::HashMap::new();
    headers.insert("Via".to_string(), "1.1 proxy".to_string());
    headers.insert("X-Forwarded-For".to_string(), "1.1.1.1".to_string());
    headers.insert("X-Real-Ip".to_string(), "2.2.2.2".to_string());
    headers.insert("Content-Type".to_string(), "application/json".to_string());
    let masked = mask_for_tunnel(headers);
    assert!(!masked.contains_key("Via"));
    assert!(!masked.contains_key("X-Forwarded-For"));
    assert!(!masked.contains_key("X-Real-Ip"));
    assert!(masked.contains_key("Content-Type"));
    assert!(masked.contains_key("User-Agent"));
}

#[tokio::test]
async fn tunnel_provider_forwards_and_returns_response() {
    // This test will be Green after TunnelProvider is implemented.
    // For now it is Red: TunnelProvider not yet exists.
    use fwllm_gateway::providers::TunnelProvider;
    use fwllm_gateway::ingress::shared_registry;
    let registry = shared_registry();
    // register a fake agent that echoes
    let agent_id = "test-agent";
    // issue a token and register
    let _entry = registry.issue_token(agent_id.to_string(), 1).await;
    registry.register_agent(agent_id.to_string()).await;

    let provider = TunnelProvider::new(agent_id.to_string(), registry.clone());
    let payload = json!({"model":"m","messages":[{"role":"user","content":"hi"}]});
    let res = provider.chat(payload.clone()).await;
    // With agent registered, tunnel returns synthetic tunneled response
    assert!(res.is_ok());
    // Without agent, should error
    let empty_registry = shared_registry();
    let orphan = TunnelProvider::new("ghost".to_string(), empty_registry);
    assert!(orphan.chat(payload.clone()).await.is_err());
}
