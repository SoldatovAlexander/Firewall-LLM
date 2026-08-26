//! Policy engine: chain resolution, budget rules, attack failover.
//!
//! In-memory port of the Python PolicyEngine (MVP semantics).

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use fwllm_core::config::RoutingConfig;

const SEVERITY: [(&str, u8); 4] = [
    ("low", 0),
    ("medium", 1),
    ("high", 2),
    ("critical", 3),
];

fn severity_rank(name: &str) -> u8 {
    SEVERITY
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, r)| *r)
        .unwrap_or(0)
}

#[derive(Debug)]
pub struct RouterBlocked {
    pub message: String,
}

type Clock = fn() -> f64;

fn system_clock() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub struct PolicyEngine {
    routing: RoutingConfig,
    clock: Clock,
    override_provider: Option<String>,
    override_until: f64,
    blocked_sources: HashMap<String, f64>,
    attack_times: Vec<(String, f64)>,
    provider_tokens: HashMap<(String, String), i64>,
}

impl PolicyEngine {
    pub fn new(routing: RoutingConfig) -> Self {
        Self::with_clock(routing, system_clock)
    }

    pub fn with_clock(routing: RoutingConfig, clock: Clock) -> Self {
        Self {
            routing,
            clock,
            override_provider: None,
            override_until: 0.0,
            blocked_sources: HashMap::new(),
            attack_times: Vec::new(),
            provider_tokens: HashMap::new(),
        }
    }

    pub fn validate_routing(
        routing: &RoutingConfig,
        known_providers: &[String],
    ) -> Result<(), String> {
        for name in &routing.default_chain {
            if !known_providers.contains(name) {
                return Err(format!(
                    "routing.default_chain references unknown provider '{name}'"
                ));
            }
        }
        if let Some(switch_to) = &routing.attack_failover.switch_to {
            if !known_providers.contains(switch_to) {
                return Err(format!(
                    "attack_failover.switch_to references unknown provider '{switch_to}'"
                ));
            }
        }
        Ok(())
    }

    pub fn record_tokens(&mut self, provider: &str, total_tokens: i64, day: &str) {
        *self.provider_tokens.entry((provider.to_string(), day.to_string())).or_default() += total_tokens;
    }

    pub fn on_attack_detected(&mut self, severity: &str, client: &str) {
        let af = self.routing.attack_failover.clone();
        if !af.enabled || severity_rank(severity) < severity_rank(&af.min_severity) {
            return;
        }
        let now = (self.clock)();
        let window_start = now - af.window_seconds as f64;
        self.attack_times.retain(|(_, ts)| *ts >= window_start);
        self.attack_times.push((client.to_string(), now));
        if self.attack_times.len() < af.count {
            return;
        }
        self.attack_times.clear();
        if af.block_source && !client.is_empty() {
            self.blocked_sources
                .insert(client.to_string(), now + af.block_ttl_seconds as f64);
        }
        if let Some(target) = &af.switch_to {
            self.override_provider = Some(target.clone());
            self.override_until = now + af.cooldown_seconds as f64;
        }
    }

    fn provider_tokens_today(&self, provider: &str, day: &str) -> f64 {
        self.provider_tokens
            .get(&(provider.to_string(), day.to_string()))
            .copied()
            .unwrap_or(0) as f64
    }

    fn violates_rule(
        &self,
        provider: &str,
        rule: &fwllm_core::config::RoutingRule,
        day: &str,
    ) -> bool {
        if let Some(p) = &rule.when.provider {
            if p != provider {
                return false;
            }
        }
        if let Some(threshold) = &rule.when.provider_tokens_today {
            if !threshold.matches(self.provider_tokens_today(provider, day)) {
                return false;
            }
        }
        true
    }

    /// Resolve the serving provider and concrete model name.
    pub fn resolve(
        &mut self,
        requested_model: &str,
        client_id: &str,
    ) -> Result<(String, String), RouterBlocked> {
        let now = (self.clock)();
        if self
            .blocked_sources
            .get(client_id)
            .copied()
            .unwrap_or(0.0)
            > now
        {
            return Err(RouterBlocked {
                message: "request source is temporarily blocked".to_string(),
            });
        }

        let mut candidates: Vec<String> = if self.routing.default_chain.is_empty() {
            vec!["default".to_string()]
        } else {
            self.routing.default_chain.clone()
        };

        // apply / expire override
        if let Some(provider) = self.override_provider.clone() {
            if now < self.override_until {
                candidates.insert(0, provider.clone());
                candidates.dedup();
            } else {
                self.override_provider = None; // cooldown expired
            }
        }

        let day = day_string(now);
        let mapping = self.routing.model_mapping.get(requested_model).cloned();

        for candidate in &candidates {
            let violating = self
                .routing
                .rules
                .iter()
                .any(|rule| self.violates_rule(candidate, rule, &day));
            if !violating {
                let concrete = mapping
                    .as_ref()
                    .and_then(|m| m.get(candidate))
                    .cloned()
                    .unwrap_or_else(|| requested_model.to_string());
                return Ok((candidate.clone(), concrete));
            }
        }
        let head = candidates[0].clone();
        let concrete = mapping
            .as_ref()
            .and_then(|m| m.get(&head))
            .cloned()
            .unwrap_or_else(|| requested_model.to_string());
        Ok((head, concrete))
    }
}

fn day_string(unix_secs: f64) -> String {
    // UTC date as YYYYMMDD without pulling chrono into MVP
    let secs = unix_secs as i64;
    let days = secs.div_euclid(86_400);
    // civil-from-days algorithm (Howard Hinnant)
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
