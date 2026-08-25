//! Production entrypoint: FWLLM_CONFIG env var points to fwllm.yaml.

use std::path::PathBuf;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config_path = std::env::var("FWLLM_CONFIG").unwrap_or_else(|_| {
        eprintln!("FWLLM_CONFIG environment variable must point to a fwllm.yaml file");
        std::process::exit(1);
    });

    let config = match fwllm_core::config::load_config(&PathBuf::from(&config_path)) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("failed to load {config_path}: {err}");
            std::process::exit(1);
        }
    };
    let addr = format!("{}:{}", config.server.host, config.server.port);

    let app = fwllm_gateway::build_app(config, None);
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    tracing::info!("fwllm-gateway {addr} listening");
    axum::serve(listener, app).await.expect("server");
}
