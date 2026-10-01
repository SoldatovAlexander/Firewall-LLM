//! Security telemetry adapter.
//!
//! The domain SecurityFinding can contain routing context such as client_id.
//! This adapter deliberately exports only an explicit metadata allowlist.

use crate::inspectors::finding::{SecurityCategory, SecurityFinding};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityEventAttributes {
    pub category: String,
    pub rule: String,
    pub severity: String,
    pub action: String,
}

impl From<&SecurityFinding> for SecurityEventAttributes {
    fn from(finding: &SecurityFinding) -> Self {
        Self {
            category: finding.category.as_str().to_string(),
            rule: finding.rule.clone(),
            severity: finding.severity.clone(),
            action: finding.action.as_str().to_string(),
        }
    }
}

pub fn record_security_finding(finding: &SecurityFinding) {
    let attrs = SecurityEventAttributes::from(finding);
    let event_name = match finding.category {
        SecurityCategory::PromptInjection => "prompt_injection.detected",
        SecurityCategory::Dlp => "dlp.detected",
    };

    tracing::warn!(
        target: "fwllm.security",
        "fwllm.event.name" = event_name,
        "security.category" = attrs.category.as_str(),
        "security.rule" = attrs.rule.as_str(),
        "security.severity" = attrs.severity.as_str(),
        "security.action" = attrs.action.as_str(),
        "security finding"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspectors::finding::{SecurityAction, SecurityFinding};

    #[test]
    fn security_event_attributes_are_allowlisted() {
        let finding = SecurityFinding::prompt_injection(
            "override_instructions",
            "critical",
            SecurityAction::Block,
            Some("sensitive-client-id"),
        );

        let attrs = SecurityEventAttributes::from(&finding);

        assert_eq!(attrs.category, "prompt_injection");
        assert_eq!(attrs.rule, "override_instructions");
        assert_eq!(attrs.severity, "critical");
        assert_eq!(attrs.action, "block");
    }
}
