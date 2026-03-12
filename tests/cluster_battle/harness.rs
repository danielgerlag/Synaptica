//! In-process multi-node Raft test harness.
//!
//! Provides `TestCluster` which manages multiple Raft nodes communicating
//! through direct in-process method calls (no gRPC). This allows testing
//! real Raft consensus, leader election, log replication, and membership
//! changes in unit tests.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use openraft::error::{
    InstallSnapshotError, NetworkError, RPCError, RaftError, RemoteError, Unreachable,
};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest,
    InstallSnapshotResponse, VoteRequest, VoteResponse,
};
use openraft::BasicNode;
use tokio::sync::RwLock;

use synaptica_cluster::log_store::RocksLogStore;
use synaptica_cluster::raft::{NodeId, RaftRequest, RaftResponse, SynapticaRaft, TypeConfig};
use synaptica_cluster::state_machine::StateMachineApplier;
use synaptica_core::graph::{GraphId, GraphMeta};
use synaptica_storage::engine::{StorageConfig, StorageEngine};

// ---------------------------------------------------------------------------
// In-process network router
// ---------------------------------------------------------------------------

/// Routes Raft RPCs between in-process Raft nodes.
#[derive(Clone)]
pub struct TestRouter {
    source_id: Option<NodeId>,
    routes: Arc<RwLock<HashMap<NodeId, SynapticaRaft>>>,
    /// Controls which nodes are "reachable". If a node is in the blocked set,
    /// RPCs to or from it will fail with Unreachable.
    blocked: Arc<RwLock<BTreeSet<NodeId>>>,
}

impl TestRouter {
    pub fn new() -> Self {
        Self {
            source_id: None,
            routes: Arc::new(RwLock::new(HashMap::new())),
            blocked: Arc::new(RwLock::new(BTreeSet::new())),
        }
    }

    /// Create a source-aware clone for a specific node.
    pub fn clone_for_node(&self, id: NodeId) -> Self {
        Self {
            source_id: Some(id),
            routes: self.routes.clone(),
            blocked: self.blocked.clone(),
        }
    }

    pub async fn register(&self, id: NodeId, raft: SynapticaRaft) {
        self.routes.write().await.insert(id, raft);
    }

    pub async fn unregister(&self, id: NodeId) {
        self.routes.write().await.remove(&id);
    }

    pub async fn get(&self, id: NodeId) -> Option<SynapticaRaft> {
        self.routes.read().await.get(&id).cloned()
    }

    /// Block RPCs to the given node (simulate network partition).
    pub async fn block_node(&self, id: NodeId) {
        self.blocked.write().await.insert(id);
    }

    /// Unblock RPCs to the given node (heal partition).
    pub async fn unblock_node(&self, id: NodeId) {
        self.blocked.write().await.remove(&id);
    }

    pub async fn is_blocked(&self, id: NodeId) -> bool {
        self.blocked.read().await.contains(&id)
    }
}

impl RaftNetworkFactory<TypeConfig> for TestRouter {
    type Network = TestNetworkConn;

    async fn new_client(&mut self, target: NodeId, _node: &BasicNode) -> Self::Network {
        TestNetworkConn {
            target,
            router: self.clone(),
        }
    }
}

/// In-process network connection to a single peer.
pub struct TestNetworkConn {
    target: NodeId,
    router: TestRouter,
}

impl TestNetworkConn {
    /// Returns true if either the source or target is blocked.
    async fn is_partitioned(&self) -> bool {
        if self.router.is_blocked(self.target).await {
            return true;
        }
        if let Some(src) = self.router.source_id {
            if self.router.is_blocked(src).await {
                return true;
            }
        }
        false
    }
}

type TestRPCError<E = openraft::error::Infallible> =
    RPCError<NodeId, BasicNode, RaftError<NodeId, E>>;

impl RaftNetwork<TypeConfig> for TestNetworkConn {
    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<NodeId>, TestRPCError> {
        if self.is_partitioned().await {
            return Err(RPCError::Unreachable(Unreachable::new(&std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "node blocked",
            ))));
        }
        let raft = self.router.get(self.target).await.ok_or_else(|| {
            RPCError::Unreachable(Unreachable::new(&std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("node {} not found", self.target),
            )))
        })?;
        raft.append_entries(req)
            .await
            .map_err(|e| RPCError::RemoteError(RemoteError::new(self.target, e)))
    }

    async fn install_snapshot(
        &mut self,
        req: InstallSnapshotRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<InstallSnapshotResponse<NodeId>, TestRPCError<InstallSnapshotError>> {
        if self.is_partitioned().await {
            return Err(RPCError::Unreachable(Unreachable::new(&std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "node blocked",
            ))));
        }
        let raft = self.router.get(self.target).await.ok_or_else(|| {
            RPCError::Unreachable(Unreachable::new(&std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("node {} not found", self.target),
            )))
        })?;
        raft.install_snapshot(req)
            .await
            .map_err(|e| RPCError::RemoteError(RemoteError::new(self.target, e)))
    }

    async fn vote(
        &mut self,
        req: VoteRequest<NodeId>,
        _option: RPCOption,
    ) -> Result<VoteResponse<NodeId>, TestRPCError> {
        if self.is_partitioned().await {
            return Err(RPCError::Unreachable(Unreachable::new(&std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "node blocked",
            ))));
        }
        let raft = self.router.get(self.target).await.ok_or_else(|| {
            RPCError::Unreachable(Unreachable::new(&std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("node {} not found", self.target),
            )))
        })?;
        raft.vote(req)
            .await
            .map_err(|e| RPCError::RemoteError(RemoteError::new(self.target, e)))
    }
}

// ---------------------------------------------------------------------------
// TestNode — a single node in the test cluster
// ---------------------------------------------------------------------------

pub struct TestNode {
    pub id: NodeId,
    pub raft: SynapticaRaft,
    pub storage: Arc<StorageEngine>,
    pub applier: Arc<StateMachineApplier>,
    pub _dir: tempfile::TempDir,
}

// ---------------------------------------------------------------------------
// TestCluster — manages multiple Raft nodes
// ---------------------------------------------------------------------------

pub struct TestCluster {
    pub router: TestRouter,
    pub nodes: HashMap<NodeId, TestNode>,
    pub graph_name: String,
}

impl TestCluster {
    /// Create a cluster with `n` nodes (IDs 1..=n), initialize membership,
    /// and wait for a leader to be elected.
    pub async fn new(n: u64) -> Self {
        Self::new_with_graph(n, "test").await
    }

    pub async fn new_with_graph(n: u64, graph_name: &str) -> Self {
        let router = TestRouter::new();
        let mut nodes = HashMap::new();

        let raft_config = Arc::new(
            openraft::Config {
                heartbeat_interval: 100,
                election_timeout_min: 300,
                election_timeout_max: 600,
                max_payload_entries: 100,
                snapshot_policy: openraft::SnapshotPolicy::LogsSinceLast(1000),
                ..Default::default()
            }
            .validate()
            .unwrap(),
        );

        // Create all nodes
        for id in 1..=n {
            let dir = tempfile::tempdir().unwrap();
            let storage = Arc::new(
                StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap(),
            );

            // Create the default graph
            let graph_id = GraphId::from_name(graph_name);
            let meta = GraphMeta {
                id: graph_id,
                name: graph_name.to_string(),
                graph_type: None,
            };
            storage.put_graph_meta(&meta).unwrap();

            let applier = Arc::new(StateMachineApplier::new(storage.clone()));
            let log_store = Arc::new(RocksLogStore::with_applier(
                storage.raw_db().clone(),
                applier.clone(),
            ));

            let (ls, sm) =
                openraft::storage::Adaptor::<TypeConfig, Arc<RocksLogStore>>::new(log_store);

            let raft: SynapticaRaft = openraft::Raft::new(
                id,
                raft_config.clone(),
                router.clone_for_node(id),
                ls,
                sm,
            )
            .await
            .unwrap();

            router.register(id, raft.clone()).await;
            nodes.insert(
                id,
                TestNode {
                    id,
                    raft,
                    storage,
                    applier,
                    _dir: dir,
                },
            );
        }

        // Initialize cluster membership with all nodes as voters
        let mut members = BTreeMap::new();
        for id in 1..=n {
            members.insert(
                id,
                BasicNode {
                    addr: format!("127.0.0.1:{}", 9190 + id),
                },
            );
        }
        // Initialize on node 1
        nodes
            .get(&1)
            .unwrap()
            .raft
            .initialize(members)
            .await
            .unwrap();

        let mut cluster = TestCluster {
            router,
            nodes,
            graph_name: graph_name.to_string(),
        };

        // Wait for leader election
        cluster.wait_for_leader(5000).await;

        cluster
    }

    /// Wait until a leader is elected, up to `timeout_ms`.
    pub async fn wait_for_leader(&self, timeout_ms: u64) {
        let start = tokio::time::Instant::now();
        let timeout = tokio::time::Duration::from_millis(timeout_ms);
        loop {
            if start.elapsed() > timeout {
                panic!("timeout waiting for leader election");
            }
            if self.get_leader().is_some() {
                return;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
    }

    /// Find the current leader node ID.
    pub fn get_leader(&self) -> Option<NodeId> {
        for node in self.nodes.values() {
            let metrics = node.raft.metrics().borrow().clone();
            if let Some(leader) = metrics.current_leader {
                // Verify the leader node itself agrees
                if let Some(leader_node) = self.nodes.get(&leader) {
                    let leader_metrics = leader_node.raft.metrics().borrow().clone();
                    if leader_metrics.current_leader == Some(leader) {
                        return Some(leader);
                    }
                }
            }
        }
        None
    }

    /// Get the Raft metrics for a node.
    pub fn metrics(&self, id: NodeId) -> openraft::RaftMetrics<NodeId, BasicNode> {
        self.nodes.get(&id).unwrap().raft.metrics().borrow().clone()
    }

    /// Write a GQL mutation through Raft consensus via the leader.
    pub async fn write(&self, query: &str) -> Result<RaftResponse, String> {
        let leader = self.get_leader().ok_or("no leader")?;
        self.write_on(leader, query).await
    }

    /// Write a GQL mutation through a specific node's Raft.
    pub async fn write_on(&self, node_id: NodeId, query: &str) -> Result<RaftResponse, String> {
        let node = self.nodes.get(&node_id).ok_or("node not found")?;
        let req = RaftRequest::WriteQuery {
            query: query.to_string(),
            graph_name: self.graph_name.clone(),
        };
        node.raft
            .client_write(req)
            .await
            .map(|r| r.data)
            .map_err(|e| format!("{}", e))
    }

    /// Execute a read query locally on a specific node (no Raft).
    pub fn read_on(
        &self,
        node_id: NodeId,
        query: &str,
    ) -> Result<synaptica_exec::result::ResultSet, String> {
        let node = self.nodes.get(&node_id).ok_or("node not found")?;
        let graph_id = GraphId::from_name(&self.graph_name);
        let program = synaptica_gql::parser::parse(query).map_err(|e| format!("{}", e))?;
        let planner = synaptica_gql::planner::QueryPlanner::new();
        let plan = planner.plan(&program).map_err(|e| format!("{}", e))?;
        let engine = synaptica_exec::engine::ExecutionEngine::new(&node.storage);
        engine
            .execute_plan(&plan, &graph_id)
            .map_err(|e| format!("{}", e))
    }

    /// Count nodes in storage on a specific cluster node.
    pub fn count_nodes_on(&self, node_id: NodeId) -> usize {
        let node = self.nodes.get(&node_id).unwrap();
        let graph_id = GraphId::from_name(&self.graph_name);
        node.storage.scan_nodes(&graph_id).unwrap().len()
    }

    /// Count edges in storage on a specific cluster node.
    pub fn count_edges_on(&self, node_id: NodeId) -> usize {
        let node = self.nodes.get(&node_id).unwrap();
        let graph_id = GraphId::from_name(&self.graph_name);
        node.storage.scan_edges_limit(&graph_id, usize::MAX).unwrap().len()
    }

    /// Wait for replication to converge — all nodes have the same node count.
    pub async fn wait_for_convergence(&self, timeout_ms: u64) {
        let start = tokio::time::Instant::now();
        let timeout = tokio::time::Duration::from_millis(timeout_ms);
        loop {
            if start.elapsed() > timeout {
                let counts: Vec<_> = self
                    .nodes
                    .keys()
                    .map(|id| (*id, self.count_nodes_on(*id)))
                    .collect();
                panic!(
                    "timeout waiting for convergence. node counts: {:?}",
                    counts
                );
            }
            let counts: Vec<usize> = self
                .nodes
                .keys()
                .map(|id| self.count_nodes_on(*id))
                .collect();
            if counts.windows(2).all(|w| w[0] == w[1]) && counts[0] > 0 {
                return;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
    }

    /// Wait for all non-blocked nodes to agree on node count.
    pub async fn wait_for_convergence_count(&self, expected: usize, timeout_ms: u64) {
        let start = tokio::time::Instant::now();
        let timeout = tokio::time::Duration::from_millis(timeout_ms);
        loop {
            if start.elapsed() > timeout {
                let counts: Vec<_> = self
                    .nodes
                    .keys()
                    .map(|id| (*id, self.count_nodes_on(*id)))
                    .collect();
                panic!(
                    "timeout waiting for convergence to {}. node counts: {:?}",
                    expected, counts
                );
            }
            let all_match = self
                .nodes
                .keys()
                .all(|id| {
                    let blocked = {
                        let b = self.router.blocked.try_read();
                        b.map_or(false, |b| b.contains(id))
                    };
                    blocked || self.count_nodes_on(*id) == expected
                });
            if all_match {
                return;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
    }

    /// Shutdown all Raft nodes.
    pub async fn shutdown(self) {
        for (_, node) in self.nodes {
            let _ = node.raft.shutdown().await;
        }
    }

    /// Add a new node as a learner to the cluster.
    pub async fn add_learner(&mut self, new_id: NodeId) {
        let dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(
            StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap(),
        );
        let graph_id = GraphId::from_name(&self.graph_name);
        let meta = GraphMeta {
            id: graph_id,
            name: self.graph_name.clone(),
            graph_type: None,
        };
        storage.put_graph_meta(&meta).unwrap();

        let applier = Arc::new(StateMachineApplier::new(storage.clone()));
        let log_store = Arc::new(RocksLogStore::with_applier(
            storage.raw_db().clone(),
            applier.clone(),
        ));

        let raft_config = Arc::new(
            openraft::Config {
                heartbeat_interval: 100,
                election_timeout_min: 300,
                election_timeout_max: 600,
                max_payload_entries: 100,
                ..Default::default()
            }
            .validate()
            .unwrap(),
        );

        let (ls, sm) =
            openraft::storage::Adaptor::<TypeConfig, Arc<RocksLogStore>>::new(log_store);

        let raft: SynapticaRaft = openraft::Raft::new(
            new_id,
            raft_config,
            self.router.clone_for_node(new_id),
            ls,
            sm,
        )
        .await
        .unwrap();

        self.router.register(new_id, raft.clone()).await;
        self.nodes.insert(
            new_id,
            TestNode {
                id: new_id,
                raft,
                storage,
                applier,
                _dir: dir,
            },
        );

        // Add as learner via leader
        let leader = self.get_leader().expect("need leader to add learner");
        let leader_raft = &self.nodes.get(&leader).unwrap().raft;
        leader_raft
            .add_learner(
                new_id,
                BasicNode {
                    addr: format!("127.0.0.1:{}", 9190 + new_id),
                },
                true,
            )
            .await
            .unwrap();
    }

    /// Promote a set of node IDs to voters via change_membership.
    pub async fn change_membership(&self, member_ids: BTreeSet<NodeId>) {
        let leader = self.get_leader().expect("need leader");
        let leader_raft = &self.nodes.get(&leader).unwrap().raft;
        leader_raft
            .change_membership(member_ids, false)
            .await
            .unwrap();
    }
}
