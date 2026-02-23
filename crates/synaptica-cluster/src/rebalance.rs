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
pub fn compute_rebalance_plan(
    _partitions: &[Partition],
    _nodes: &[String],
) -> RebalancePlan {
    RebalancePlan::default()
}