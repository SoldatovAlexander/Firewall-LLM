//! Shared application state.

use crate::providers::ProviderRegistry;
use fwllm_core::config::Config;
use std::collections::BTreeMap;
use std::time::Duration;
use std::sync::Arc;

pub struct AppState {
    pub config: Config,
    pub clients: BTreeMap<String, String>,
    pub providers: Arc<ProviderRegistry>,
    pub router: tokio::sync::Mutex<crate::router::PolicyEngine>,
    /// None when redis is unreachable at startup (fail-open accounting).
    pub metering: Option<crate::metering::Metering>,
    pub audit: Option<Arc<crate::audit::AuditLog>>,
}

impl AppState {
    pub fn new(config: Config, providers: Option<Arc<ProviderRegistry>>) -> Arc<Self> {
        Self::build(config, providers, None, None)
    }

    pub fn new_with_metering(
        config: Config,
        providers: Option<Arc<ProviderRegistry>>,
        metering_override: Option<crate::metering::Metering>,
    ) -> Arc<Self> {
        Self::build(config, providers, metering_override, None)
    }

    pub fn build(
        config: Config,
        providers: Option<Arc<ProviderRegistry>>,
        metering_override: Option<crate::metering::Metering>,
        audit: Option<Arc<crate::audit::AuditLog>>,
    ) -> Arc<Self> {
        let registry = providers.unwrap_or_else(|| {
            let mut map = ProviderRegistry::new();
            let timeout =
                Duration::from_secs_f64(config.server.request_timeout_seconds);
            for (name, pcfg) in &config.providers {
                let provider = match pcfg.provider_type.as_str() {
                    "openrouter" => crate::providers::OpenAiCompatProvider::new(
                        pcfg.base_url.clone(),
                        pcfg.api_key.clone(),
                        timeout,
                        vec![
                            (
                                "HTTP-Referer".to_string(),
                                "https://github.com/SoldatovAlexander/Firewall-LLM"
                                    .to_string(),
                            ),
                            ("X-Title".to_string(), "Firewall LLM".to_string()),
                        ],
                    ),
                    _ => crate::providers::OpenAiCompatProvider::new(
                        pcfg.base_url.clone(),
                        pcfg.api_key.clone(),
                        timeout,
                        Vec::new(),
                    ),
                };
                map.insert(name.clone(), Arc::new(provider));
            }
            Arc::new(map)
        });

        let mut routing = config.routing.clone();
        if routing.default_chain.is_empty() {
            routing.default_chain = config.providers.keys().cloned().collect();
        }
        if let Err(msg) = crate::router::PolicyEngine::validate_routing(
            &routing,
            &config.providers.keys().cloned().collect::<Vec<_>>(),
        ) {
            panic!("{msg}");
        }

        let metering = metering_override.or_else(|| {
            crate::metering::RedisStore::new(&config.redis_url).ok().map(|store| {
                crate::metering::Metering::new(Box::new(store), &config.quotas)
            })
        });

        Arc::new(Self {
            clients: config.clients.clone(),
            config,
            providers: registry,
            router: tokio::sync::Mutex::new(crate::router::PolicyEngine::new(routing)),
            metering,
            audit,
        })
    }
}
