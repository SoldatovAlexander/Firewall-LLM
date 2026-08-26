//! Metering: daily token/request counters, quotas (429), fail-open.

use redis::Commands;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
#[error("quota exceeded: {scope}")]
pub struct QuotaExceeded {
    pub scope: &'static str,
    pub limit: i64,
}

/// Storage abstraction so tests can run without Redis.
pub trait MeteringStore: Send + Sync {
    fn incr(&self, key: &str, amount: i64) -> i64;
    fn get(&self, key: &str) -> i64;
}

pub struct InMemoryStore {
    counters: std::sync::Mutex<HashMap<String, i64>>,
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self { counters: std::sync::Mutex::new(HashMap::new()) }
    }
}

impl MeteringStore for InMemoryStore {
    fn incr(&self, key: &str, amount: i64) -> i64 {
        let mut map = self.counters.lock().unwrap();
        let entry = map.entry(key.to_string()).or_default();
        *entry += amount;
        *entry
    }
    fn get(&self, key: &str) -> i64 {
        self.counters.lock().unwrap().get(key).copied().unwrap_or(0)
    }
}

pub struct RedisStore {
    client: redis::Client,
}

impl RedisStore {
    pub fn new(url: &str) -> Result<Self, redis::RedisError> {
        Ok(Self { client: redis::Client::open(url)? })
    }
}

impl MeteringStore for RedisStore {
    fn incr(&self, key: &str, amount: i64) -> i64 {
        let mut conn = match self.client.get_connection() {
            Ok(c) => c,
            Err(_) => return 0, // fail-open mirrors Python behavior
        };
        conn.incr(key, amount).unwrap_or(0)
    }
    fn get(&self, key: &str) -> i64 {
        let mut conn = match self.client.get_connection() {
            Ok(c) => c,
            Err(_) => return 0,
        };
        conn.get(key).unwrap_or(0)
    }
}

pub struct Metering {
    store: Box<dyn MeteringStore>,
    client_tokens_per_day: Option<i64>,
    client_requests_per_day: Option<i64>,
}

impl Metering {
    pub fn new(store: Box<dyn MeteringStore>, quotas: &fwllm_core::config::Quotas) -> Self {
        Self {
            store,
            client_tokens_per_day: quotas.client_tokens_per_day,
            client_requests_per_day: quotas.client_requests_per_day,
        }
    }

    fn day(&self) -> String {
        day_string(now_ts())
    }

    /// Check daily quotas; Err -> the gateway must answer 429.
    pub fn check_client(&self, client_id: &str) -> Result<(), QuotaExceeded> {
        let day = self.day();
        if let Some(limit) = self.client_tokens_per_day {
            let used = self.store.get(&format!("fwllm:c:tokens:{client_id}:{day}"));
            if used >= limit {
                return Err(QuotaExceeded { scope: "tokens", limit });
            }
        }
        if let Some(limit) = self.client_requests_per_day {
            let used = self.store.get(&format!("fwllm:c:req:{client_id}:{day}"));
            if used >= limit {
                return Err(QuotaExceeded { scope: "requests", limit });
            }
        }
        Ok(())
    }

    pub fn record(
        &self,
        client_id: &str,
        provider: &str,
        model: &str,
        prompt: i64,
        completion: i64,
    ) {
        let day = self.day();
        let total = prompt + completion;
        self.store
            .incr(&format!("fwllm:c:tokens:{client_id}:{day}"), total);
        self.store.incr(&format!("fwllm:c:req:{client_id}:{day}"), 1);
        self.store
            .incr(&format!("fwllm:p:tokens:{provider}:{day}"), total);
        self.store.incr(&format!("fwllm:p:req:{provider}:{day}"), 1);
        self.store
            .incr(&format!("fwllm:m:tokens:{model}:{day}"), total);
    }
}

fn now_ts() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn day_string(unix_secs: f64) -> String {
    let secs = unix_secs as i64;
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}{m:02}{d:02}")
}
