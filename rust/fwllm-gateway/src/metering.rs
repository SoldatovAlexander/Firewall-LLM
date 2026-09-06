//! Metering: daily token/request counters, quotas (429), fail-open.
//!
//! 0.1.1 async backends: the Redis store is fully async (multiplexed
//! connection, per-op deadlines) so slow backends never stall the request
//! loop. SQLite audit stays synchronous by design (sub-ms WAL writes).

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Per-op deadline: a hung backend fails fast instead of stalling loop tasks.
const REDIS_OP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum MeteringError {
    #[error("daily {scope} quota exceeded (limit={limit})")]
    QuotaExceeded { scope: &'static str, limit: i64 },
    #[error("metering backend unavailable: {0}")]
    BackendUnavailable(String),
}

// Keep backwards-compatible alias for existing code
#[derive(Debug, thiserror::Error)]
#[error("quota exceeded: {scope}")]
pub struct QuotaExceeded {
    pub scope: &'static str,
    pub limit: i64,
}

/// Which budget an atomic admission was denied by (R05).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReserveDenied {
    ClientTokens,
    ClientRequests,
    ProviderTokens,
}

#[derive(Debug)]
pub enum ReserveError {
    Denied(ReserveDenied),
    Backend(String),
}

/// Atomic admission request (R05): admit iff the client request counter is
/// below its limit and both limited token buckets fit the reserve; then
/// apply all increments plus the open reservation record in one step.
/// A limit < 0 means "count without gating".
pub struct AdmitOp {
    pub c_tokens_key: String,
    pub c_tokens_limit: i64,
    pub c_req_key: String,
    pub c_req_limit: i64,
    pub p_tokens_key: String,
    pub p_tokens_limit: i64,
    pub p_req_key: String,
    pub m_tokens_key: String,
    pub reserve: i64,
    pub reservation_key: String,
    pub reservation_ttl_secs: u64,
    pub bucket_ttl_secs: u64,
}

/// Idempotent settle request (R05): apply only for an open reservation.
pub struct SettleReq {
    pub reservation_key: String,
    pub token_keys: Vec<String>,
    pub delta: i64,
    pub bucket_ttl_secs: u64,
}

/// Storage abstraction so tests can run without Redis.
#[async_trait::async_trait]
pub trait MeteringStore: Send + Sync {
    async fn incr(&self, key: &str, amount: i64) -> Result<i64, String>;
    async fn get(&self, key: &str) -> Result<i64, String>;
    async fn ping(&self) -> Result<(), String>;
    async fn reserve(&self, req: &AdmitOp) -> Result<(), ReserveError>;
    /// Returns true when the settle was applied (false = already settled or
    /// expired reservation — a no-op by design).
    async fn settle(&self, req: &SettleReq) -> Result<bool, String>;
}

pub struct InMemoryStore {
    counters: std::sync::Mutex<HashMap<String, i64>>,
    settled: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self {
            counters: std::sync::Mutex::new(HashMap::new()),
            settled: std::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }
}

#[async_trait::async_trait]
impl MeteringStore for InMemoryStore {
    async fn incr(&self, key: &str, amount: i64) -> Result<i64, String> {
        let mut map = self.counters.lock().unwrap();
        let entry = map.entry(key.to_string()).or_default();
        *entry += amount;
        Ok(*entry)
    }
    async fn get(&self, key: &str) -> Result<i64, String> {
        Ok(self.counters.lock().unwrap().get(key).copied().unwrap_or(0))
    }
    async fn ping(&self) -> Result<(), String> {
        Ok(())
    }
    async fn reserve(&self, req: &AdmitOp) -> Result<(), ReserveError> {
        // Single lock section: atomic for one process (test backend).
        let mut map = self.counters.lock().unwrap();
        let get = |map: &HashMap<String, i64>, key: &str| {
            map.get(key).copied().unwrap_or(0)
        };
        if req.c_req_limit >= 0 && get(&map, &req.c_req_key) + 1 > req.c_req_limit {
            return Err(ReserveError::Denied(ReserveDenied::ClientRequests));
        }
        if req.c_tokens_limit >= 0
            && get(&map, &req.c_tokens_key) + req.reserve > req.c_tokens_limit
        {
            return Err(ReserveError::Denied(ReserveDenied::ClientTokens));
        }
        if req.p_tokens_limit >= 0
            && get(&map, &req.p_tokens_key) + req.reserve > req.p_tokens_limit
        {
            return Err(ReserveError::Denied(ReserveDenied::ProviderTokens));
        }
        *map.entry(req.c_tokens_key.clone()).or_default() += req.reserve;
        *map.entry(req.c_req_key.clone()).or_default() += 1;
        *map.entry(req.p_tokens_key.clone()).or_default() += req.reserve;
        *map.entry(req.p_req_key.clone()).or_default() += 1;
        *map.entry(req.m_tokens_key.clone()).or_default() += req.reserve;
        Ok(())
    }
    async fn settle(&self, req: &SettleReq) -> Result<bool, String> {
        let mut settled = self.settled.lock().unwrap();
        if !settled.insert(req.reservation_key.clone()) {
            return Ok(false);
        }
        drop(settled);
        let mut map = self.counters.lock().unwrap();
        for key in &req.token_keys {
            let v = map.get(key).copied().unwrap_or(0) + req.delta;
            map.insert(key.clone(), v.max(0));
        }
        Ok(true)
    }
}

pub struct RedisStore {
    client: redis::Client,
}

impl RedisStore {
    pub fn new(url: &str) -> Result<Self, redis::RedisError> {
        Ok(Self { client: redis::Client::open(url)? })
    }

    /// 0.1.1: async multiplexed connection per op with a deadline. A refused
    /// host fails fast (no retry loop); a hung backend trips the deadline.
    /// (Deliberately no cache: correctness first — pooling is a later
    /// optimization. The pre-async code also connected per op.)
    async fn conn(&self) -> Result<redis::aio::MultiplexedConnection, String> {
        with_deadline(self.client.get_multiplexed_async_connection(), "connect").await
    }
}

async fn with_deadline<T, E>(
    fut: impl std::future::Future<Output = Result<T, E>>,
    ctx: &str,
) -> Result<T, String>
where
    E: std::fmt::Display,
{
    tokio::time::timeout(REDIS_OP_TIMEOUT, fut)
        .await
        .map_err(|_| format!("redis op timeout ({ctx})"))?
        .map_err(|e| e.to_string())
}

#[async_trait::async_trait]
#[async_trait::async_trait]
impl MeteringStore for RedisStore {
    async fn incr(&self, key: &str, amount: i64) -> Result<i64, String> {
        use redis::AsyncCommands;
        let mut conn = self.conn().await?;
        with_deadline(conn.incr(key, amount), "incr").await
    }
    async fn get(&self, key: &str) -> Result<i64, String> {
        use redis::AsyncCommands;
        let mut conn = self.conn().await?;
        // Missing keys read as zero (daily buckets start empty).
        let val: Option<i64> =
            with_deadline(conn.get(key), "get").await?;
        Ok(val.unwrap_or(0))
    }
    async fn ping(&self) -> Result<(), String> {
        let mut conn = self.conn().await?;
        with_deadline(
            redis::cmd("PING").query_async::<String>(&mut conn),
            "ping",
        )
        .await?;
        Ok(())
    }
    async fn reserve(&self, req: &AdmitOp) -> Result<(), ReserveError> {
        // R05: one Lua script — check and reserve are atomic across
        // processes. Standalone/Sentinel only; Cluster rejects cross-slot
        // scripts (documented, same as Python).
        let mut conn = self.conn().await.map_err(ReserveError::Backend)?;
        let script = redis::Script::new(ADMIT_LUA);
        script
            .key(&req.c_tokens_key)
            .key(&req.c_req_key)
            .key(&req.p_tokens_key)
            .key(&req.p_req_key)
            .key(&req.m_tokens_key)
            .key(&req.reservation_key)
            .arg(req.c_tokens_limit)
            .arg(req.c_req_limit)
            .arg(req.p_tokens_limit)
            .arg(req.reserve)
            .arg(req.reservation_ttl_secs as i64)
            .arg(req.bucket_ttl_secs as i64);
        let invoke = script.invoke_async(&mut conn);
        let res: Vec<redis::Value> = with_deadline(invoke, "reserve")
            .await
            .map_err(ReserveError::Backend)?;
        match res.as_slice() {
            [redis::Value::BulkString(status), _] if status == b"ok" => Ok(()),
            [redis::Value::BulkString(scope), _] if scope == b"requests" => {
                Err(ReserveError::Denied(ReserveDenied::ClientRequests))
            }
            [redis::Value::BulkString(scope), _] if scope == b"provider_tokens" => {
                Err(ReserveError::Denied(ReserveDenied::ProviderTokens))
            }
            _ => Err(ReserveError::Denied(ReserveDenied::ClientTokens)),
        }
    }
    async fn settle(&self, req: &SettleReq) -> Result<bool, String> {
        let mut conn = self.conn().await?;
        let script = redis::Script::new(SETTLE_LUA);
        script
            .key(&req.reservation_key)
            .key(&req.token_keys[0])
            .key(&req.token_keys[1])
            .key(&req.token_keys[2])
            .arg(req.delta)
            .arg(req.bucket_ttl_secs as i64);
        let invoke = script.invoke_async(&mut conn);
        let applied: i64 = with_deadline(invoke, "settle").await?;
        Ok(applied == 1)
    }
}

/// R05 Lua scripts (same semantics as the Python side; see metering.py).
const ADMIT_LUA: &str = r#"
local ct = tonumber(redis.call('GET', KEYS[1]) or '0')
local cr = tonumber(redis.call('GET', KEYS[2]) or '0')
local pt = tonumber(redis.call('GET', KEYS[3]) or '0')
local reserve = tonumber(ARGV[4])
if tonumber(ARGV[1]) >= 0 and ct + reserve > tonumber(ARGV[1]) then
  return {'tokens', ct}
end
if tonumber(ARGV[2]) >= 0 and cr + 1 > tonumber(ARGV[2]) then
  return {'requests', cr}
end
if tonumber(ARGV[3]) >= 0 and pt + reserve > tonumber(ARGV[3]) then
  return {'provider_tokens', pt}
end
redis.call('INCRBY', KEYS[1], reserve)
redis.call('INCR', KEYS[2])
redis.call('INCRBY', KEYS[3], reserve)
redis.call('INCR', KEYS[4])
redis.call('INCRBY', KEYS[5], reserve)
for i = 1, 5 do redis.call('EXPIRE', KEYS[i], ARGV[6]) end
redis.call('SET', KEYS[6], 'open', 'EX', ARGV[5])
return {'ok', reserve}
"#;

const SETTLE_LUA: &str = r#"
if redis.call('GET', KEYS[1]) ~= 'open' then return 0 end
redis.call('SET', KEYS[1], 'settled', 'KEEPTTL')
local d = tonumber(ARGV[1])
for i = 2, 4 do
  local v = tonumber(redis.call('GET', KEYS[i]) or '0') + d
  if v < 0 then v = 0 end
  redis.call('SET', KEYS[i], v, 'EX', ARGV[2])
end
return 1
"#;

pub struct Metering {
    store: Box<dyn MeteringStore>,
    client_tokens_per_day: Option<i64>,
    client_requests_per_day: Option<i64>,
    provider_tokens_per_day: Option<i64>,
    backend_fail_closed: bool,
    completion_reserve_tokens: i64,
}

/// Open budget reservation from Metering::admit (R05).
#[derive(Debug, Clone)]
pub struct Reservation {
    pub id: String,
    pub client: String,
    pub provider: String,
    pub model: String,
    pub prompt_est: i64,
    pub completion_cap: i64,
}

impl Reservation {
    pub fn reserved_total(&self) -> i64 {
        self.prompt_est + self.completion_cap
    }
}

const RSV_TTL_SECS: u64 = 600;
const BUCKET_TTL_SECS: u64 = 60 * 60 * 48;

impl Metering {
    pub fn new(store: Box<dyn MeteringStore>, quotas: &fwllm_core::config::Quotas) -> Self {
        Self {
            store,
            client_tokens_per_day: quotas.client_tokens_per_day,
            client_requests_per_day: quotas.client_requests_per_day,
            provider_tokens_per_day: quotas.provider_tokens_per_day,
            backend_fail_closed: quotas.backend_fail_closed,
            completion_reserve_tokens: quotas.completion_reserve_tokens,
        }
    }

    /// When fail-closed, a backend error is returned to the caller instead of ignored.
    pub fn backend_fail_closed(&self) -> bool {
        self.backend_fail_closed
    }

    fn day(&self) -> String {
        day_string(now_ts())
    }

    /// R11: in fail-closed mode the backend is verified on every check,
    /// even when no numeric quotas are set — same semantics as Python.
    async fn ensure_ready(&self) -> Result<(), MeteringError> {
        if self.backend_fail_closed {
            self.store
                .ping()
                .await
                .map_err(MeteringError::BackendUnavailable)?;
        }
        Ok(())
    }

    /// Check daily quotas. Err(QuotaExceeded) -> 429; Err(BackendUnavailable) when fail-closed -> 503.
    pub async fn check_client(&self, client_id: &str) -> Result<(), MeteringError> {
        self.ensure_ready().await?;
        let day = self.day();
        if let Some(limit) = self.client_tokens_per_day {
            let used = match self.store.get(&format!("fwllm:c:tokens:{client_id}:{day}")).await {
                Ok(v) => v,
                Err(e) => {
                    if self.backend_fail_closed {
                        return Err(MeteringError::BackendUnavailable(e));
                    }
                    return Ok(());
                }
            };
            if used >= limit {
                return Err(MeteringError::QuotaExceeded { scope: "tokens", limit });
            }
        }
        if let Some(limit) = self.client_requests_per_day {
            let used = match self.store.get(&format!("fwllm:c:req:{client_id}:{day}")).await {
                Ok(v) => v,
                Err(e) => {
                    if self.backend_fail_closed {
                        return Err(MeteringError::BackendUnavailable(e));
                    }
                    return Ok(());
                }
            };
            if used >= limit {
                return Err(MeteringError::QuotaExceeded { scope: "requests", limit });
            }
        }
        Ok(())
    }

    pub async fn check_provider(&self, provider: &str) -> Result<(), MeteringError> {
        self.ensure_ready().await?;
        if let Some(limit) = self.provider_tokens_per_day {
            let day = self.day();
            let used = match self.store.get(&format!("fwllm:p:tokens:{provider}:{day}")).await {
                Ok(v) => v,
                Err(e) => {
                    if self.backend_fail_closed {
                        return Err(MeteringError::BackendUnavailable(e));
                    }
                    return Ok(());
                }
            };
            if used >= limit {
                return Err(MeteringError::QuotaExceeded {
                    scope: "provider_tokens",
                    limit,
                });
            }
        }
        Ok(())
    }

    pub async fn record(
        &self,
        client_id: &str,
        provider: &str,
        model: &str,
        prompt: i64,
        completion: i64,
    ) {
        let day = self.day();
        let total = prompt + completion;
        let _ = self.store.incr(&format!("fwllm:c:tokens:{client_id}:{day}"), total).await;
        let _ = self.store.incr(&format!("fwllm:c:req:{client_id}:{day}"), 1).await;
        let _ = self.store.incr(&format!("fwllm:p:tokens:{provider}:{day}"), total).await;
        let _ = self.store.incr(&format!("fwllm:p:req:{provider}:{day}"), 1).await;
        let _ = self.store.incr(&format!("fwllm:m:tokens:{model}:{day}"), total).await;
    }

    fn token_keys(&self, rsv: &Reservation) -> (String, String, String) {
        let day = self.day();
        (
            format!("fwllm:c:tokens:{0}:{day}", rsv.client),
            format!("fwllm:p:tokens:{0}:{day}", rsv.provider),
            format!("fwllm:m:tokens:{0}:{day}", rsv.model),
        )
    }

    /// Atomically check quotas and reserve budget (R05). The reserve is the
    /// prompt estimate plus the completion cap handed to the adapter.
    /// Denied admission consumes nothing.
    pub async fn admit(
        &self,
        client_id: &str,
        provider: &str,
        model: &str,
        prompt_est: i64,
        completion_cap: Option<i64>,
    ) -> Result<Reservation, MeteringError> {
        self.ensure_ready().await?;
        let day = self.day();
        let rsv = Reservation {
            id: format!(
                "{:016x}{:016x}",
                rand::random::<u64>(),
                rand::random::<u64>()
            ),
            client: client_id.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            prompt_est: prompt_est.max(0),
            completion_cap: completion_cap.unwrap_or(self.completion_reserve_tokens).max(0),
        };
        let op = AdmitOp {
            c_tokens_key: format!("fwllm:c:tokens:{client_id}:{day}"),
            c_tokens_limit: self.client_tokens_per_day.unwrap_or(-1),
            c_req_key: format!("fwllm:c:req:{client_id}:{day}"),
            c_req_limit: self.client_requests_per_day.unwrap_or(-1),
            p_tokens_key: format!("fwllm:p:tokens:{provider}:{day}"),
            p_tokens_limit: self.provider_tokens_per_day.unwrap_or(-1),
            p_req_key: format!("fwllm:p:req:{provider}:{day}"),
            m_tokens_key: format!("fwllm:m:tokens:{model}:{day}"),
            reserve: rsv.reserved_total(),
            reservation_key: format!("fwllm:rsv:{0}", rsv.id),
            reservation_ttl_secs: RSV_TTL_SECS,
            bucket_ttl_secs: BUCKET_TTL_SECS,
        };
        match self.store.reserve(&op).await {
            Ok(()) => Ok(rsv),
            Err(ReserveError::Denied(denied)) => {
                let (scope, limit) = match denied {
                    ReserveDenied::ClientTokens => {
                        ("tokens", self.client_tokens_per_day.unwrap_or(0))
                    }
                    ReserveDenied::ClientRequests => {
                        ("requests", self.client_requests_per_day.unwrap_or(0))
                    }
                    ReserveDenied::ProviderTokens => {
                        ("provider_tokens", self.provider_tokens_per_day.unwrap_or(0))
                    }
                };
                Err(MeteringError::QuotaExceeded { scope, limit })
            }
            // Backend errors surface like check_* errors: the caller maps
            // them to 503 fail-closed or skips fail-open.
            Err(ReserveError::Backend(msg)) => Err(MeteringError::BackendUnavailable(msg)),
        }
    }

    /// Reconcile a reservation with actual usage, exactly once (R05).
    /// Best-effort like record: backend errors are ignored.
    pub async fn settle(&self, rsv: &Reservation, prompt: i64, completion: i64) {
        let delta = (prompt + completion) - rsv.reserved_total();
        let (ctok, ptok, mtok) = self.token_keys(rsv);
        let _ = self.store.settle(&SettleReq {
            reservation_key: format!("fwllm:rsv:{0}", rsv.id),
            token_keys: vec![ctok, ptok, mtok],
            delta,
            bucket_ttl_secs: BUCKET_TTL_SECS,
        }).await;
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
