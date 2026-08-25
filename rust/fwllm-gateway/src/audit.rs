//! Audit log: append-only SQLite records with PII redaction.

use std::path::Path;
use std::sync::Mutex;

pub struct AuditLog {
    enabled: bool,
    dlp_redact: bool,
    conn: Mutex<rusqlite::Connection>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditRecord {
    pub ts: String,
    pub client: String,
    pub provider: String,
    pub model: String,
    pub code: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub messages: String,
    #[serde(rename = "response_text")]
    pub response: String,
}

impl AuditLog {
    pub fn open(config: &fwllm_core::config::AuditConfig) -> Result<Self, rusqlite::Error> {
        if let Some(parent) = Path::new(&config.db_path).parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        let conn = rusqlite::Connection::open(&config.db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts TEXT NOT NULL,
                client TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                code TEXT NOT NULL,
                prompt_tokens INTEGER NOT NULL DEFAULT 0,
                completion_tokens INTEGER NOT NULL DEFAULT 0,
                messages TEXT NOT NULL,
                response_text TEXT NOT NULL
            )",
            [],
        )?;
        Ok(Self {
            enabled: config.enabled,
            dlp_redact: config.dlp_redact,
            conn: Mutex::new(conn),
        })
    }

    /// PII redaction: emails, bank cards, RU phones.
    fn redact(&self, text: &str) -> String {
        if !self.dlp_redact {
            return text.to_string();
        }
        use std::sync::OnceLock;
        static EMAIL: OnceLock<regex::Regex> = OnceLock::new();
        static CARD: OnceLock<regex::Regex> = OnceLock::new();
        static PHONE: OnceLock<regex::Regex> = OnceLock::new();
        let email = EMAIL.get_or_init(|| {
            regex::Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap()
        });
        let card = CARD.get_or_init(|| regex::Regex::new(r"(?:\d[ -]?){13,19}").unwrap());
        let phone = PHONE.get_or_init(|| {
            regex::Regex::new(r"(?:\+7|8)[\s(-]?\d{3}[\s)-]?\d{3}[- ]?\d{2}[- ]?\d{2}")
                .unwrap()
        });
        let out = email.replace_all(text, "[EMAIL]");
        let out = card.replace_all(&out, "[CARD]");
        let out = phone.replace_all(&out, "[PHONE]");
        out.into_owned()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn write(
        &self,
        client: &str,
        provider: &str,
        model: &str,
        code: &str,
        prompt_tokens: i64,
        completion_tokens: i64,
        messages_json: &str,
        response_text: &str,
    ) {
        if !self.enabled {
            return;
        }
        let ts = iso_now();
        let messages = self.redact(messages_json);
        let response = self.redact(response_text);
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT INTO audit (ts, client, provider, model, code,
                    prompt_tokens, completion_tokens, messages, response_text)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![ts, client, provider, model, code,
                    prompt_tokens, completion_tokens, messages, response],
            );
        }
    }

    pub fn search(
        &self,
        client: Option<&str>,
        code: Option<&str>,
        limit: usize,
    ) -> Vec<AuditRecord> {
        let query = format!(
            "SELECT ts, client, provider, model, code, prompt_tokens,
                    completion_tokens, messages, response_text
             FROM audit{} ORDER BY id DESC LIMIT {}",
            match (client.is_some(), code.is_some()) {
                (true, true) => " WHERE client = ?1 AND code = ?2",
                (true, false) => " WHERE client = ?1",
                (false, true) => " WHERE code = ?1",
                (false, false) => "",
            },
            limit.clamp(1, 1000),
        );
        let Ok(conn) = self.conn.lock() else { return Vec::new() };
        let mut stmt = match conn.prepare(&query) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<AuditRecord> {
            Ok(AuditRecord {
                ts: row.get(0)?,
                client: row.get(1)?,
                provider: row.get(2)?,
                model: row.get(3)?,
                code: row.get(4)?,
                prompt_tokens: row.get(5)?,
                completion_tokens: row.get(6)?,
                messages: row.get(7)?,
                response: row.get(8)?,
            })
        };
        let rows = match (client, code) {
            (Some(c), Some(k)) => stmt.query_map(rusqlite::params![c, k], map_row),
            (Some(c), None) => stmt.query_map(rusqlite::params![c], map_row),
            (None, Some(k)) => stmt.query_map(rusqlite::params![k], map_row),
            (None, None) => stmt.query_map([], map_row),
        };
        match rows {
            Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }
}



fn iso_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let z = days as i64 + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    let rem = secs % 86_400;
    format!(
        "{year:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}
