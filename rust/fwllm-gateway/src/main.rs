//! Production entrypoint: FWLLM_CONFIG env var points to fwllm.yaml.
//! Starts main API on :8080 and ingress TLS listener on :8443 (self-signed).

use std::path::{Path, PathBuf};

fn ensure_self_signed_certs(dir: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
    let cert_path = dir.join("server.crt");
    let key_path = dir.join("server.key");
    let ca_path = dir.join("ca.crt");
    if cert_path.exists() && key_path.exists() && ca_path.exists() {
        return Ok((cert_path, key_path));
    }
    std::fs::create_dir_all(dir)?;
    let cert = rcgen::generate_simple_self_signed(vec![
        "localhost".to_string(),
        "fwllm-gateway".to_string(),
        "192.168.88.101".to_string(),
    ])?;
    let cert_pem = cert.cert.pem();
    let key_pem = cert.key_pair.serialize_pem();
    // For self-signed, ca.crt == server.crt
    std::fs::write(&cert_path, &cert_pem)?;
    std::fs::write(&key_path, &key_pem)?;
    std::fs::write(&ca_path, &cert_pem)?;
    tracing::info!("generated self-signed certs in {}", dir.display());
    Ok((cert_path, key_path))
}

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
    let ingress_addr = std::env::var("FWLLM_INGRESS_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8443".to_string());
    let certs_dir = std::env::var("FWLLM_CERTS_DIR").unwrap_or_else(|_| "./certs".to_string());

    let app = fwllm_gateway::build_app(config, None);
    let ingress_app = app.clone();

    // Spawn ingress TLS listener on :8443
    let certs_dir_clone = certs_dir.clone();
    tokio::spawn(async move {
        match ensure_self_signed_certs(Path::new(&certs_dir_clone)) {
            Ok((cert_path, key_path)) => {
                let tls_config =
                    match axum_server::tls_rustls::RustlsConfig::from_pem_file(cert_path, key_path)
                        .await
                    {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::warn!("ingress TLS config failed: {e}");
                            return;
                        }
                    };
                tracing::info!("fwllm ingress TLS listening on {ingress_addr}");
                if let Err(e) = axum_server::bind_rustls(
                    ingress_addr.parse().unwrap(),
                    tls_config,
                )
                .serve(ingress_app.into_make_service())
                .await
                {
                    tracing::warn!("ingress server error: {e}");
                }
            }
            Err(e) => tracing::warn!("failed to ensure certs: {e}"),
        }
    });

    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    tracing::info!("fwllm-gateway {addr} listening (ingress wss on :8443)");
    axum::serve(listener, app).await.expect("server");
}
