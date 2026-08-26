//! fwllm-gateway: production Rust gateway (axum).

pub mod audit;
pub mod error;
pub mod ingress;
pub mod metering;
pub mod metrics;
pub mod providers;
pub mod router;
pub mod inspectors;
pub mod state;

use crate::error::ApiError;
use crate::state::AppState;
use axum::body::Body;
use bytes::Bytes;
use axum::extract::{rejection::JsonRejection, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use fwllm_core::config::Config;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

/// Build the full application router.
///
/// `providers` are injected so tests can substitute deterministic adapters;
/// `None` builds HTTP clients from the config (production path).
pub fn build_app(
    config: Config,
    providers: Option<Arc<providers::ProviderRegistry>>,
) -> Router {
    build_app_with_metering(config, providers, None)
}

pub fn build_app_with_metering(
    config: Config,
    providers: Option<Arc<providers::ProviderRegistry>>,
    metering: Option<metering::Metering>,
) -> Router {
    let audit = match audit::AuditLog::open(&config.audit) {
        Ok(log) => Some(Arc::new(log)),
        Err(e) => {
            tracing::warn!("audit disabled: {e}");
            None
        }
    };
    let state = AppState::build(config, providers, metering, audit);
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/admin/audit", get(admin_audit))
        .route("/admin/ingress/tokens", post(create_ingress_token))
        .route("/admin/ingress/agents", get(list_ingress_agents))
        .route("/ingress", get(ingress_ws_handler))
        .route("/metrics", get(metrics_handler))
        .with_state(state)
}

async fn admin_audit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    if let Err(err) = require_client(&state, &headers).await {
        return err.into_response();
    }
    let Some(audit) = &state.audit else {
        return (StatusCode::OK, Json(json!({"total": 0, "records": []}))).into_response();
    };
    let records = audit.search(
        params.get("client").map(String::as_str),
        params.get("code").map(String::as_str),
        params
            .get("limit")
            .and_then(|l| l.parse().ok())
            .unwrap_or(100),
    );
    let total = records.len();
    (StatusCode::OK, Json(json!({"total": total, "records": records}))).into_response()
}

#[derive(Debug, Deserialize)]
struct CreateTokenRequest {
    agent_id: String,
    #[serde(default = "default_ttl")]
    ttl_hours: u64,
}
fn default_ttl() -> u64 { 168 }

async fn create_ingress_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<CreateTokenRequest>, JsonRejection>,
) -> Response {
    if let Err(err) = require_client(&state, &headers).await {
        return err.into_response();
    }
    let Json(req) = match body {
        Ok(b) => b,
        Err(e) => return ApiError::invalid_request(format!("invalid body: {e}")).into_response(),
    };
    if req.agent_id.trim().is_empty() {
        return ApiError::invalid_request("agent_id is required").into_response();
    }
    let entry = state.ingress.issue_token(req.agent_id, req.ttl_hours).await;
    (StatusCode::OK, Json(json!({"token": entry.token, "agent_id": entry.agent_id, "expires_at": entry.expires_at}))).into_response()
}

async fn list_ingress_agents(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    if let Err(err) = require_client(&state, &headers).await {
        return err.into_response();
    }
    let tokens = state.ingress.list_tokens().await;
    let agents = state.ingress.list_agents().await;
    (StatusCode::OK, Json(json!({"tokens": tokens, "agents": agents}))).into_response()
}

async fn ingress_ws_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ws: Option<axum::extract::ws::WebSocketUpgrade>,
) -> Response {
    let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
    let token = auth.strip_prefix("Bearer ").map(str::trim).unwrap_or("");
    let entry = match state.ingress.validate_token(token).await {
        Some(e) => e,
        None => return ApiError::auth("invalid ingress token", "invalid_api_key").into_response(),
    };
    let Some(ws) = ws else {
        return (StatusCode::UPGRADE_REQUIRED, "Upgrade Required").into_response();
    };
    let agent_id = entry.agent_id.clone();
    let registry = state.ingress.clone();
    ws.on_upgrade(move |socket| async move {
        registry.register_agent(agent_id.clone()).await;
        handle_ingress_socket(socket, agent_id).await;
    })
}

async fn handle_ingress_socket(mut socket: axum::extract::ws::WebSocket, _agent_id: String) {
    use axum::extract::ws::Message;
    while let Some(Ok(msg)) = socket.recv().await {
        match msg {
            Message::Text(text) => {
                // mask Via/X-Forwarded headers if present in forwarded payload
                let _ = text;
                let _ = socket.send(Message::Text(text)).await;
            }
            Message::Ping(d) => { let _ = socket.send(Message::Pong(d)).await; }
            Message::Close(_) => break,
            _ => {}
        }
    }
}

async fn metrics_handler() -> String {
    metrics::render_metrics()
}

async fn healthz() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

/// SSE passthrough: chunks from the provider are re-emitted as
/// `data: {...}\n\n` lines terminated by `data: [DONE]`.
#[allow(clippy::too_many_arguments)]
async fn stream_response(
    _state: &Arc<AppState>,
    client_id: &str,
    provider_name: &str,
    model: &str,
    provider: Arc<dyn providers::Provider>,
    payload: Value,
    started: Instant,
) -> Response {
    let client_id = client_id.to_string();
    let provider_name = provider_name.to_string();
    let model = model.to_string();
    match provider.chat_stream(payload).await {
        Ok(stream) => {
            use futures_util::StreamExt;
            let client = client_id.to_string();
            let provider_tag = provider_name.to_string();
            let model_tag = model.to_string();
            let mapped = stream.map(move |item| match item {
                Ok(chunk) => Ok::<Bytes, std::convert::Infallible>(Bytes::from(format!(
                    "data: {}\n\n",
                    chunk
                ))),
                Err(err) => {
                    metrics::observe_request(
                        &client,
                        &provider_tag,
                        &model_tag,
                        "upstream_error",
                        started.elapsed().as_secs_f64(),
                        0,
                        0,
                    );
                    Ok(Bytes::from(format!(
                        "data: {}\n\n",
                        json!({"error": {"message": err.to_string(), "type": "upstream_error"}})
                    )))
                }
            });
            let with_done =
                mapped.chain(futures_util::stream::once(async move {
                    metrics::observe_request(
                        &client_id,
                        &provider_name,
                        &model,
                        "ok",
                        started.elapsed().as_secs_f64(),
                        0,
                        0,
                    );
                    Ok::<Bytes, std::convert::Infallible>(Bytes::from("data: [DONE]\n\n"))
                }));
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/event-stream")
                .body(Body::from_stream(with_done))
                .unwrap()
        }
        Err(err) => {
            metrics::observe_request(
                &client_id,
                &provider_name,
                &model,
                "upstream_error",
                started.elapsed().as_secs_f64(),
                0,
                0,
            );
            ApiError::upstream(err.to_string()).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    #[allow(dead_code)]
    role: String,
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(default)]
    stream: bool,
}

async fn require_client(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<String, ApiError> {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let token = auth.strip_prefix("Bearer ").map(str::trim).unwrap_or("");
    if token.is_empty() {
        return Err(ApiError::auth("missing bearer token", "missing_api_key"));
    }
    if state.clients.is_empty() || !state.clients.contains_key(token) {
        return Err(ApiError::auth("invalid API key", "invalid_api_key"));
    }
    Ok(state.clients.get(token).cloned().unwrap_or_default())
}

async fn chat_completions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<ChatCompletionRequest>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(b) => b,
        Err(rejection) => {
            return ApiError::invalid_request(format!("invalid request body: {rejection}"))
                .into_response()
        }
    };

    let client_id = match require_client(&state, &headers).await {
        Ok(id) => id,
        Err(err) => return err.into_response(),
    };

    let provider_name;
    let concrete_model;
    {
        let mut router = state.router.lock().await;
        match router.resolve(&body.model, &client_id) {
            Ok((provider, model)) => {
                provider_name = provider;
                concrete_model = model;
            }
            Err(blocked) => {
                metrics::observe_request(
                    &client_id,
                    "unrouted",
                    &body.model,
                    "blocked_source",
                    0.0,
                    0,
                    0,
                );
                return ApiError::blocked(blocked.message, "blocked_source")
                    .into_response();
            }
        }
    }

    let started = Instant::now();

    // quota gate -> 429
    if let Some(metering) = &state.metering {
        if let Err(quota_err) = metering.check_client(&client_id) {
            metrics::observe_request(&client_id, &provider_name, &body.model, "rate_limited", 0.0, 0, 0);
            return ApiError::rate_limited(format!(
                "daily {} quota exceeded ({}/{})",
                quota_err.scope, 0, quota_err.limit
            ))
            .into_response();
        }
    }

    let provider = match state.providers.get(&provider_name) {
        Some(p) => p.clone(),
        None => {
            metrics::observe_request(
                &client_id,
                &provider_name,
                &body.model,
                "upstream_error",
                0.0,
                0,
                0,
            );
            return ApiError::upstream(format!(
                "routed provider '{provider_name}' not configured"
            ))
            .into_response();
        }
    };

    if body.stream {
        let payload = json!({
            "model": concrete_model,
            "messages": body.messages.iter()
                .map(|m| json!({"role": m.role, "content": m.content}))
                .collect::<Vec<_>>(),
            "stream": true,
        });
        return stream_response(&state, &client_id, &provider_name, &body.model, provider, payload, started)
            .await;
    }

    let mut payload = json!({
        "model": concrete_model,
        "messages": body.messages.iter()
            .map(|m| json!({"role": m.role, "content": m.content}))
            .collect::<Vec<_>>(),
        "stream": false,
    });

    let chain_state = match state.inspectors.process_request(&mut payload) {
        Ok(s) => s,
        Err(e) => {
            metrics::observe_request(&client_id, &provider_name, &body.model, "blocked", 0.0, 0, 0);
            if let Some(audit) = &state.audit {
                audit.write(&client_id, &provider_name, &body.model, "blocked", 0, 0, &serde_json::to_string(&payload).unwrap_or_default(), &e.message);
            }
            return e.into_response();
        }
    };

    let payload_json = serde_json::to_string(&payload).unwrap_or_default();
    let result = provider.chat(payload.take()).await;
    let duration = started.elapsed().as_secs_f64();

    match result {
        Ok(mut completion) => {
            if concrete_model != body.model {
                completion["routed_from"] = json!(body.model);
            }
            // DLP restore/mask on response
            if let Some(content) = completion["choices"][0]["message"]["content"].as_str().map(|s| s.to_string()) {
                let restored = state.inspectors.process_response(&content, &chain_state);
                completion["choices"][0]["message"]["content"] = serde_json::Value::String(restored);
            }
            let usage = completion.get("usage").cloned().unwrap_or(Value::Null);
            let prompt = usage["prompt_tokens"].as_u64().unwrap_or(0);
            let done = usage["completion_tokens"].as_u64().unwrap_or(0);
            metrics::observe_request(
                &client_id,
                &provider_name,
                &body.model,
                "ok",
                duration,
                prompt,
                done,
            );
            if let Some(metering) = &state.metering {
                metering.record(
                    &client_id,
                    &provider_name,
                    &body.model,
                    prompt as i64,
                    done as i64,
                );
            }
            if let Some(audit) = &state.audit {
                let response_text = completion["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                audit.write(
                    &client_id, &provider_name, &body.model, "ok",
                    prompt as i64, done as i64, &payload_json, &response_text,
                );
            }
            (StatusCode::OK, Json(completion)).into_response()
        }
        Err(err) => {
            metrics::observe_request(&client_id, &provider_name, &body.model, "upstream_error", duration, 0, 0);
            if let Some(audit) = &state.audit {
                audit.write(
                    &client_id, &provider_name, &body.model, "upstream_error",
                    0, 0, &payload_json, &err.to_string(),
                );
            }
            let message = match &err {
                providers::ProviderError::Http { status, body } => {
                    format!("provider returned {status}: {body}")
                }
                providers::ProviderError::Connection(m) => {
                    format!("provider connection failed: {m}")
                }
            };
            ApiError::upstream(message).into_response()
        }
    }
}
