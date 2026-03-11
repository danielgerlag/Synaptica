use clap::Parser;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tonic::transport::Server;
use tonic_web::GrpcWebLayer;
use tower_http::cors::{Any, CorsLayer};
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

    /// Path to UI static files directory (enables UI server on --ui-addr)
    #[arg(long)]
    ui_dir: Option<String>,

    /// Listen address for UI static file server
    #[arg(long, default_value = "0.0.0.0:8080")]
    ui_addr: String,
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

    // CORS layer for browser-based gRPC-Web clients
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_headers(Any)
        .allow_methods(Any)
        .expose_headers(Any);

    let mut builder = Server::builder()
        .accept_http1(true)
        .layer(cors)
        .layer(GrpcWebLayer::new());

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

    // Optionally start the static file UI server
    if let Some(ref ui_dir) = cli.ui_dir {
        let ui_addr = cli.ui_addr.clone();
        let ui_dir = ui_dir.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_static_files(&ui_addr, &ui_dir).await {
                tracing::error!(error = %e, "UI server failed");
            }
        });
        tracing::info!(addr = %cli.ui_addr, dir = %cli.ui_dir.as_deref().unwrap_or(""), "UI server started");
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

/// Minimal HTTP server for serving static UI files with SPA fallback.
async fn serve_static_files(addr: &str, dir: &str) -> anyhow::Result<()> {
    use std::path::PathBuf;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let base_dir = PathBuf::from(dir);
    let semaphore = Arc::new(Semaphore::new(200));
    loop {
        let (mut stream, _) = listener.accept().await?;
        let permit = match semaphore.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let base = base_dir.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let mut buf = [0u8; 4096];
            let n = match tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await {
                Ok(n) => n,
                Err(_) => return,
            };
            let request = String::from_utf8_lossy(&buf[..n]);
            // Extract path from "GET /path HTTP/1.1"
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");

            let file_path = if path == "/" {
                base.join("index.html")
            } else {
                let relative = path.trim_start_matches('/');
                let candidate = base.join(relative);
                // Prevent path traversal: canonicalize and verify it's under base
                match candidate.canonicalize() {
                    Ok(canonical) => {
                        let base_canonical = match base.canonicalize() {
                            Ok(b) => b,
                            Err(_) => return,
                        };
                        if !canonical.starts_with(&base_canonical) {
                            let header = "HTTP/1.1 403 Forbidden\r\nContent-Length: 9\r\n\r\nForbidden";
                            let _ = stream.write_all(header.as_bytes()).await;
                            return;
                        }
                        canonical
                    }
                    Err(_) => candidate, // File doesn't exist; will fall through to SPA fallback
                }
            };

            // Serve file or fall back to index.html for SPA routing
            let (body, content_type, status) = if file_path.is_file() {
                let ct = guess_content_type(&file_path);
                match tokio::fs::read(&file_path).await {
                    Ok(data) => (data, ct, "200 OK"),
                    Err(_) => (b"Internal Server Error".to_vec(), "text/plain", "500 Internal Server Error"),
                }
            } else {
                let index = base.join("index.html");
                match tokio::fs::read(&index).await {
                    Ok(data) => (data, "text/html", "200 OK"),
                    Err(_) => (b"Not Found".to_vec(), "text/plain", "404 Not Found"),
                }
            };

            let header = format!(
                "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n\r\n",
                status,
                content_type,
                body.len(),
            );
            let _ = stream.write_all(header.as_bytes()).await;
            let _ = stream.write_all(&body).await;
        });
    }
}

fn guess_content_type(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html",
        Some("js") | Some("mjs") => "application/javascript",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}
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
