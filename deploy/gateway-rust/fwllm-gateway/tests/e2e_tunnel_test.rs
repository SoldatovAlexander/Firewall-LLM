//! E2E: gateway -> agent tunnel -> destination, verify header masking.

use axum::body::Body;
use axum::extract::Request;
use axum::routing::post;
use fwllm_gateway::ingress::{shared_registry, ProxyResponse};
use fwllm_gateway::providers::{Provider, TunnelProvider};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[tokio::test]
async fn e2e_tunnel_masks_headers() {
    // Mock destination that echoes received headers
    let received_headers: Arc<Mutex<Option<HashMap<String, String>>>> = Arc::new(Mutex::new(None));
    let received_clone = received_headers.clone();

    let mock_app = axum::Router::new().route(
        "/v1/chat/completions",
        post(move |req: Request| {
            let received_clone = received_clone.clone();
            async move {
                let headers: HashMap<String, String> = req
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect();
                *received_clone.lock().unwrap() = Some(headers);
                axum::Json(json!({
                    "id": "mock-1",
                    "object": "chat.completion",
                    "choices": [{"message": {"content": "ok"}}]
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, mock_app).await.unwrap(); });

    let mock_url = format!("http://{addr}/v1");
    let registry = shared_registry();
    let agent_id = "e2e-agent";

    // Create tunnel channel and spawn fake agent that forwards via reqwest with masking
    let (tx, mut rx) = mpsc::unbounded_channel();
    registry.register_tunnel(agent_id.to_string(), tx).await;

    tokio::spawn({
        let registry_clone = registry.clone();
        async move {
            while let Some(req) = rx.recv().await {
                // Fake agent: forward to mock_url with masked headers (already masked by registry)
                let client = reqwest::Client::new();
                let mut builder = client.post(&req.url).header("content-type", "application/json");
                for (k, v) in req.headers {
                    builder = builder.header(k, v);
                }
                let body = req.body.unwrap_or_default();
                let resp = builder.body(body).send().await.unwrap();
                let status = resp.status().as_u16();
                let text = resp.text().await.unwrap_or_default();
                let _ = req.responder.send(ProxyResponse {
                    status,
                    headers: HashMap::new(),
                    body: text,
                });
            }
        }
    });

    // Also register agent as connected (for TunnelProvider check)
    registry.register_agent(agent_id.to_string()).await;

    let provider = TunnelProvider::new(
        agent_id.to_string(),
        mock_url.clone(),
        None,
        registry.clone(),
    );

    // Simulate gateway sending a request with leaking headers that should be masked
    let payload = json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
    // We need to inject leaking headers via registry.forward direct test
    // Instead test via TunnelProvider which builds its own headers (content-type + auth)
    // To test masking, we directly call registry.forward with leaking headers
    let mut leaking_headers = HashMap::new();
    leaking_headers.insert("X-Forwarded-For".to_string(), "1.1.1.1".to_string());
    leaking_headers.insert("Via".to_string(), "1.1 proxy".to_string());
    leaking_headers.insert("Content-Type".to_string(), "application/json".to_string());

    let resp = registry
        .forward(
            agent_id,
            "POST".to_string(),
            format!("{}/chat/completions", mock_url),
            leaking_headers,
            Some(r#"{"test":1}"#.to_string()),
        )
        .await
        .unwrap();

    assert_eq!(resp.status, 200);
    // Verify mock destination did NOT receive masked headers
    let headers = received_headers.lock().unwrap().clone().unwrap();
    // header names are lowercased by axum/http
    assert!(!headers.keys().any(|k| k.to_ascii_lowercase() == "via"));
    assert!(!headers.keys().any(|k| k.to_ascii_lowercase() == "x-forwarded-for"));
    assert!(headers.keys().any(|k| k.to_ascii_lowercase() == "content-type"));

    // Also verify TunnelProvider path still works
    let res = provider.chat(payload).await;
    assert!(res.is_ok());
}
