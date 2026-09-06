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

// 0.1.1: ru_152 parity — patterns ported verbatim from LightAnon
// (py pin; see docs parity matrix). Order mirrors the Python profile:
// EMAIL, PHONE, PASSPORT, SNILS, INN, CARD, PERSON, ONLINE_ACCOUNT,
// PROFILE_URL, SOCIAL_HANDLE, USERNAME.
fn passport_re() -> &'static Regex { static C: OnceLock<Regex> = OnceLock::new(); C.get_or_init(|| Regex::new(r"(?i:\bпаспорт\s*(?:серия\s*)?\d{2}[\s\-]?\d{2}\s*(?:№|номер|n)?\s*\d{6}\b)|\b\d{2}[\s\-]?\d{2}[\s\-]+\d{6}\b").unwrap()) }
fn snils_re() -> &'static Regex { static C: OnceLock<Regex> = OnceLock::new(); C.get_or_init(|| Regex::new(r"\b\d{3}[\s\-]?\d{3}[\s\-]?\d{3}[\s\-]?\d{2}\b").unwrap()) }
fn inn_re() -> &'static Regex { static C: OnceLock<Regex> = OnceLock::new(); C.get_or_init(|| Regex::new(r"\b(?:\d{10}|\d{12})\b").unwrap()) }
fn person_re() -> &'static Regex { static C: OnceLock<Regex> = OnceLock::new(); C.get_or_init(|| Regex::new(r"\b(?:[А-ЯЁ][а-яё]+\s+[А-ЯЁ][а-яё]+(?:\s+[А-ЯЁ][а-яё]+)?|[А-ЯЁ][а-яё]+\s+[А-ЯЁ]\.\s*[А-ЯЁ]\.)").unwrap()) }
// NOTE: LightAnon's trailing `(?!\w)` is dropped — the regex crate has no
// look-around. Removal can only widen matches (greedy classes already end at
// word ends); for mask/block that errs toward safety, never toward leaks.
fn online_account_re() -> &'static Regex { static C: OnceLock<Regex> = OnceLock::new(); C.get_or_init(|| Regex::new(r"\b(?:ник(?:нейм)?|логин|аккаунт|профиль|пользователь)\s+@?[A-Za-z0-9][A-Za-z0-9_.-]{2,31}\s+(?:на|в)\s+(?:[A-Za-zА-Яа-яЁё0-9][A-Za-zА-Яа-яЁё0-9_.-]{1,63}(?:\.[A-Za-zА-Яа-яЁё]{2,})?)\b|\b(?:nickname|nick|login|account|profile|user(?:name)?)\s+@?[A-Za-z0-9][A-Za-z0-9_.-]{2,31}\s+(?:on|at)\s+(?:[A-Za-z0-9][A-Za-z0-9_.-]{1,63}(?:\.[A-Za-z]{2,})?)\b").unwrap()) }
fn profile_url_re() -> &'static Regex { static C: OnceLock<Regex> = OnceLock::new(); C.get_or_init(|| Regex::new(r"\b(?:https?://)?(?:t\.me|telegram\.me|vk\.com|vkontakte\.ru|github\.com|gitlab\.com|habr\.com|career\.habr\.com|linkedin\.com/in|facebook\.com|instagram\.com|x\.com|twitter\.com)/[A-Za-z0-9_.@#-]{3,64}\b").unwrap()) }
fn social_handle_re() -> &'static Regex { static C: OnceLock<Regex> = OnceLock::new(); C.get_or_init(|| Regex::new(r"@[A-Za-z0-9_][A-Za-z0-9_.-]{2,31}\b").unwrap()) }
// NOTE: LightAnon's leading `(?<![\w.%+-])` is dropped (no look-around in
// the regex crate). Safety comes from profile order: EMAIL runs first and
// tokenizes `ivan@mail.ru` before this rule sees it, so the `@` it can
// still match is genuinely handle-shaped. Residual risk is over-masking
// (e.g. `.@handle`), never an email leak.
fn username_re() -> &'static Regex { static C: OnceLock<Regex> = OnceLock::new(); C.get_or_init(|| Regex::new(r"\b(?:username|user|login|nick|nickname|логин|ник(?:нейм)?|пользователь)\s*[:=]\s*@?[A-Za-z0-9][A-Za-z0-9_.-]{2,31}\b").unwrap()) }

pub fn sanitize(text: &str, vault: &mut HashMap<String, String>, scope: &mut HashMap<String, usize>) -> String {
    // Matches are collected on the ORIGINAL text in profile order, then
    // applied back-to-front by span. This guarantees two properties the
    // naive sequential-pass approach breaks:
    // 1. No token-in-token corruption: later patterns never see earlier
    //    tokens' random hex (e.g. an INN `\d{10}` matching inside an EMAIL
    //    token and destroying it — a real flake caught by tests).
    // 2. Span-exact replacement (no `replacen` on duplicate values).
    // Overlapping hits resolve by profile order, then position.
    struct Hit {
        start: usize,
        end: usize,
        order: usize,
        typ: &'static str,
    }
    let patterns: [(&Regex, &'static str); 11] = [
        (email_re(), "EMAIL"),
        (phone_re(), "PHONE"),
        (passport_re(), "PASSPORT"),
        (snils_re(), "SNILS"),
        (inn_re(), "INN"),
        (card_re(), "CARD"),
        (person_re(), "PERSON"),
        (online_account_re(), "ONLINE_ACCOUNT"),
        (profile_url_re(), "PROFILE_URL"),
        (social_handle_re(), "SOCIAL_HANDLE"),
        (username_re(), "USERNAME"),
    ];
    let mut hits: Vec<Hit> = Vec::new();
    for (order, (re, typ)) in patterns.iter().enumerate() {
        for m in re.find_iter(text) {
            hits.push(Hit { start: m.start(), end: m.end(), order, typ });
        }
    }
    hits.sort_by_key(|h| (h.start, h.order));
    // Greedy non-overlapping accept first (profile order wins ties by sort).
    let mut accepted: Vec<Hit> = Vec::new();
    let mut last_end = 0;
    for h in hits {
        if h.start >= last_end {
            last_end = h.end;
            accepted.push(h);
        }
    }
    let mut out = text.to_string();
    // Back-to-front so byte spans stay valid while replacing.
    for h in accepted.iter().rev() {
        let val = out[h.start..h.end].to_string();
        // Reuse existing token for same value within this request
        let token = if let Some(existing) = vault.iter().find(|(_, v)| *v == &val).map(|(k, _)| k.clone()) {
            existing
        } else {
            let mut t;
            loop {
                t = format!("[{}_{:016x}{:016x}]", h.typ, rand::random::<u64>(), rand::random::<u64>());
                if !vault.contains_key(&t) {
                    break;
                }
            }
            vault.insert(t.clone(), val.clone());
            t
        };
        *scope.entry(token.clone()).or_insert(0) += 1;
        out.replace_range(h.start..h.end, &token);
    }
    out
}

/// Stateful restore session for one streamed response (R13).
///
/// Holds back a trailing partial token (`[EMAIL_ab12` without the closing
/// bracket yet) until the next chunk completes it; `flush()` emits whatever
/// is left so streamed text is never silently dropped.
pub struct StreamRestore {
    vault: HashMap<String, String>,
    scope: HashMap<String, usize>,
    policy: String,
    carry: String,
}

impl StreamRestore {
    pub fn new(
        vault: HashMap<String, String>,
        scope: HashMap<String, usize>,
        policy: &str,
    ) -> Self {
        Self { vault, scope, policy: policy.to_string(), carry: String::new() }
    }

    fn restore(&self, text: &str) -> String {
        deanonymize(text, &self.vault, &self.scope, &self.policy)
    }

    pub fn feed(&mut self, text: &str) -> String {
        let buf = format!("{}{}", self.carry, text);
        self.carry.clear();
        // Trailing partial token: "[" followed only by token chars, no "]".
        let cut = buf.rfind('[').filter(|&i| {
            !buf[i..].contains(']') && buf[i + 1..].chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
        let (head, carry) = match cut {
            Some(i) if i + 1 + 64 >= buf.len() => buf.split_at(i),
            Some(_) => {
                // Overlong bracketed run: not a token, emit as-is.
                (buf.as_str(), "")
            }
            None => (buf.as_str(), ""),
        };
        self.carry = carry.to_string();
        self.restore(head)
    }

    pub fn flush(&mut self) -> String {
        let tail = std::mem::take(&mut self.carry);
        if tail.is_empty() {
            return String::new();
        }
        self.restore(&tail)
    }
}

pub fn deanonymize(text: &str, vault: &HashMap<String, String>, scope: &HashMap<String, usize>, policy: &str) -> String {
    let mut out = text.to_string();
    for (token, val) in vault {
        if policy == "restore" && scope.contains_key(token) {
            out = out.replace(token, val);
        } else {
            // 0.1.1: generic mask label derived from the token type
            // ([SOCIAL_HANDLE_ab12..] -> [SOCIAL_HANDLE]); covers all
            // present and future detector types without per-type arms.
            // rsplit: multi-word types contain '_' themselves.
            let label = token
                .strip_prefix('[')
                .and_then(|t| t.strip_suffix(']'))
                .and_then(|t| t.rsplit_once('_'))
                .map(|(typ, _hex)| format!("[{typ}]"))
                .unwrap_or_else(|| "[REDACTED]".to_string());
            out = out.replace(token, &label);
        }
    }
    out
}
