//! R12: exactly one final audit row per request lifecycle.

use axum::body::Body;
use fwllm_gateway::providers::{ChatFuture, Provider, ProviderError};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const CLIENT_KEY: &str = "secret-client-key";

fn auth() -> (String, String) {
    ("authorization".into(), format!("Bearer {CLIENT_KEY}"))
}

fn config(db: &std::path::Path) -> fwllm_core::config::Config {
    let yaml = format!(
        r#"
providers:
  primary:
    type: openai_compat
    base_url: https://p.example/v1
clients:
  {CLIENT_KEY}: alice
admin_clients:
  admin-key-1: admin
audit:
  enabled: true
  db_path: {}
"#,
        db.to_string_lossy()
    );
    fwllm_core::config::load_config_from_str(&yaml).unwrap()
}

struct Fake {
    fail_stream_open: bool,
    calls: Mutex<Vec<Value>>,
}

impl Provider for Fake {
    fn chat(&self, payload: Value) -> ChatFuture {
        self.calls.lock().unwrap().push(payload.clone());
        Box::pin(async move {
            Ok(json!({
                "choices": [{"message": {"role": "assistant", "content": "Hi!"}}],
                "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5},
            }))
        })
    }
    fn chat_stream(&self, _p: Value) -> fwllm_gateway::providers::StreamFuture {
        use futures_util::stream;
        if self.fail_stream_open {
            return Box::pin(async move {
                Err::<
                    Pin<Box<dyn futures_util::Stream<Item = Result<Value, ProviderError>> + Send>>,
                    ProviderError,
                >(ProviderError::Connection("boom".into()))
            });
        }
        Box::pin(async move {
            let items: Vec<Result<Value, ProviderError>> = vec![
                Ok(json!({"choices":[{"delta":{"content":"Hi"}}]})),
                Ok(json!({"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2}})),
            ];
            Ok(Box::pin(stream::iter(items))
                as Pin<Box<dyn futures_util::Stream<Item = Result<Value, ProviderError>> + Send>>)
        })
    }
}

fn setup(
    db: &std::path::Path,
    fail_stream_open: bool,
    quotas: Option<fwllm_core::config::Quotas>,
) -> (axum::Router, Arc<fwllm_gateway::state::AppState>, Arc<Fake>) {
    let fake = Arc::new(Fake { fail_stream_open, calls: Mutex::new(vec![]) });
    let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    providers.insert("primary".into(), fake.clone());
    let metering = quotas.map(|q| {
        fwllm_gateway::metering::Metering::new(
            Box::new(fwllm_gateway::metering::InMemoryStore::default()),
            &q,
        )
    });
    let (app, state) =
        fwllm_gateway::build_app_full(config(db), Some(Arc::new(providers)), metering);
    (app, state, fake)
}

fn quotas(requests: Option<i64>) -> fwllm_core::config::Quotas {
    fwllm_core::config::Quotas {
        client_tokens_per_day: None,
        client_requests_per_day: requests,
        provider_tokens_per_day: None,
        backend_fail_closed: false,
        completion_reserve_tokens: 1024,
    }
}

async fn post(app: &axum::Router, body: &str) -> axum::response::Response {
    let (name, value) = auth();
    app.clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header(name, value)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

fn rows(state: &Arc<fwllm_gateway::state::AppState>) -> Vec<fwllm_gateway::audit::AuditRecord> {
    state.audit.as_ref().unwrap().search(None, None, 100)
}

#[tokio::test]
async fn non_stream_success_audited_once_with_request_id() {
    let dir = tempfile::tempdir().unwrap();
    let (app, state, _fake) = setup(&dir.path().join("a.db"), false, None);
    let res = post(&app, r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#).await;
    assert_eq!(res.status(), 200);
    let all = rows(&state);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].code, "ok");
    assert!(!all[0].request_id.is_empty());
    assert_eq!(all[0].prompt_tokens + all[0].completion_tokens, 5);
}

#[tokio::test]
async fn quota_denial_audited_once() {
    let dir = tempfile::tempdir().unwrap();
    let (app, state, _fake) = setup(&dir.path().join("a.db"), false, Some(quotas(Some(1))));
    let body = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#;
    assert_eq!(post(&app, body).await.status(), 200);
    assert_eq!(post(&app, body).await.status(), 429);
    let all = rows(&state);
    assert_eq!(all.len(), 2);
    assert_eq!(all.iter().filter(|r| r.code == "rate_limited").count(), 1);
}

#[tokio::test]
async fn stream_success_audited_once() {
    let dir = tempfile::tempdir().unwrap();
    let (app, state, _fake) = setup(&dir.path().join("a.db"), false, None);
    let res = post(&app, r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"stream":true}"#).await;
    assert_eq!(res.status(), 200);
    let _ = res.into_body().collect().await.unwrap().to_bytes();
    let all = rows(&state);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].code, "ok");
    assert!(!all[0].request_id.is_empty());
}

#[tokio::test]
async fn stream_open_failure_audited_as_upstream_error() {
    let dir = tempfile::tempdir().unwrap();
    let (app, state, _fake) = setup(&dir.path().join("a.db"), true, None);
    let res = post(&app, r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"stream":true}"#).await;
    assert_eq!(res.status(), 502);
    let all = rows(&state);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].code, "upstream_error");
}

#[tokio::test]
async fn stream_disconnect_audited_as_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let (app, state, _fake) = setup(&dir.path().join("a.db"), false, None);
    let res = post(&app, r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"stream":true}"#).await;
    assert_eq!(res.status(), 200);
    drop(res); // client disconnect without reading: exactly one cancelled row
    let all = rows(&state);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].code, "cancelled");
}
