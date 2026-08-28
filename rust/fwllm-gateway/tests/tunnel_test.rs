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
    use fwllm_gateway::ingress::{shared_registry, ProxyResponse};
    use fwllm_gateway::providers::TunnelProvider;
    use tokio::sync::mpsc;

    let registry = shared_registry();
    let agent_id = "test-agent";
    let _entry = registry.issue_token(agent_id.to_string(), 1).await;
    // Register a real tunnel channel with a handler that returns a proper JSON
    let (tx, mut rx) = mpsc::unbounded_channel();
    registry.register_tunnel(agent_id.to_string(), tx).await;
    tokio::spawn(async move {
        while let Some(req) = rx.recv().await {
            let _ = req.responder.send(ProxyResponse {
                status: 200,
                headers: std::collections::HashMap::new(),
                body: r#"{"id":"tunnel-1","object":"chat.completion","choices":[{"message":{"content":"tunneled"}}]}"#.to_string(),
            });
        }
    });

    let provider = TunnelProvider::new(agent_id.to_string(), "https://api.example.com/v1".to_string(), None, registry.clone());
    let payload = json!({"model":"m","messages":[{"role":"user","content":"hi"}]});
    let res = provider.chat(payload.clone()).await;
    assert!(res.is_ok());
    assert_eq!(res.unwrap()["choices"][0]["message"]["content"], "tunneled");
    // Without agent, should error
    let empty_registry = shared_registry();
    let orphan = TunnelProvider::new("ghost".to_string(), "https://api.example.com/v1".to_string(), None, empty_registry);
    assert!(orphan.chat(payload.clone()).await.is_err());
}
