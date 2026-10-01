//! Domain model for security findings.
//!
//! This module intentionally has no OpenTelemetry dependency. A finding can
//! feed routing, telemetry, audit, SIEM, or future runtime policy consumers.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityCategory {
    PromptInjection,
    Dlp,
}

impl SecurityCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PromptInjection => "prompt_injection",
            Self::Dlp => "dlp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityAction {
    Observe,
    Block,
    Mask,
}

impl SecurityAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Block => "block",
            Self::Mask => "mask",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityFinding {
    pub category: SecurityCategory,
    pub rule: String,
    pub severity: String,
    pub action: SecurityAction,
    pub client_id: Option<String>,
}

impl SecurityFinding {
    pub fn prompt_injection(
        rule: impl Into<String>,
        severity: impl Into<String>,
        action: SecurityAction,
        client_id: Option<&str>,
    ) -> Self {
        Self {
            category: SecurityCategory::PromptInjection,
            rule: rule.into(),
            severity: severity.into(),
            action,
            client_id: client_id.map(str::to_string),
        }
    }
}
