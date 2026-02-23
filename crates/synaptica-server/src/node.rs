use std::sync::Arc;
use synaptica_core::graph::{GraphId, GraphMeta};
use synaptica_storage::engine::{StorageConfig, StorageEngine};

use crate::config::ServerConfig;

pub struct NodeRuntime {
    pub storage: Arc<StorageEngine>,
    pub default_graph_id: GraphId,
}

impl NodeRuntime {
    pub fn start(config: &ServerConfig) -> anyhow::Result<Self> {
        let storage = StorageEngine::open(&config.data_dir, &StorageConfig::default())?;
        let storage = Arc::new(storage);

        // Create or load the default graph
        let default_graph_id = GraphId::new();
        let meta = GraphMeta {
            id: default_graph_id,
            name: config.default_graph.clone(),
            graph_type: None,
        };
        // Best-effort: if it already exists we ignore the error
        let _ = storage.put_graph_meta(&meta);

        tracing::info!(
            graph = %config.default_graph,
            data_dir = %config.data_dir,
            "node started"
        );

        Ok(Self {
            storage,
            default_graph_id,
        })
    }

    pub fn shutdown(&self) {
        tracing::info!("node shutting down");
    }
}