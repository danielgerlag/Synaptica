use clap::Parser;
use tonic::transport::Server;
use tracing_subscriber::EnvFilter;

use synaptica_server::client_service::proto::synaptica_service_server::SynapticaServiceServer;
use synaptica_server::client_service::SynapticaServiceImpl;
use synaptica_server::config::ServerConfig;
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
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("synaptica=info".parse()?))
        .init();

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

    let node = NodeRuntime::start(&config)?;

    let svc = SynapticaServiceImpl {
        storage: node.storage.clone(),
        default_graph_id: node.default_graph_id,
    };

    let addr = config.listen_addr.parse()?;
    tracing::info!(%addr, "starting gRPC server");

    Server::builder()
        .add_service(SynapticaServiceServer::new(svc))
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
