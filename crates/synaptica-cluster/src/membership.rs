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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(id: &str, alive: bool) -> ClusterNode {
        ClusterNode {
            id: id.to_string(),
            address: format!("127.0.0.1:{}", id),
            is_alive: alive,
            last_heartbeat: Instant::now(),
        }
    }

    #[test]
    fn test_add_node() {
        let mut m = ClusterMembership::new();
        m.add_node(make_node("n1", true));
        assert!(m.get_node("n1").is_some());
    }

    #[test]
    fn test_add_duplicate_node() {
        let mut m = ClusterMembership::new();
        m.add_node(make_node("n1", true));
        m.add_node(make_node("n1", true));
        assert_eq!(m.alive_nodes().len(), 1);
    }

    #[test]
    fn test_remove_node() {
        let mut m = ClusterMembership::new();
        m.add_node(make_node("n1", true));
        m.add_node(make_node("n2", true));
        m.add_node(make_node("n3", true));
        m.remove_node("n2");
        assert!(m.get_node("n2").is_none());
        assert!(m.get_node("n1").is_some());
        assert!(m.get_node("n3").is_some());
    }

    #[test]
    fn test_remove_nonexistent_node() {
        let mut m = ClusterMembership::new();
        m.add_node(make_node("n1", true));
        m.remove_node("n999"); // should not panic
        assert!(m.get_node("n1").is_some());
    }

    #[test]
    fn test_alive_nodes() {
        let mut m = ClusterMembership::new();
        m.add_node(make_node("n1", true));
        m.add_node(make_node("n2", true));
        m.add_node(make_node("n3", false));
        assert_eq!(m.alive_nodes().len(), 2);
    }

    #[test]
    fn test_mark_dead() {
        let mut m = ClusterMembership::new();
        m.add_node(make_node("n1", true));
        m.mark_dead("n1");
        assert!(!m.get_node("n1").unwrap().is_alive);
    }

    #[test]
    fn test_mark_alive() {
        let mut m = ClusterMembership::new();
        m.add_node(make_node("n1", true));
        m.mark_dead("n1");
        let before = m.get_node("n1").unwrap().last_heartbeat;
        std::thread::sleep(std::time::Duration::from_millis(10));
        m.mark_alive("n1");
        let node = m.get_node("n1").unwrap();
        assert!(node.is_alive);
        assert!(node.last_heartbeat > before);
    }

    #[test]
    fn test_empty_membership() {
        let m = ClusterMembership::new();
        assert!(m.alive_nodes().is_empty());
    }

    #[test]
    fn test_get_nonexistent_node() {
        let m = ClusterMembership::new();
        assert!(m.get_node("unknown").is_none());
    }
}
