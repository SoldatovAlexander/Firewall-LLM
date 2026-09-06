//! Production entrypoint: FWLLM_CONFIG env var points to fwllm.yaml.
//! Starts main API on :8080 and ingress TLS listener on :8443 (self-signed).

use std::path::{Path, PathBuf};

fn ensure_self_signed_certs(dir: &Path, sans: &[String]) -> anyhow::Result<(PathBuf, PathBuf)> {
    use std::os::unix::fs::OpenOptionsExt;

    let cert_path = dir.join("server.crt");
    let key_path = dir.join("server.key");
    let ca_path = dir.join("ca.crt");
    if cert_path.exists() && key_path.exists() && ca_path.exists() {
        return Ok((cert_path, key_path));
    }
    std::fs::create_dir_all(dir)?;
    // R10: SANs come from config, no hardcoded addresses. Dev-only self-signed.
    let cert = rcgen::generate_simple_self_signed(sans.to_vec())?;
    let cert_pem = cert.cert.pem();
    let key_pem = cert.key_pair.serialize_pem();
    // For self-signed, ca.crt == server.crt
    std::fs::write(&cert_path, &cert_pem)?;
    // R10: private key with 0600, never world-readable
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true).mode(0o600);
    use std::io::Write;
    opts.open(&key_path)?.write_all(key_pem.as_bytes())?;
    std::fs::write(&ca_path, &cert_pem)?;
    tracing::info!("generated self-signed dev certs in {}", dir.display());
    Ok((cert_path, key_path))
}

#[tokio::main]
async fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();
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
    // R10: ingress listener is driven by config, not env; disabled by default flag.
    let ingress_cfg = config.ingress.clone();
    let certs_dir = std::env::var("FWLLM_CERTS_DIR").unwrap_or_else(|_| "./certs".to_string());

    let (app, state) = fwllm_gateway::build_app_full(config, None, None);

    if ingress_cfg.enabled {
        // Fail fast: a mandatory listener that cannot start must fail startup,
        // so readiness reflects the error instead of silently serving half (R10).
        let (cert_path, key_path) =
            ensure_self_signed_certs(Path::new(&certs_dir), &ingress_cfg.sans)
                .unwrap_or_else(|e| {
                    eprintln!("ingress cert setup failed: {e}");
                    std::process::exit(1);
                });
        let tls_config =
            axum_server::tls_rustls::RustlsConfig::from_pem_file(cert_path, key_path)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("ingress TLS config failed: {e}");
                    std::process::exit(1);
                });
        let ingress_addr: std::net::SocketAddr = ingress_cfg.listen.parse().unwrap_or_else(|e| {
            eprintln!("invalid ingress.listen '{}': {e}", ingress_cfg.listen);
            std::process::exit(1);
        });
        // R10: agent port serves ONLY /ingress — no chat/admin/metrics here.
        let ingress_app = fwllm_gateway::build_ingress_router(state);
        tracing::info!("fwllm ingress TLS listening on {ingress_addr}");
        tokio::spawn(async move {
            if let Err(e) = axum_server::bind_rustls(ingress_addr, tls_config)
                .serve(ingress_app.into_make_service())
                .await
            {
                eprintln!("ingress server error: {e}");
                std::process::exit(1);
            }
        });
    } else {
        tracing::info!("ingress listener disabled by config");
    }

    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    tracing::info!("fwllm-gateway {addr} listening");
    axum::serve(listener, app).await.expect("server");
}
