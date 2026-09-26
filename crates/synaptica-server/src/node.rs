use std::collections::BTreeMap;
use std::sync::Arc;

use openraft::BasicNode;
use synaptica_cluster::log_store::RocksLogStore;
use synaptica_cluster::network::GrpcNetwork;
use synaptica_cluster::raft::{NodeId, SynapticaRaft, TypeConfig};
use synaptica_cluster::state_machine::StateMachineApplier;
use synaptica_core::graph::{GraphId, GraphMeta};
use synaptica_storage::engine::{StorageConfig, StorageEngine};

use crate::config::ServerConfig;

/// A follower is healthy when its matched log index is within this many
/// entries of the leader's last log index.
pub const REPLICATION_LAG_HEALTHY: u64 = 64;

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
    /// Node id → cluster gRPC address, including this node.
    pub peer_addrs: BTreeMap<NodeId, String>,
    pub started_at: std::time::Instant,
}

/// Address for `node_id`, preferring the Raft membership map and falling
/// back to the addresses supplied at startup.
pub fn resolve_node_addr(
    node_id: u64,
    membership_addrs: &BTreeMap<u64, String>,
    configured_addrs: &BTreeMap<u64, String>,
) -> String {
    membership_addrs
        .get(&node_id)
        .cloned()
        .filter(|addr| !addr.is_empty())
        .or_else(|| configured_addrs.get(&node_id).cloned())
        .unwrap_or_default()
}

/// Leader address, if both the leader id and an address for it are known.
pub fn resolve_leader_addr(
    leader_id: Option<u64>,
    membership_addrs: &BTreeMap<u64, String>,
    configured_addrs: &BTreeMap<u64, String>,
) -> Option<String> {
    let id = leader_id?;
    let addr = resolve_node_addr(id, membership_addrs, configured_addrs);
    if addr.is_empty() {
        None
    } else {
        Some(addr)
    }
}

/// Whether a member should be reported healthy.
///
/// The leader is healthy. This node is healthy when its Raft runtime is
/// running. Other members are healthy only when this node is the leader and
/// their matched index is within [`REPLICATION_LAG_HEALTHY`] of `last_log_index`.
/// `replication` is `None` when this node is not the leader. A missing matched
/// index means the follower has not reported yet.
pub fn member_is_healthy(
    node_id: u64,
    self_id: u64,
    leader_id: Option<u64>,
    self_running: bool,
    last_log_index: Option<u64>,
    replication: Option<&BTreeMap<u64, Option<u64>>>,
) -> bool {
    if leader_id == Some(node_id) {
        return true;
    }
    if node_id == self_id {
        return self_running;
    }
    let Some(repl) = replication else {
        return false;
    };
    match repl.get(&node_id).copied() {
        Some(Some(matched)) => {
            last_log_index.unwrap_or(0).saturating_sub(matched) <= REPLICATION_LAG_HEALTHY
        }
        _ => false,
    }
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
            peer_addrs: BTreeMap::new(),
            started_at: std::time::Instant::now(),
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
        let mut addrs = BTreeMap::new();
        addrs.insert(node_id, cluster_addr.clone());
        for (id, addr) in &peers {
            addrs.insert(*id, addr.clone());
        }
        self.peer_addrs = addrs;

        // Create the applier that executes committed mutations
        let applier = Arc::new(StateMachineApplier::new(self.storage.clone()));
        self.applier = Some(applier.clone());

        // Create RocksDB-backed log store sharing the same DB, with the applier
        let log_store = Arc::new(RocksLogStore::with_applier(
            self.storage.clone(),
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

        let raft =
            openraft::Raft::new(node_id, raft_config, network, log_store_adapter, sm_adapter)
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

    /// Returns the leader's cluster address, if this node knows it.
    pub fn leader_addr(&self) -> Option<String> {
        let raft = self.raft.as_ref()?;
        let metrics = raft.metrics().borrow().clone();
        let mut membership_addrs = BTreeMap::new();
        for (id, node) in metrics.membership_config.membership().nodes() {
            membership_addrs.insert(*id, node.addr.clone());
        }
        resolve_leader_addr(
            metrics.current_leader,
            &membership_addrs,
            &self.peer_addrs,
        )
    }

    pub fn shutdown(&self) {
        tracing::info!("node shutting down");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addrs(pairs: &[(u64, &str)]) -> BTreeMap<u64, String> {
        pairs
            .iter()
            .map(|(id, addr)| (*id, (*addr).to_string()))
            .collect()
    }

    #[test]
    fn leader_addr_prefers_membership_over_startup_config() {
        let membership = addrs(&[(1, "10.0.0.1:9191"), (2, "10.0.0.2:9191")]);
        let configured = addrs(&[(1, "127.0.0.1:9191"), (2, "127.0.0.1:9192")]);
        assert_eq!(
            resolve_leader_addr(Some(2), &membership, &configured).as_deref(),
            Some("10.0.0.2:9191")
        );
    }

    #[test]
    fn leader_addr_falls_back_to_configured_peers() {
        let membership = BTreeMap::new();
        let configured = addrs(&[(1, "127.0.0.1:9191"), (2, "127.0.0.1:9192")]);
        assert_eq!(
            resolve_leader_addr(Some(2), &membership, &configured).as_deref(),
            Some("127.0.0.1:9192")
        );
    }

    #[test]
    fn leader_addr_is_none_when_unknown() {
        let empty = BTreeMap::new();
        assert_eq!(resolve_leader_addr(Some(9), &empty, &empty), None);
        assert_eq!(resolve_leader_addr(None, &empty, &empty), None);
    }

    #[test]
    fn follower_health_uses_replication_lag() {
        let mut repl = BTreeMap::new();
        repl.insert(2, Some(100));
        repl.insert(3, Some(10));
        repl.insert(4, None);

        assert!(member_is_healthy(1, 1, Some(1), true, Some(120), Some(&repl)));
        assert!(member_is_healthy(2, 1, Some(1), true, Some(120), Some(&repl)));
        assert!(!member_is_healthy(
            3,
            1,
            Some(1),
            true,
            Some(120),
            Some(&repl)
        ));
        assert!(!member_is_healthy(
            4,
            1,
            Some(1),
            true,
            Some(120),
            Some(&repl)
        ));
        // A follower does not have a replication map, so it cannot confirm peers.
        assert!(!member_is_healthy(2, 3, Some(1), true, Some(120), None));
        assert!(member_is_healthy(3, 3, Some(1), true, Some(120), None));
        assert!(!member_is_healthy(3, 3, Some(1), false, Some(120), None));
    }
}
