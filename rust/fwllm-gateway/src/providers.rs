//! Provider adapters: OpenAI-compatible transport + registry.

use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("provider returned {status}: {body}")]
    Http { status: u16, body: String },
    #[error("provider connection failed: {0}")]
    Connection(String),
}

pub type ChatFuture = Pin<Box<dyn Future<Output = Result<Value, ProviderError>> + Send>>;

/// A single external LLM API adapter.
pub trait Provider: Send + Sync {
    fn chat(&self, payload: Value) -> ChatFuture;
}

pub struct OpenAiCompatProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    extra_headers: Vec<(String, String)>,
}

impl OpenAiCompatProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        timeout: Duration,
        extra_headers: Vec<(String, String)>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("reqwest client");
        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key,
            extra_headers,
        }
    }
}

impl Provider for OpenAiCompatProvider {
    fn chat(&self, payload: Value) -> ChatFuture {
        let url = format!("{}/chat/completions", self.base_url);
        let mut req = self.http.post(&url).json(&payload);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        for (name, value) in &self.extra_headers {
            req = req.header(name, value);
        }
        Box::pin(async move {
            let response = req.send().await.map_err(|e| match e.status() {
                Some(status) => ProviderError::Http {
                    status: status.as_u16(),
                    body: String::new(),
                },
                None => ProviderError::Connection(e.to_string()),
            })?;
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            if !status.is_success() {
                return Err(ProviderError::Http {
                    status: status.as_u16(),
                    body: body_text.chars().take(200).collect(),
                });
            }
            serde_json::from_str(&body_text)
                .map_err(|e| ProviderError::Connection(format!("non-JSON body: {e}")))
        })
    }
}

pub type ProviderRegistry = HashMap<String, Arc<dyn Provider>>;
