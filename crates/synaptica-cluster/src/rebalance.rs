use crate::partition::Partition;
use serde::{Deserialize, Serialize};

/// A planned move of a partition from one node to another.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartitionMove {
    pub partition_id: String,
    pub from_node: String,
    pub to_node: String,
}

/// The set of moves required to rebalance partitions across nodes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RebalancePlan {
    pub moves: Vec<PartitionMove>,
}

/// Compute a rebalance plan given current partitions and available nodes.
///
/// Stub: returns an empty plan (no moves).
pub fn compute_rebalance_plan(_partitions: &[Partition], _nodes: &[String]) -> RebalancePlan {
    RebalancePlan::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::partition::{Partition, PartitionId, PartitionRange};

    #[test]
    fn test_empty_plan() {
        let plan = compute_rebalance_plan(&[], &[]);
        assert!(plan.moves.is_empty());
    }

    #[test]
    fn test_rebalance_with_partitions() {
        let partitions = vec![Partition {
            id: PartitionId("p1".to_string()),
            range: PartitionRange {
                start: vec![0x00],
                end: vec![0xFF],
            },
            leader_node: "node-1".to_string(),
            replicas: vec!["node-2".to_string()],
        }];
        let nodes = vec!["node-1".to_string(), "node-2".to_string()];
        let plan = compute_rebalance_plan(&partitions, &nodes);
        // Current stub returns empty plan — just verify the interface works
        assert!(plan.moves.is_empty());
    }
}
