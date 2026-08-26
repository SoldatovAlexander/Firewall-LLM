use regex::Regex;
use std::sync::OnceLock;

const SEVERITY_ORDER: &[(&str, u8)] = &[("low", 0), ("medium", 1), ("high", 2), ("critical", 3)];

fn rank(s: &str) -> u8 { SEVERITY_ORDER.iter().find(|(n,_)| *n==s).map(|(_,r)| *r).unwrap_or(0) }

struct Sig { name: &'static str, severity: &'static str, re: Regex }

fn sigs() -> &'static [Sig] {
    static CELL: OnceLock<Vec<Sig>> = OnceLock::new();
    CELL.get_or_init(|| vec![
        Sig { name: "override_instructions", severity: "critical", re: Regex::new(r"(?i)ignore\s+(all\s+)?(previous|prior|above|earlier)\s+instructions|disregard\s+(all\s+)?(previous|prior|above)|(?:reveal|print|show)\s+(your\s+)?(system\s+)?(prompt|instructions)").unwrap() },
        Sig { name: "jailbreak_persona", severity: "high", re: Regex::new(r"(?i)\bjailbreak\b|\bDAN\s+mode\b|developer\s+mode|you\s+are\s+now\s+(a|an|no\s+longer)").unwrap() },
        Sig { name: "roleplay_probe", severity: "medium", re: Regex::new(r"(?i)pretend\s+(you\s+are|to\s+be)|act\s+as\s+(if|a|an)\b|without\s+(any\s+)?restrictions").unwrap() },
    ])
}

pub fn scan(messages: &[serde_json::Value]) -> Vec<(&'static str, &'static str)> {
    let mut findings = Vec::new();
    for m in messages {
        if let Some(content) = m.get("content").and_then(|v| v.as_str()) {
            for sig in sigs() {
                if sig.re.is_match(content) {
                    findings.push((sig.name, sig.severity));
                }
            }
        }
    }
    findings
}

pub fn verdict(findings: &[(&'static str, &'static str)], block_gte: &str) -> Option<(&'static str, &'static str)> {
    if findings.is_empty() { return None; }
    let mut best = findings[0];
    for &f in findings.iter().skip(1) {
        if rank(f.1) > rank(best.1) { best = f; }
    }
    if rank(best.1) >= rank(block_gte) { Some(best) } else { None }
}
