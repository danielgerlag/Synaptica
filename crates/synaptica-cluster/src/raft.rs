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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> RaftConfig {
        RaftConfig {
            node_id: "node-1".to_string(),
            peers: vec!["node-2".to_string(), "node-3".to_string()],
            election_timeout: Duration::from_millis(300),
            heartbeat_interval: Duration::from_millis(100),
        }
    }

    #[test]
    fn test_raft_node_creation() {
        let node = RaftNode::new(make_config());
        assert_eq!(node.config().node_id, "node-1");
    }

    #[test]
    fn test_initial_term_is_zero() {
        let node = RaftNode::new(make_config());
        assert_eq!(node.current_term(), 0);
    }

    #[test]
    fn test_is_leader_initially_false() {
        let node = RaftNode::new(make_config());
        assert!(!node.is_leader());
    }

    #[test]
    fn test_leader_id_initially_none() {
        let node = RaftNode::new(make_config());
        assert!(node.leader_id().is_none());
    }

    #[test]
    fn test_config_preserved() {
        let node = RaftNode::new(make_config());
        let cfg = node.config();
        assert_eq!(cfg.node_id, "node-1");
        assert_eq!(cfg.peers, vec!["node-2".to_string(), "node-3".to_string()]);
    }
}