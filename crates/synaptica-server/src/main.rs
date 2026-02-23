use clap::Parser;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tonic::transport::Server;
use tracing_subscriber::EnvFilter;

use synaptica_server::auth::AuthInterceptor;
use synaptica_server::client_service::proto::synaptica_service_server::SynapticaServiceServer;
use synaptica_server::client_service::SynapticaServiceImpl;
use synaptica_server::config::ServerConfig;
use synaptica_server::metrics;
use synaptica_server::node::NodeRuntime;

#[derive(Parser, Debug)]
#[command(name = "synaptica", version, about = "Synaptica graph database server")]
struct Cli {
    /// Path to configuration file (TOML)
    #[arg(long)]
    config: Option<String>,

    /// Listen address (overrides config)
    #[arg(long)]
    listen: Option<String>,

    /// Data directory (overrides config)
    #[arg(long)]
    data_dir: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let mut config = match &cli.config {
        Some(path) => ServerConfig::from_file(path)?,
        None => ServerConfig::default(),
    };

    if let Some(addr) = cli.listen {
        config.listen_addr = addr;
    }
    if let Some(dir) = cli.data_dir {
        config.data_dir = dir;
    }

    // Initialize tracing with configured log level
    let filter = EnvFilter::from_default_env()
        .add_directive(format!("synaptica={}", config.log_level).parse()?);
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Register and optionally start Prometheus metrics endpoint
    metrics::register_metrics();
    if config.metrics_enabled {
        let metrics_addr = config.metrics_addr.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_metrics(&metrics_addr).await {
                tracing::error!(error = %e, "metrics server failed");
            }
        });
        tracing::info!(addr = %config.metrics_addr, "metrics endpoint started");
    }

    let node = NodeRuntime::start(&config)?;

    let svc = SynapticaServiceImpl {
        storage: node.storage.clone(),
        default_graph_id: node.default_graph_id,
    };

    let addr = config.listen_addr.parse()?;
    tracing::info!(%addr, "starting gRPC server");

    let interceptor = AuthInterceptor::new(config.auth.clone());
    let grpc_service = SynapticaServiceServer::with_interceptor(svc, interceptor);

    let mut builder = Server::builder();

    if let Some(tls_config) = &config.tls {
        let cert = std::fs::read(&tls_config.cert_path)?;
        let key = std::fs::read(&tls_config.key_path)?;
        let identity = tonic::transport::Identity::from_pem(cert, key);
        let mut tls = tonic::transport::ServerTlsConfig::new().identity(identity);
        if let Some(ca_path) = &tls_config.ca_cert_path {
            let ca_cert = std::fs::read(ca_path)?;
            let ca = tonic::transport::Certificate::from_pem(ca_cert);
            tls = tls.client_ca_root(ca);
        }
        builder = builder.tls_config(tls)?;
        tracing::info!("TLS enabled");
    }

    builder
        .add_service(grpc_service)
        .serve_with_shutdown(addr, async {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to listen for ctrl-c");
            tracing::info!("shutting down");
            node.shutdown();
        })
        .await?;

    Ok(())
}

/// Minimal HTTP server for Prometheus metrics scraping.
async fn serve_metrics(addr: &str) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let semaphore = Arc::new(Semaphore::new(100));
    loop {
        let (mut stream, _) = listener.accept().await?;
        let permit = match semaphore.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => continue,
        };
        tokio::spawn(async move {
            let _permit = permit;
            let mut buf = [0u8; 1024];
            let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
            let body = metrics::metrics_handler().await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body,
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}
