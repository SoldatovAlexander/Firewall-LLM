//! Unified API error format (mirrors contracts/openapi.yaml).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub kind: &'static str,
    pub message: String,
    pub code: Option<&'static str>,
    pub details: Option<serde_json::Value>,
}

impl ApiError {
    pub fn auth(message: impl Into<String>, code: &'static str) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            kind: "authentication_error",
            message: message.into(),
            code: Some(code),
            details: None,
        }
    }

    pub fn blocked(message: impl Into<String>, reason: &str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            kind: "permission_error",
            message: message.into(),
            code: Some("blocked_by_inspector"),
            details: Some(json!({ "reason": reason })),
        }
    }

    pub fn rate_limited(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            kind: "rate_limit_error",
            message: message.into(),
            code: None,
            details: None,
        }
    }

    pub fn upstream(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            kind: "upstream_error",
            message: message.into(),
            code: None,
            details: None,
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            kind: "invalid_request_error",
            message: message.into(),
            code: Some("invalid_body"),
            details: None,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({
            "error": {
                "message": self.message,
                "type": self.kind,
            }
        });
        if let Some(code) = self.code {
            body["error"]["code"] = json!(code);
        }
        if let Some(details) = self.details {
            body["error"]["details"] = details;
        }
        (self.status, Json(body)).into_response()
    }
}
