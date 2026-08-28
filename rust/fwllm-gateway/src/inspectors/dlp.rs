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
        // Collect matches first to avoid borrowing issues
        let matches: Vec<String> = re.find_iter(&out.clone()).map(|m| m.as_str().to_string()).collect();
        let mut n = out.clone();
        for val in matches {
            // Reuse existing token for same value within this request
            let token = if let Some(existing) = vault.iter().find(|(_, v)| *v == &val).map(|(k, _)| k.clone()) {
                existing
            } else {
                let mut t;
                loop {
                    t = format!("[{}_{:016x}{:016x}]", typ, rand::random::<u64>(), rand::random::<u64>());
                    if !vault.contains_key(&t) {
                        break;
                    }
                }
                vault.insert(t.clone(), val.clone());
                t
            };
            *scope.entry(token.clone()).or_insert(0) += 1;
            n = n.replacen(&val, &token, 1);
        }
        out = n;
    }
    out
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
