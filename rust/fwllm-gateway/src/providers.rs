//! Provider adapters: OpenAI-compatible transport + registry.

use tokio::io::AsyncBufReadExt;
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
pub type StreamFuture =
    Pin<Box<dyn Future<Output = Result<Pin<Box<dyn Stream<Item = Result<Value, ProviderError>> + Send>>, ProviderError>> + Send>>;

use futures_util::Stream;

/// A single external LLM API adapter.
pub trait Provider: Send + Sync {
    fn chat(&self, payload: Value) -> ChatFuture;

    /// Open a streaming completion; returns the chunk stream.
    fn chat_stream(&self, _payload: Value) -> StreamFuture {
        Box::pin(async move { Err(ProviderError::Connection("streaming unsupported".into())) })
    }
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
        let req = self.build_request(payload, false);
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

    fn chat_stream(&self, payload: Value) -> StreamFuture {
        use futures_util::StreamExt;
        let req = self.build_request(payload, true);
        Box::pin(async move {
            let response = req.send().await.map_err(|e| match e.status() {
                Some(status) => ProviderError::Http {
                    status: status.as_u16(),
                    body: String::new(),
                },
                None => ProviderError::Connection(e.to_string()),
            })?;
            let status = response.status();
            if !status.is_success() {
                let body_text = response.text().await.unwrap_or_default();
                return Err(ProviderError::Http {
                    status: status.as_u16(),
                    body: body_text.chars().take(200).collect(),
                });
            }

            let reader = tokio_util::io::StreamReader::new(
                response
                    .bytes_stream()
                    .map(|r| r.map_err(|e| std::io::Error::other(e.to_string()))),
            );
            let lines = tokio::io::BufReader::new(reader).lines();

            let chunks = futures_util::stream::unfold(lines, |mut lines| async move {
                loop {
                    match lines.next_line().await {
                        Ok(Some(line)) => {
                            let line = line.trim();
                            if let Some(data) = line.strip_prefix("data: ") {
                                if data == "[DONE]" {
                                    return None;
                                }
                                if let Ok(v) = serde_json::from_str::<Value>(data) {
                                    return Some((Ok(v), lines));
                                }
                            }
                        }
                        Ok(None) => return None,
                        Err(e) => {
                            return Some((
                                Err(ProviderError::Connection(e.to_string())),
                                lines,
                            ))
                        }
                    }
                }
            });

            Ok(Box::pin(chunks)
                as Pin<Box<dyn futures_util::Stream<Item = Result<Value, ProviderError>> + Send>>)
        })
    }
}

impl OpenAiCompatProvider {
    fn build_request(&self, payload: Value, stream: bool) -> reqwest::RequestBuilder {
        let url = format!("{}/chat/completions", self.base_url);
        let mut body = payload;
        if stream {
            body["stream"] = Value::Bool(true);
        }
        let mut req = self
            .http
            .post(&url)
            .json(&body)
            .header("accept", if stream { "text/event-stream" } else { "application/json" });
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        for (name, value) in &self.extra_headers {
            req = req.header(name, value);
        }
        req
    }
}

pub type ProviderRegistry = HashMap<String, Arc<dyn Provider>>;
