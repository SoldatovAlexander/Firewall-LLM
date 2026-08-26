use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct DlpState {
    pub vault: HashMap<String, String>,
    pub scope: HashMap<String, usize>,
}

fn email_re() -> &'static Regex { static C: OnceLock<Regex> = OnceLock::new(); C.get_or_init(|| Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap()) }
fn phone_re() -> &'static Regex { static C: OnceLock<Regex> = OnceLock::new(); C.get_or_init(|| Regex::new(r"(?:\+7|8)[\s(-]?\d{3}[\s)-]?\d{3}[- ]?\d{2}[- ]?\d{2}").unwrap()) }
fn card_re() -> &'static Regex { static C: OnceLock<Regex> = OnceLock::new(); C.get_or_init(|| Regex::new(r"(?:\d[ -]?){13,19}").unwrap()) }

pub fn sanitize(text: &str, vault: &mut HashMap<String, String>, scope: &mut HashMap<String, usize>) -> String {
    let mut out = text.to_string();
    for (re, typ) in [(email_re(), "EMAIL"), (phone_re(), "PHONE"), (card_re(), "CARD")] {
        let mut n = out.clone();
        for m in re.find_iter(&out.clone()) {
            let val = m.as_str().to_string();
            let token = format!("[{}_{:08x}]", typ, fxhash(&val));
            vault.entry(token.clone()).or_insert(val);
            *scope.entry(token.clone()).or_insert(0) += 1;
            n = n.replacen(m.as_str(), &token, 1);
        }
        out = n;
    }
    out
}

fn fxhash(s: &str) -> u32 {
    let mut h: u32 = 0;
    for b in s.bytes() { h = h.wrapping_mul(31).wrapping_add(b as u32); }
    h
}

pub fn deanonymize(text: &str, vault: &HashMap<String, String>, scope: &HashMap<String, usize>, policy: &str) -> String {
    let mut out = text.to_string();
    for (token, val) in vault {
        if policy == "restore" && scope.contains_key(token) {
            out = out.replace(token, val);
        } else if token.starts_with("[EMAIL") {
            out = out.replace(token, "[EMAIL]");
        } else if token.starts_with("[PHONE") {
            out = out.replace(token, "[PHONE]");
        } else if token.starts_with("[CARD") {
            out = out.replace(token, "[CARD]");
        }
    }
    out
}
