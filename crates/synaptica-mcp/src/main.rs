use std::sync::Arc;

use clap::Parser;
use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

use synaptica_mcp::backend::SynapticaBackend;
use synaptica_mcp::local_backend::LocalBackend;
use synaptica_mcp::remote_backend::RemoteBackend;
use synaptica_mcp::server::SynapticaMcpServer;

#[derive(Parser, Debug)]
#[command(
    name = "synaptica-mcp",
    version,
    about = "MCP server for Synaptica graph database"
)]
struct Cli {
    /// Mode: "stdio" for local embedded database, "remote" for connecting to a server
    #[arg(long, default_value = "stdio")]
    mode: String,

    /// Data directory for local mode (RocksDB storage)
    #[arg(long, default_value = "./data")]
    data_dir: String,

    /// Server URL for remote mode
    #[arg(long, default_value = "http://localhost:9090")]
    server_url: String,

    /// Default graph name
    #[arg(long, default_value = "default")]
    graph: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // MCP stdio servers must not write logs to stdout/stderr (they interfere with the protocol).
    // Only enable tracing if RUST_LOG is explicitly set.
    if std::env::var("RUST_LOG").is_ok() {
        let filter = EnvFilter::from_default_env();
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    }

    let backend: Arc<dyn SynapticaBackend> = match cli.mode.as_str() {
        "stdio" | "local" => {
            let backend = LocalBackend::new(&cli.data_dir, &cli.graph)?;
            Arc::new(backend)
        }
        "remote" => {
            let backend = RemoteBackend::connect(&cli.server_url, &cli.graph).await?;
            Arc::new(backend)
        }
        other => {
            anyhow::bail!(
                "Unknown mode '{}'. Use 'stdio' (local embedded) or 'remote' (gRPC client).",
                other
            );
        }
    };

    let server = SynapticaMcpServer::new(backend);

    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| anyhow::anyhow!("MCP server error: {}", e))?;

    service
        .waiting()
        .await
        .map_err(|e| anyhow::anyhow!("MCP server stopped: {}", e))?;

    Ok(())
}
