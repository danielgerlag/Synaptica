use std::time::Instant;

/// A node in the cluster.
#[derive(Debug, Clone)]
pub struct ClusterNode {
    pub id: String,
    pub address: String,
    pub is_alive: bool,
    pub last_heartbeat: Instant,
}

/// Tracks cluster membership and node liveness.
#[derive(Debug, Default)]
pub struct ClusterMembership {
    nodes: Vec<ClusterNode>,
}

impl ClusterMembership {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, node: ClusterNode) {
        if !self.nodes.iter().any(|n| n.id == node.id) {
            self.nodes.push(node);
        }
    }

    pub fn remove_node(&mut self, id: &str) {
        self.nodes.retain(|n| n.id != id);
    }

    pub fn get_node(&self, id: &str) -> Option<&ClusterNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn alive_nodes(&self) -> Vec<&ClusterNode> {
        self.nodes.iter().filter(|n| n.is_alive).collect()
    }

    pub fn mark_dead(&mut self, id: &str) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.id == id) {
            node.is_alive = false;
        }
    }

    pub fn mark_alive(&mut self, id: &str) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.id == id) {
            node.is_alive = true;
            node.last_heartbeat = Instant::now();
        }
    }
}