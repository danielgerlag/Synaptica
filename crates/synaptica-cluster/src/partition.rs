use serde::{Deserialize, Serialize};

/// Unique identifier for a partition.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PartitionId(pub String);

/// Byte-range that a partition is responsible for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartitionRange {
    pub start: Vec<u8>,
    pub end: Vec<u8>,
}

/// A single partition with its range, leader, and replica set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Partition {
    pub id: PartitionId,
    pub range: PartitionRange,
    pub leader_node: String,
    pub replicas: Vec<String>,
}

/// Maps keys to partitions using range-based partitioning.
#[derive(Debug, Clone, Default)]
pub struct PartitionMap {
    partitions: Vec<Partition>,
}

impl PartitionMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_partition(&mut self, partition: Partition) {
        self.partitions.push(partition);
        // Keep sorted by range start for deterministic lookups.
        self.partitions
            .sort_by(|a, b| a.range.start.cmp(&b.range.start));
    }

    /// Find the partition whose range contains `key`.
    pub fn find_partition(&self, key: &[u8]) -> Option<&Partition> {
        self.partitions
            .iter()
            .find(|p| key >= p.range.start.as_slice() && key < p.range.end.as_slice())
    }

    pub fn all_partitions(&self) -> &[Partition] {
        &self.partitions
    }
}