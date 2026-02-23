use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub listen_addr: String,
    pub data_dir: String,
    pub default_graph: String,
    pub cluster: Option<ClusterConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    pub node_id: String,
    pub peers: Vec<String>,
    pub listen_addr: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:9090".to_string(),
            data_dir: "./data".to_string(),
            default_graph: "default".to_string(),
            cluster: None,
        }
    }
}

impl ServerConfig {
    pub fn from_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let config: ServerConfig = toml::from_str(&contents)?;
        Ok(config)
    }
}