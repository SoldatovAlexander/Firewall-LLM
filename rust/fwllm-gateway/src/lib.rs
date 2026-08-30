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
    // Try admin first, then fall back to client self-audit
    let (client_id, is_admin) = match require_admin(&state, &headers).await {
        Ok(id) => (id, true),
        Err(_) => match require_client(&state, &headers).await {
            Ok(id) => (id, false),
            Err(err) => return err.into_response(),
        },
    };
    let Some(audit) = &state.audit else {
        return (StatusCode::OK, Json(json!({"total": 0, "records": []}))).into_response();
    };
    // Non-admin can only see own records, ignoring ?client= param
    let client_filter = if is_admin {
        params.get("client").map(String::as_str)
    } else {
        Some(client_id.as_str())
    };
    let records = audit.search(
        client_filter,
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
    if let Err(err) = require_admin(&state, &headers).await {
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
    if let Err(err) = require_admin(&state, &headers).await {
        return err.into_response();
    }
    let tokens = state.ingress.list_token_summaries().await;
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
        handle_ingress_socket(socket, agent_id, registry).await;
    })
}

async fn handle_ingress_socket(
    mut socket: axum::extract::ws::WebSocket,
    agent_id: String,
    registry: std::sync::Arc<crate::ingress::IngressRegistry>,
) {
    use axum::extract::ws::Message;
    use tokio::sync::mpsc;
    let (tx, mut rx) = mpsc::unbounded_channel();
    registry.register_tunnel(agent_id.clone(), tx).await;
    let mut pending: std::collections::HashMap<String, tokio::sync::oneshot::Sender<crate::ingress::ProxyResponse>> = std::collections::HashMap::new();
    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // Agent reply: {id, status, headers, body}
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                            if let Some(id) = val.get("id").and_then(|v| v.as_str()) {
                                if let Some(tx) = pending.remove(id) {
                                    let resp = crate::ingress::ProxyResponse {
                                        status: val.get("status").and_then(|v| v.as_u64()).unwrap_or(200) as u16,
                                        headers: val.get("headers").and_then(|v| v.as_object()).map(|m| m.iter().map(|(k,v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect()).unwrap_or_default(),
                                        body: val.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                    };
                                    let _ = tx.send(resp);
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(d))) => { let _ = socket.send(Message::Pong(d)).await; }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            req = rx.recv() => {
                match req {
                    Some(proxy_req) => {
                        let id = proxy_req.id.clone();
                        pending.insert(id.clone(), proxy_req.responder);
                        let frame = serde_json::json!({
                            "id": proxy_req.id,
                            "method": proxy_req.method,
                            "url": proxy_req.url,
                            "headers": proxy_req.headers,
                            "body": proxy_req.body,
                        });
                        if socket.send(Message::Text(frame.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
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
    state: &Arc<AppState>,
    client_id: &str,
    provider_name: &str,
    model: &str,
    provider: Arc<dyn providers::Provider>,
    payload: Value,
    started: Instant,
    chain_state: crate::inspectors::chain::ChainState,
) -> Response {
    let client_id = client_id.to_string();
    let provider_name = provider_name.to_string();
    let model = model.to_string();
    match provider.chat_stream(payload).await {
        Ok(stream) => {
            use futures_util::StreamExt;
            let client = client_id.clone();
            let provider_tag = provider_name.clone();
            let model_tag = model.clone();
            let state_clone = state.clone();
            let chain_state_clone = chain_state.clone();
            let last_usage = std::sync::Arc::new(std::sync::Mutex::new(None::<Value>));
            let last_usage_clone = last_usage.clone();
            let mapped = stream.map(move |item| {
                let mut chunk = match item {
                    Ok(c) => c,
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
                        return Ok::<Bytes, std::convert::Infallible>(Bytes::from(format!(
                            "data: {}\n\n",
                            json!({"error": {"message": err.to_string(), "type": "upstream_error"}})
                        )));
                    }
                };
                if chunk.get("usage").is_some() {
                    *last_usage_clone.lock().unwrap() = chunk.get("usage").cloned();
                }
                if let Some(delta) = chunk
                    .get_mut("choices")
                    .and_then(|c| c.get_mut(0))
                    .and_then(|c| c.get_mut("delta"))
                    .and_then(|d| d.get_mut("content"))
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string())
                {
                    let restored = state_clone
                        .inspectors
                        .process_response(&delta, &chain_state_clone);
                    if let Some(d) = chunk
                        .get_mut("choices")
                        .and_then(|c| c.get_mut(0))
                        .and_then(|c| c.get_mut("delta"))
                    {
                        d["content"] = json!(restored);
                    }
                }
                Ok::<Bytes, std::convert::Infallible>(Bytes::from(format!(
                    "data: {}\n\n",
                    chunk
                )))
            });
            let with_done = {
                let last_usage = last_usage.clone();
                let state = state.clone();
                let client_id = client_id.clone();
                let provider_name = provider_name.clone();
                let model = model.clone();
                let started = started;
                mapped.chain(futures_util::stream::once(async move {
                    let guard = last_usage.lock().unwrap();
                    let prompt = guard
                        .as_ref()
                        .and_then(|u| u.get("prompt_tokens"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let done = guard
                        .as_ref()
                        .and_then(|u| u.get("completion_tokens"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    drop(guard);
                    if prompt != 0 || done != 0 {
                        if let Some(metering) = &state.metering {
                            metering.record(&client_id, &provider_name, &model, prompt as i64, done as i64);
                        }
                    }
                    metrics::observe_request(
                        &client_id,
                        &provider_name,
                        &model,
                        "ok",
                        started.elapsed().as_secs_f64(),
                        prompt,
                        done,
                    );
                    Ok::<Bytes, std::convert::Infallible>(Bytes::from("data: [DONE]\n\n"))
                }))
            };
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

async fn require_admin(
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
    // Prefer dedicated admin tokens; fall back to clients only if no admin tokens configured (with warning)
    if !state.admin_clients.is_empty() {
        if let Some(label) = state.admin_clients.get(token) {
            return Ok(label.clone());
        }
        return Err(ApiError {
            status: axum::http::StatusCode::FORBIDDEN,
            kind: "permission_error",
            message: "admin privileges required".to_string(),
            code: Some("admin_required"),
            details: None,
        });
    }
    // Fallback: if no admin tokens configured, treat any valid client as admin but log warning
    if state.clients.contains_key(token) {
        tracing::warn!("admin endpoint accessed with client token; configure admin_clients for proper isolation");
        return Ok(state.clients.get(token).cloned().unwrap_or_default());
    }
    Err(ApiError::auth("invalid API key", "invalid_api_key"))
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

    // quota gate -> 429, or 503 when fail-closed and backend unreachable
    if let Some(metering) = &state.metering {
        match metering.check_client(&client_id) {
            Ok(()) => {}
            Err(crate::metering::MeteringError::QuotaExceeded { scope, limit }) => {
                metrics::observe_request(&client_id, &provider_name, &body.model, "rate_limited", 0.0, 0, 0);
                return ApiError::rate_limited(format!("daily {scope} quota exceeded (limit={limit})"))
                    .into_response();
            }
            Err(crate::metering::MeteringError::BackendUnavailable(msg)) => {
                if metering.backend_fail_closed() {
                    metrics::observe_request(&client_id, &provider_name, &body.model, "backend_error", 0.0, 0, 0);
                    return ApiError {
                        status: axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        kind: "rate_limit_error",
                        message: format!("metering backend unavailable: {msg}"),
                        code: Some("backend_unavailable"),
                        details: None,
                    }.into_response();
                }
                // fail-open: ignore backend errors
            }
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

    // Common payload construction and inspection for both streaming and non-streaming
    let mut payload = json!({
        "model": concrete_model,
        "messages": body.messages.iter()
            .map(|m| json!({"role": m.role, "content": m.content}))
            .collect::<Vec<_>>(),
        "stream": body.stream,
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

    if body.stream {
        return stream_response(&state, &client_id, &provider_name, &body.model, provider, payload, started, chain_state)
            .await;
    }

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
