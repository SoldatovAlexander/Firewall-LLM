//! Gateway integration tests via tower oneshot (no network).

use axum::body::Body;
use tower::ServiceExt;
use fwllm_gateway::providers::{Provider, ProviderError};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

const CLIENT_KEY: &str = "secret-client-key";

fn auth_header() -> (&'static str, String) {
    ("authorization", format!("Bearer {CLIENT_KEY}"))
}

struct FakeProvider {
    fail: bool,
    calls: Mutex<Vec<Value>>,
}

impl Provider for FakeProvider {
    fn chat(&self, payload: Value) -> Pin<Box<dyn Future<Output = Result<Value, ProviderError>> + Send>> {
        let fail = self.fail;
        self.calls.lock().unwrap().push(payload.clone());
        Box::pin(async move {
            if fail {
                return Err(ProviderError::Connection("upstream exploded".into()));
            }
            Ok(json!({
                "id": "chatcmpl-1",
                "object": "chat.completion",
                "created": 1_700_000_000,
                "model": payload["model"],
                "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hi!"}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5},
            }))
        })
    }
}

fn base_config(routing: Option<serde_yaml::Value>) -> fwllm_core::config::Config {
    let yaml = format!(
        r#"
providers:
  primary:
    type: openai_compat
    base_url: https://p.example/v1
  backup:
    type: openai_compat
    base_url: https://b.example/v1
clients:
  {CLIENT_KEY}: alice
{}
"#,
        routing.map(|r| serde_yaml::to_string(&r).unwrap()).unwrap_or_default()
    );
    fwllm_core::config::load_config_from_str(&yaml).unwrap()
}

fn app_with(fail: bool) -> axum::Router {
    let cfg = base_config(None);
    let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    providers.insert("primary".into(), Arc::new(FakeProvider { fail, calls: Mutex::new(vec![]) }));
    providers.insert("backup".into(), Arc::new(FakeProvider { fail, calls: Mutex::new(vec![]) }));
    fwllm_gateway::build_app(cfg, Some(Arc::new(providers)))
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn healthz_ok() {
    let res = app_with(false)
        .oneshot(
            axum::http::Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn metrics_endpoint_renders() {
    let res = app_with(false)
        .oneshot(
            axum::http::Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn missing_auth_is_401_contract_error() {
    let res = app_with(false)
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"m","messages":[{"role":"user","content":"hi"}]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
    let body = body_json(res).await;
    assert_eq!(body["error"]["type"], "authentication_error");
}

#[tokio::test]
async fn invalid_key_is_401() {
    let res = app_with(false)
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header(auth_header().0, "Bearer wrong")
                .body(Body::from(r#"{"model":"m","messages":[{"role":"user","content":"hi"}]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn invalid_body_maps_to_422_contract_error() {
    let res = app_with(false)
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header(auth_header().0, auth_header().1)
                .body(Body::from(r#"{"model":"m"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 422);
    let body = body_json(res).await;
    assert_eq!(body["error"]["type"], "invalid_request_error");
}

#[tokio::test]
async fn completion_success_passthrough() {
    let res = app_with(false)
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header(auth_header().0, auth_header().1)
                .body(Body::from(
                    r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = body_json(res).await;
    assert_eq!(body["object"], "chat.completion");
    assert_eq!(body["usage"]["total_tokens"], 5);
}

#[tokio::test]
async fn upstream_failure_maps_to_502() {
    let res = app_with(true)
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header(auth_header().0, auth_header().1)
                .body(Body::from(
                    r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 502);
    let body = body_json(res).await;
    assert_eq!(body["error"]["type"], "upstream_error");
}

#[tokio::test]
async fn model_mapping_sets_routed_from() {
    let routing: serde_yaml::Value = serde_yaml::from_str(
        r#"
routing:
  default_chain: [primary]
  model_mapping:
    gpt-4o:
      primary: gpt-4o-2024
"#,
    )
    .unwrap();

    let cfg = base_config(Some(routing));
    let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    let fake = Arc::new(FakeProvider { fail: false, calls: Mutex::new(vec![]) });
    providers.insert("primary".into(), fake.clone());
    providers.insert("backup".into(), fake);

    let res = fwllm_gateway::build_app(cfg, Some(Arc::new(providers)))
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header(auth_header().0, auth_header().1)
                .body(Body::from(
                    r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body = body_json(res).await;
    assert_eq!(body["routed_from"], "gpt-4o");
}
