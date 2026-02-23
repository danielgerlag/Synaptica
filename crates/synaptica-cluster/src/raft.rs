use std::time::Duration;

/// Configuration for a Raft node.
#[derive(Debug, Clone)]
pub struct RaftConfig {
    pub node_id: String,
    pub peers: Vec<String>,
    pub election_timeout: Duration,
    pub heartbeat_interval: Duration,
}

/// Placeholder Raft node  will integrate with openraft in a future pass.
#[derive(Debug)]
pub struct RaftNode {
    config: RaftConfig,
    term: u64,
}

impl RaftNode {
    pub fn new(config: RaftConfig) -> Self {
        Self { config, term: 0 }
    }

    /// Returns whether this node believes it is the leader.
    pub fn is_leader(&self) -> bool {
        false
    }

    /// Returns the id of the current leader, if known.
    pub fn leader_id(&self) -> Option<String> {
        None
    }

    /// Returns the current Raft term.
    pub fn current_term(&self) -> u64 {
        self.term
    }

    /// Returns a reference to the node configuration.
    pub fn config(&self) -> &RaftConfig {
        &self.config
    }
}