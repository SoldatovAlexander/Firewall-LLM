//! Shared application state.

use crate::providers::ProviderRegistry;
use fwllm_core::config::Config;
use std::collections::BTreeMap;
use std::time::Duration;
use std::sync::Arc;

pub struct AppState {
    pub inspectors: std::sync::Arc<crate::inspectors::chain::InspectorChain>,
    pub config: Config,
    pub clients: BTreeMap<String, String>,
    pub admin_clients: BTreeMap<String, String>,
    pub providers: Arc<ProviderRegistry>,
    pub router: std::sync::Arc<tokio::sync::Mutex<crate::router::PolicyEngine>>,
    /// None when redis is unreachable at startup (fail-open accounting).
    pub metering: Option<crate::metering::Metering>,
    pub audit: Option<Arc<crate::audit::AuditLog>>,
    pub ingress: Arc<crate::ingress::IngressRegistry>,
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
        let ingress = crate::ingress::shared_registry();
        let registry = providers.unwrap_or_else(|| {
            let mut map = ProviderRegistry::new();
            let timeout =
                Duration::from_secs_f64(config.server.request_timeout_seconds);
            for (name, pcfg) in &config.providers {
                let provider: Arc<dyn crate::providers::Provider> =
                    match pcfg.provider_type.as_str() {
                        "tunnel" => {
                            let agent_id = pcfg.agent_id.clone().unwrap_or_else(|| {
                                panic!("tunnel provider '{name}' requires agent_id")
                            });
                            Arc::new(crate::providers::TunnelProvider::new(
                                agent_id,
                                pcfg.base_url.clone(),
                                pcfg.api_key.clone(),
                                ingress.clone(),
                            ))
                        }
                        "openrouter" => Arc::new(crate::providers::OpenAiCompatProvider::new(
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
                        )),
                        _ => Arc::new(crate::providers::OpenAiCompatProvider::new(
                            pcfg.base_url.clone(),
                            pcfg.api_key.clone(),
                            timeout,
                            Vec::new(),
                        )),
                    };
                map.insert(name.clone(), provider);
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

        let router = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::router::PolicyEngine::new(routing),
        ));
        let mut inspectors_chain =
            crate::inspectors::chain::InspectorChain::from_config(&config.inspectors)
                .unwrap_or_else(|e| panic!("failed to build inspectors: {e}"));
        {
            let router_clone = router.clone();
            inspectors_chain.set_publish(move |severity, _rule, client_id| {
                let router = router_clone.clone();
                let severity = severity.clone();
                let client_id = client_id.clone();
                // Try to spawn on current runtime, otherwise block
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async move {
                        router.lock().await.on_attack_detected(&severity, &client_id);
                    });
                } else {
                    // For tests without runtime, use blocking
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();
                    rt.block_on(async {
                        router.lock().await.on_attack_detected(&severity, &client_id);
                    });
                }
            });
        }
        let inspectors = std::sync::Arc::new(inspectors_chain);

        let metering = metering_override.or_else(|| {
            crate::metering::RedisStore::new(&config.redis_url).ok().map(|store| {
                crate::metering::Metering::new(Box::new(store), &config.quotas)
            })
        });

        Arc::new(Self {
            inspectors,
            clients: config.clients.clone(),
            admin_clients: config.admin_clients.clone(),
            config,
            providers: registry,
            router,
            metering,
            audit,
            ingress,
        })
    }
}
