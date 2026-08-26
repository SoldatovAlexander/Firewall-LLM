//! TDD Red: ingress token issuance + agent registry (PLAN 10.1 / 10.2).

use axum::body::Body;
use fwllm_gateway::providers::{Provider, ProviderError};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::future::Future;
use tower::ServiceExt;

const CLIENT_KEY: &str = "secret-client-key";

fn auth() -> String { format!("Bearer {CLIENT_KEY}") }

fn app() -> axum::Router {
    let cfg = fwllm_core::config::load_config_from_str(&format!(r#"
providers:
  p:
    type: openai_compat
    base_url: https://p.example/v1
clients:
  {CLIENT_KEY}: alice
"#)).unwrap();
    struct Noop;
    impl Provider for Noop {
        fn chat(&self, _p: Value) -> Pin<Box<dyn Future<Output = Result<Value, ProviderError>> + Send>> {
            Box::pin(async { Ok(json!({})) })
        }
    }
    let mut m = HashMap::new();
    m.insert("p".into(), Arc::new(Noop) as Arc<dyn Provider>);
    fwllm_gateway::build_app(cfg, Some(Arc::new(m)))
}

async fn body_json(resp: axum::response::Response) -> Value {
    let b = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&b).unwrap()
}

#[tokio::test]
async fn create_token_requires_auth() {
    let res = app().oneshot(
        axum::http::Request::builder().method("POST").uri("/admin/ingress/tokens")
            .header("content-type","application/json")
            .body(Body::from(json!({"agent_id":"a1"}).to_string())).unwrap()
    ).await.unwrap();
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn create_token_returns_token() {
    let res = app().oneshot(
        axum::http::Request::builder().method("POST").uri("/admin/ingress/tokens")
            .header("content-type","application/json")
            .header("authorization", auth())
            .body(Body::from(json!({"agent_id":"agent-1"}).to_string())).unwrap()
    ).await.unwrap();
    assert_eq!(res.status(), 200);
    let body = body_json(res).await;
    assert_eq!(body["agent_id"], "agent-1");
    assert!(body["token"].as_str().unwrap().len() >= 30);
}

#[tokio::test]
async fn list_agents_initially_empty() {
    let res = app().oneshot(
        axum::http::Request::builder().uri("/admin/ingress/agents")
            .header("authorization", auth())
            .body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(res.status(), 200);
    let body = body_json(res).await;
    assert!(body["agents"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn ws_handshake_requires_valid_token() {
    // invalid token must be rejected with 401, not 101 Switching Protocols
    let res = app().oneshot(
        axum::http::Request::builder().uri("/ingress")
            .header("authorization", "Bearer bad-token-xyz")
            .header("upgrade", "websocket")
            .header("connection", "Upgrade")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .header("sec-websocket-version", "13")
            .body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(res.status(), 401);
}
