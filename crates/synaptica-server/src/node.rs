use std::collections::BTreeMap;
use std::sync::Arc;

use openraft::BasicNode;
use synaptica_core::graph::{GraphId, GraphMeta};
use synaptica_cluster::log_store::RocksLogStore;
use synaptica_cluster::network::GrpcNetwork;
use synaptica_cluster::raft::{NodeId, SynapticaRaft, TypeConfig};
use synaptica_cluster::state_machine::StateMachineApplier;
use synaptica_storage::engine::{StorageConfig, StorageEngine};

use crate::config::ServerConfig;

pub struct NodeRuntime {
    pub storage: Arc<StorageEngine>,
    pub default_graph_id: GraphId,
    /// Raft instance — Some if running in cluster mode, None for standalone.
    pub raft: Option<SynapticaRaft>,
    /// State machine applier for executing replicated mutations.
    pub applier: Option<Arc<StateMachineApplier>>,
    /// This node's ID in the cluster.
    pub node_id: Option<NodeId>,
    /// Cluster listen address for inter-node gRPC.
    pub cluster_addr: Option<String>,
}

impl NodeRuntime {
    pub fn start(config: &ServerConfig) -> anyhow::Result<Self> {
        let storage = StorageEngine::open(&config.data_dir, &StorageConfig::default())?;
        let storage = Arc::new(storage);

        let default_graph_id = GraphId::from_name(&config.default_graph);
        let meta = GraphMeta {
            id: default_graph_id,
            name: config.default_graph.clone(),
            graph_type: None,
        };
        let _ = storage.put_graph_meta(&meta);

        tracing::info!(
            graph = %config.default_graph,
            data_dir = %config.data_dir,
            "node started"
        );

        Ok(Self {
            storage,
            default_graph_id,
            raft: None,
            applier: None,
            node_id: None,
            cluster_addr: None,
        })
    }

    /// Initialize Raft consensus for cluster mode.
    /// Call this after `start()` when cluster config is present.
    pub async fn init_cluster(
        &mut self,
        node_id: NodeId,
        cluster_addr: String,
        peers: Vec<(NodeId, String)>,
    ) -> anyhow::Result<()> {
        self.node_id = Some(node_id);
        self.cluster_addr = Some(cluster_addr.clone());

        // Create the applier that executes committed mutations
        let applier = Arc::new(StateMachineApplier::new(self.storage.clone()));
        self.applier = Some(applier.clone());

        // Create RocksDB-backed log store sharing the same DB, with the applier
        let log_store = Arc::new(RocksLogStore::with_applier(
            self.storage.raw_db().clone(),
            applier.clone(),
        ));

        // Create Raft configuration
        let raft_config = openraft::Config {
            heartbeat_interval: 500,
            election_timeout_min: 1500,
            election_timeout_max: 3000,
            ..Default::default()
        };
        let raft_config = Arc::new(raft_config.validate()?);

        // Create the gRPC network for inter-node communication
        let network = GrpcNetwork;

        // Create the Raft instance using the Adaptor for combined RaftStorage
        let (log_store_adapter, sm_adapter) =
            openraft::storage::Adaptor::<TypeConfig, Arc<RocksLogStore>>::new(log_store);

        let raft = openraft::Raft::new(
            node_id,
            raft_config,
            network,
            log_store_adapter,
            sm_adapter,
        )
        .await?;

        // If this is the first node (no peers), initialize as single-node cluster
        if peers.is_empty() {
            let mut members = BTreeMap::new();
            members.insert(
                node_id,
                BasicNode {
                    addr: cluster_addr.clone(),
                },
            );
            if let Err(e) = raft.initialize(members).await {
                tracing::warn!(error = %e, "raft initialize (may already be initialized)");
            }
        }

        tracing::info!(
            node_id = node_id,
            cluster_addr = %cluster_addr,
            peer_count = peers.len(),
            "raft cluster initialized"
        );

        self.raft = Some(raft);
        Ok(())
    }

    /// Returns true if this node is the Raft leader.
    pub fn is_leader(&self) -> bool {
        match &self.raft {
            Some(raft) => {
                let metrics = raft.metrics().borrow().clone();
                metrics.current_leader == self.node_id
            }
            None => true, // Standalone mode — always "leader"
        }
    }

    /// Returns the leader's node address, if known.
    pub fn leader_addr(&self) -> Option<String> {
        let raft = self.raft.as_ref()?;
        let metrics = raft.metrics().borrow().clone();
        let _leader_id = metrics.current_leader?;
        // TODO: look up leader address in membership config
        None
    }

    pub fn shutdown(&self) {
        tracing::info!("node shutting down");
    }
}