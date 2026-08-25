//! fwllm-gateway: production Rust gateway (axum).

pub mod error;
pub mod metrics;
pub mod providers;
pub mod router;
pub mod state;

use crate::error::ApiError;
use crate::state::AppState;
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
    let state = AppState::new(config, providers);
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/metrics", get(metrics_handler))
        .with_state(state)
}

async fn metrics_handler() -> String {
    metrics::render_metrics()
}

async fn healthz() -> Json<Value> {
    Json(json!({"status": "ok"}))
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
    let code_for =
        |code: &'static str| (client_id.clone(), provider_name.clone(), body.model.clone(), code);

    let provider = match state.providers.get(&provider_name) {
        Some(p) => p.clone(),
        None => {
            let c = code_for("upstream_error");
            metrics::observe_request(&c.0, &c.1, &c.2, c.3, 0.0, 0, 0);
            return ApiError::upstream(format!(
                "routed provider '{provider_name}' not configured"
            ))
            .into_response();
        }
    };

    if body.stream {
        return ApiError::invalid_request(
            "streaming is not yet supported by the Rust gateway",
        )
        .into_response();
    }

    let mut payload = json!({
        "model": concrete_model,
        "messages": body.messages.iter()
            .map(|m| json!({"role": m.role, "content": m.content}))
            .collect::<Vec<_>>(),
        "stream": false,
    });

    let result = provider.chat(payload.take()).await;
    let duration = started.elapsed().as_secs_f64();

    match result {
        Ok(mut completion) => {
            if concrete_model != body.model {
                completion["routed_from"] = json!(body.model);
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
            (StatusCode::OK, Json(completion)).into_response()
        }
        Err(err) => {
            metrics::observe_request(&client_id, &provider_name, &body.model, "upstream_error", duration, 0, 0);
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
