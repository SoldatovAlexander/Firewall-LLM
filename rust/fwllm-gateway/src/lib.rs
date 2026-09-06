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
use std::pin::Pin;
use std::sync::{Arc, Mutex};
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
    build_app_full(config, providers, metering).0
}

/// Build main router plus the shared state, so embeddings (e.g. a separate
/// ingress listener) can serve additional routers from the same AppState.
pub fn build_app_full(
    config: Config,
    providers: Option<Arc<providers::ProviderRegistry>>,
    metering: Option<metering::Metering>,
) -> (Router, Arc<AppState>) {
    let audit = match audit::AuditLog::open(&config.audit) {
        Ok(log) => Some(Arc::new(log)),
        Err(e) => {
            tracing::warn!("audit disabled: {e}");
            None
        }
    };
    let state = AppState::build(config, providers, metering, audit);
    let router = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/admin/audit", get(admin_audit))
        .route("/admin/ingress/tokens", post(create_ingress_token))
        .route("/admin/ingress/agents", get(list_ingress_agents))
        .route("/ingress", get(ingress_ws_handler))
        .route("/metrics", get(metrics_handler))
        .with_state(state.clone());
    (router, state)
}

/// Agent-facing router for the :8443 listener: only `/ingress` exists here.
/// Chat, admin and metrics routes intentionally return 404 on this port (R10).
pub fn build_ingress_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/ingress", get(ingress_ws_handler))
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
                        if socket.send(Message::Text(frame.to_string())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }
}

async fn metrics_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    if let Err(err) = require_admin(&state, &headers).await {
        return err.into_response();
    }
    let body = metrics::render_metrics();
    ([(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")], body).into_response()
}

/// Heuristic token estimate (~4 chars per token) used when the provider
/// sends no usage object (R03). Explicitly marked, never silently zero.
fn estimate_usage(prompt_text: &str, completion_text: &str) -> (u64, u64) {
    // chars (not bytes) for parity with the Python estimator.
    (
        prompt_text.chars().count() as u64 / 4,
        completion_text.chars().count() as u64 / 4,
    )
}

/// Join request message contents for usage estimation (R03).
fn prompt_text(payload: &Value) -> String {
    payload
        .get("messages")
        .and_then(|v| v.as_array())
        .map(|messages| {
            messages
                .iter()
                .filter_map(|m| m.get("content")?.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// R03: exactly-once accounting for streamed responses.
///
/// The terminal `once` records on normal completion; the `Drop` impl covers
/// client disconnect (axum drops the body stream, so the `once` future may
/// never run). Both paths share one atomic flag, so usage is never double
/// counted (coordinates with R05 reserves).
struct StreamAccountant {
    state: Arc<AppState>,
    client_id: String,
    provider_name: String,
    model: String,
    request_text: String,
    completion_text: Arc<Mutex<String>>,
    last_usage: Arc<Mutex<Option<Value>>>,
    recorded: Arc<std::sync::atomic::AtomicBool>,
    reservation: Option<crate::metering::Reservation>,
}

impl StreamAccountant {
    /// Record metering exactly once; returns the (prompt, completion) pair.
    fn record_once(&self) -> (u64, u64) {
        if self
            .recorded
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return (0, 0);
        }
        let guard = self.last_usage.lock().unwrap();
        let (prompt, done) = match guard
            .as_ref()
            .and_then(|u| u.as_object())
            .filter(|o| !o.is_empty())
        {
            Some(usage) => (
                usage
                    .get("prompt_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                usage
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
            ),
            None => {
                let completion = self.completion_text.lock().unwrap().clone();
                estimate_usage(&self.request_text, &completion)
            }
        };
        drop(guard);
        // R05: reconcile the admission reserve; fall back to a plain record
        // when admission was skipped (fail-open without reservation).
        if let Some(metering) = &self.state.metering {
            if let Some(rsv) = &self.reservation {
                metering.settle(rsv, prompt as i64, done as i64);
            } else {
                metering.record(
                    &self.client_id,
                    &self.provider_name,
                    &self.model,
                    prompt as i64,
                    done as i64,
                );
            }
        }
        (prompt, done)
    }
}

impl Drop for StreamAccountant {
    fn drop(&mut self) {
        // Best-effort accounting on disconnect; a no-op after normal finish.
        let _ = self.record_once();
    }
}

/// Body stream wrapper that owns the accountant, so dropping the response
/// (client disconnect) still records usage via the `Drop` impl above.
struct AccountedStream {
    inner: Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, std::convert::Infallible>> + Send>>,
    _accountant: StreamAccountant,
}

impl futures_util::Stream for AccountedStream {
    type Item = Result<Bytes, std::convert::Infallible>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
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
    reservation: Option<crate::metering::Reservation>,
) -> Response {
    let client_id = client_id.to_string();
    let provider_name = provider_name.to_string();
    let model = model.to_string();
    // R03: request text for usage estimation when the provider sends none.
    let request_text = prompt_text(&payload);
    match provider.chat_stream(payload).await {
        Ok(stream) => {
            use futures_util::StreamExt;
            let client = client_id.clone();
            let provider_tag = provider_name.clone();
            let model_tag = model.clone();
            // R13: stateful restore session reassembles DLP tokens split
            // across SSE chunk boundaries.
            let restore_session = std::sync::Arc::new(std::sync::Mutex::new(
                state.inspectors.stream_restore_session(&chain_state),
            ));
            let restore_clone = restore_session.clone();
            let last_usage = std::sync::Arc::new(std::sync::Mutex::new(None::<Value>));
            let last_usage_clone = last_usage.clone();
            // R03: restored completion text feeds the estimate fallback.
            let completion_text = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
            let completion_clone = completion_text.clone();
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
                // R03: only a non-empty usage object counts as provider
                // usage; anything else falls back to estimation.
                if chunk
                    .get("usage")
                    .and_then(|u| u.as_object())
                    .map(|o| !o.is_empty())
                    .unwrap_or(false)
                {
                    *last_usage_clone.lock().unwrap() = chunk.get("usage").cloned();
                }
                // R04: restore every choice by index, never assume choices[0].
                if let Some(choices) = chunk
                    .get_mut("choices")
                    .and_then(|c| c.as_array_mut())
                {
                    for choice in choices.iter_mut() {
                        let delta = choice
                            .get_mut("delta")
                            .and_then(|d| d.get_mut("content"))
                            .and_then(|c| c.as_str())
                            .map(|s| s.to_string());
                        if let Some(delta) = delta {
                            let restored =
                                restore_clone.lock().unwrap().feed(&delta);
                            completion_clone.lock().unwrap().push_str(&restored);
                            if let Some(d) = choice.get_mut("delta") {
                                d["content"] = json!(restored);
                            }
                        }
                    }
                }
                Ok::<Bytes, std::convert::Infallible>(Bytes::from(format!(
                    "data: {}\n\n",
                    chunk
                )))
            });
            // R03: single accountant shared by the terminal chunk (normal
            // finish) and the Drop impl (client disconnect) — exactly once.
            let accountant = StreamAccountant {
                state: state.clone(),
                client_id: client_id.clone(),
                provider_name: provider_name.clone(),
                model: model.clone(),
                request_text,
                completion_text: completion_text.clone(),
                last_usage: last_usage.clone(),
                recorded: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                reservation: reservation.clone(),
            };
            let with_done = {
                let restore_done = restore_session.clone();
                let completion_done = completion_text.clone();
                // Flush held-back trailing text (R13) ahead of [DONE].
                let tail = mapped.chain(futures_util::stream::once(async move {
                    let flushed = restore_done.lock().unwrap().flush();
                    if flushed.is_empty() {
                        None
                    } else {
                        completion_done.lock().unwrap().push_str(&flushed);
                        Some(Ok::<Bytes, std::convert::Infallible>(Bytes::from(format!(
                            "data: {}\n\n",
                            json!({
                                "object": "chat.completion.chunk",
                                "choices": [{"index": 0, "delta": {"content": flushed}}],
                            })
                        ))))
                    }
                }).filter_map(|x| async move { x }));
                let accountant_done = StreamAccountant {
                    state: accountant.state.clone(),
                    client_id: accountant.client_id.clone(),
                    provider_name: accountant.provider_name.clone(),
                    model: accountant.model.clone(),
                    request_text: accountant.request_text.clone(),
                    completion_text: accountant.completion_text.clone(),
                    last_usage: accountant.last_usage.clone(),
                    recorded: accountant.recorded.clone(),
                    reservation: accountant.reservation.clone(),
                };
                tail.chain(futures_util::stream::once(async move {
                    // R03: always count the admitted request, even on zero
                    // usage; Drop covers the disconnect path with the flag.
                    let _guard = accountant_done;
                    let (prompt, done) = _guard.record_once();
                    // R06: feed served usage to routing (normal finish; the
                    // disconnect Drop path updates metering only — router
                    // counters are a best-effort mirror, metering is truth).
                    _guard
                        .state
                        .router
                        .lock()
                        .await
                        .record_tokens_today(&_guard.provider_name, prompt as i64 + done as i64);
                    metrics::observe_request(
                        &_guard.client_id,
                        &_guard.provider_name,
                        &_guard.model,
                        "ok",
                        started.elapsed().as_secs_f64(),
                        prompt,
                        done,
                    );
                    Ok::<Bytes, std::convert::Infallible>(Bytes::from("data: [DONE]\n\n"))
                }))
            };
            let body_stream = AccountedStream {
                inner: Box::pin(with_done),
                _accountant: accountant,
            };
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/event-stream")
                .body(Body::from_stream(body_stream))
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
    role: String,
    content: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    tool_calls: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    top_p: Option<f64>,
    #[serde(default)]
    max_tokens: Option<i64>,
    #[serde(default)]
    stop: Option<Value>,
    // Client-side per contract; accepted but never forwarded upstream.
    #[serde(default)]
    #[allow(dead_code)]
    metadata: Option<Value>,
    // R03: user-supplied stream options are preserved and forwarded upstream.
    #[serde(default)]
    stream_options: Option<Value>,
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
    // No fallback: an empty admin list means admin endpoints are disabled.
    // Self-audit stays available via require_client with mandatory ID filter.
    if let Some(label) = state.admin_clients.get(token) {
        return Ok(label.clone());
    }
    if state.clients.contains_key(token) {
        return Err(ApiError {
            status: axum::http::StatusCode::FORBIDDEN,
            kind: "permission_error",
            message: "admin privileges required".to_string(),
            code: Some("admin_required"),
            details: None,
        });
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
            Err(crate::router::RoutingError::Blocked(blocked)) => {
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
            Err(crate::router::RoutingError::BudgetExhausted(msg)) => {
                metrics::observe_request(&client_id, "unrouted", &body.model, "rate_limited", 0.0, 0, 0);
                return ApiError::rate_limited(msg).into_response();
            }
        }
    }

    // R07: contract validation — ranges per openapi.yaml, messages min 1.
    if body.messages.is_empty() {
        return ApiError::invalid_request("messages must contain at least 1 item")
            .into_response();
    }
    if let Some(t) = body.temperature {
        if !(0.0..=2.0).contains(&t) {
            return ApiError::invalid_request("temperature must be within 0..2")
                .into_response();
        }
    }
    if let Some(p) = body.top_p {
        if !(0.0..=1.0).contains(&p) {
            return ApiError::invalid_request("top_p must be within 0..1")
                .into_response();
        }
    }
    if let Some(m) = body.max_tokens {
        if m < 1 {
            return ApiError::invalid_request("max_tokens must be >= 1").into_response();
        }
    }
    if let Some(stop) = &body.stop {
        let ok = stop.is_string()
            || stop
                .as_array()
                .map(|a| a.iter().all(|v| v.is_string()))
                .unwrap_or(false);
        if !ok {
            return ApiError::invalid_request("stop must be a string or array of strings")
                .into_response();
        }
    }

    let started = Instant::now();

    // R05: atomic admission replaces separate check calls — the request and
    // its token budget (prompt estimate + completion cap handed to the
    // adapter) are reserved in one step. 429 on breach, 503 when
    // fail-closed and the backend is unreachable.
    let request_text: String = body
        .messages
        .iter()
        .filter_map(|m| m.content.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    let (prompt_est, _) = estimate_usage(&request_text, "");
    let reservation: Option<crate::metering::Reservation> =
        if let Some(metering) = &state.metering {
            match metering.admit(
                &client_id,
                &provider_name,
                &body.model,
                prompt_est as i64,
                body.max_tokens,
            ) {
                Ok(rsv) => Some(rsv),
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
                    None
                }
            }
        } else {
            None
        };

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

    // Common payload construction and inspection for both streaming and non-streaming.
    // Contract fields (temperature/top_p/max_tokens/stop/name/tool_calls) are
    // forwarded; metadata stays client-side and is never sent upstream.
    let mut payload = json!({
        "model": concrete_model,
        "messages": body.messages.iter()
            .map(|m| {
                let mut msg = serde_json::Map::new();
                msg.insert("role".to_string(), json!(m.role));
                msg.insert("content".to_string(), json!(m.content));
                if let Some(name) = &m.name {
                    msg.insert("name".to_string(), json!(name));
                }
                if let Some(tool_calls) = &m.tool_calls {
                    msg.insert("tool_calls".to_string(), tool_calls.clone());
                }
                Value::Object(msg)
            })
            .collect::<Vec<_>>(),
        "stream": body.stream,
    });
    if let Some(t) = body.temperature {
        payload["temperature"] = json!(t);
    }
    if let Some(p) = body.top_p {
        payload["top_p"] = json!(p);
    }
    if let Some(m) = body.max_tokens {
        payload["max_tokens"] = json!(m);
    }
    if let Some(stop) = &body.stop {
        payload["stop"] = stop.clone();
    }
    if let Some(options) = &body.stream_options {
        if options.is_object() {
            payload["stream_options"] = options.clone();
        }
    }
    if body.stream {
        // R03: ask supporting providers for a terminal usage chunk so
        // streaming responses can be accounted exactly. An explicit user
        // choice is respected; absence defaults to True.
        let mut options = payload
            .get("stream_options")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        options.entry("include_usage".to_string()).or_insert(json!(true));
        payload["stream_options"] = Value::Object(options);
    }

    let chain_state = match state
        .inspectors
        .process_request_with_client(&mut payload, Some(&client_id))
    {
        Ok(s) => s,
        Err(e) => {
            metrics::observe_request(&client_id, &provider_name, &body.model, "blocked", 0.0, 0, 0);
            if let Some(audit) = &state.audit {
                audit.write(&client_id, &provider_name, &body.model, "blocked", 0, 0, &serde_json::to_string(&payload).unwrap_or_default(), &e.message, "upstream");
            }
            return e.into_response();
        }
    };

    if body.stream {
        return stream_response(&state, &client_id, &provider_name, &body.model, provider, payload, started, chain_state, reservation)
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
            // R03: provider usage wins; otherwise estimate from the
            // exchanged text and mark the source explicitly.
            let (prompt, done, usage_source) =
                match completion.get("usage").and_then(|u| u.as_object()) {
                    Some(usage) if !usage.is_empty() => (
                        usage.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                        usage.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                        "upstream",
                    ),
                    _ => {
                        let response_text: String = completion
                            .get("choices")
                            .and_then(|c| c.as_array())
                            .map(|choices| {
                                choices
                                    .iter()
                                    .filter_map(|ch| {
                                        ch.get("message")?.get("content")?.as_str()
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                            .unwrap_or_default();
                        let (p, d) = estimate_usage(&prompt_text(&payload), &response_text);
                        (p, d, "estimated")
                    }
                };
            metrics::observe_request(
                &client_id,
                &provider_name,
                &body.model,
                "ok",
                duration,
                prompt,
                done,
            );
            // R05: reconcile the admission reserve with actual usage.
            if let Some(metering) = &state.metering {
                if let Some(rsv) = &reservation {
                    metering.settle(rsv, prompt as i64, done as i64);
                } else {
                    metering.record(
                        &client_id,
                        &provider_name,
                        &body.model,
                        prompt as i64,
                        done as i64,
                    );
                }
            }
            // R06: feed served usage to routing before the next admission,
            // so budget rules observe it.
            state
                .router
                .lock()
                .await
                .record_tokens_today(&provider_name, prompt as i64 + done as i64);
            if let Some(audit) = &state.audit {
                let response_text = completion["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                audit.write(
                    &client_id, &provider_name, &body.model, "ok",
                    prompt as i64, done as i64, &payload_json, &response_text,
                    usage_source,
                );
            }
            (StatusCode::OK, Json(completion)).into_response()
        }
        Err(err) => {
            // R05: input may have been spent; completion never happened.
            if let Some(metering) = &state.metering {
                if let Some(rsv) = &reservation {
                    metering.settle(rsv, prompt_est as i64, 0);
                }
            }
            metrics::observe_request(&client_id, &provider_name, &body.model, "upstream_error", duration, 0, 0);
            if let Some(audit) = &state.audit {
                audit.write(
                    &client_id, &provider_name, &body.model, "upstream_error",
                    0, 0, &payload_json, &err.to_string(), "upstream",
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
