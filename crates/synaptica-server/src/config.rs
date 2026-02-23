use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub listen_addr: String,
    pub data_dir: String,
    pub default_graph: String,
    pub cluster: Option<ClusterConfig>,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_metrics_enabled")]
    pub metrics_enabled: bool,
    #[serde(default = "default_metrics_addr")]
    pub metrics_addr: String,
    pub tls: Option<TlsConfig>,
    pub auth: Option<AuthConfig>,
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_metrics_enabled() -> bool {
    true
}

fn default_metrics_addr() -> String {
    "0.0.0.0:9091".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    pub node_id: String,
    pub peers: Vec<String>,
    pub listen_addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    pub cert_path: String,
    pub key_path: String,
    pub ca_cert_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub enabled: bool,
    pub tokens: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:9090".to_string(),
            data_dir: "./data".to_string(),
            default_graph: "default".to_string(),
            cluster: None,
            log_level: default_log_level(),
            metrics_enabled: default_metrics_enabled(),
            metrics_addr: default_metrics_addr(),
            tls: None,
            auth: None,
        }
    }
}

impl ServerConfig {
    pub fn from_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let config: ServerConfig = toml::from_str(&contents)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(ref auth) = self.auth {
            if auth.enabled && auth.tokens.is_empty() {
                anyhow::bail!(
                    "auth is enabled but no tokens are configured; add at least one token or disable auth"
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_empty_tokens_validation() {
        let config = ServerConfig {
            auth: Some(AuthConfig {
                enabled: true,
                tokens: vec![],
            }),
            ..ServerConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_auth_with_tokens_validation() {
        let config = ServerConfig {
            auth: Some(AuthConfig {
                enabled: true,
                tokens: vec!["secret-token".to_string()],
            }),
            ..ServerConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_no_auth_validation() {
        let config = ServerConfig {
            auth: None,
            ..ServerConfig::default()
        };
        assert!(config.validate().is_ok());
    }
}