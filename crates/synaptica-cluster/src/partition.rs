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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_partition(id: &str, start: u8, end: u8) -> Partition {
        Partition {
            id: PartitionId(id.to_string()),
            range: PartitionRange {
                start: vec![start],
                end: vec![end],
            },
            leader_node: "leader".to_string(),
            replicas: vec!["r1".to_string()],
        }
    }

    #[test]
    fn test_add_partition() {
        let mut pm = PartitionMap::new();
        pm.add_partition(make_partition("p1", 0x00, 0x80));
        assert_eq!(pm.all_partitions().len(), 1);
        assert_eq!(pm.all_partitions()[0].id, PartitionId("p1".to_string()));
    }

    #[test]
    fn test_find_partition_in_range() {
        let mut pm = PartitionMap::new();
        pm.add_partition(make_partition("p1", 0x00, 0x80));
        let found = pm.find_partition(&[0x40]);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, PartitionId("p1".to_string()));
    }

    #[test]
    fn test_find_partition_not_in_range() {
        let mut pm = PartitionMap::new();
        pm.add_partition(make_partition("p1", 0x00, 0x80));
        assert!(pm.find_partition(&[0xFF]).is_none());
    }

    #[test]
    fn test_multiple_partitions() {
        let mut pm = PartitionMap::new();
        pm.add_partition(make_partition("p1", 0x00, 0x40));
        pm.add_partition(make_partition("p2", 0x40, 0x80));
        pm.add_partition(make_partition("p3", 0x80, 0xFF));

        assert_eq!(pm.find_partition(&[0x20]).unwrap().id, PartitionId("p1".to_string()));
        assert_eq!(pm.find_partition(&[0x60]).unwrap().id, PartitionId("p2".to_string()));
        assert_eq!(pm.find_partition(&[0xA0]).unwrap().id, PartitionId("p3".to_string()));
    }

    #[test]
    fn test_partition_boundary_start() {
        let mut pm = PartitionMap::new();
        pm.add_partition(make_partition("p1", 0x00, 0x80));
        assert!(pm.find_partition(&[0x00]).is_some());
    }

    #[test]
    fn test_partition_boundary_end() {
        let mut pm = PartitionMap::new();
        pm.add_partition(make_partition("p1", 0x00, 0x80));
        // End is exclusive
        assert!(pm.find_partition(&[0x80]).is_none());
    }

    #[test]
    fn test_empty_partition_map() {
        let pm = PartitionMap::new();
        assert!(pm.find_partition(&[0x42]).is_none());
    }

    #[test]
    fn test_partitions_sorted_after_add() {
        let mut pm = PartitionMap::new();
        // Add out of order
        pm.add_partition(make_partition("p3", 0x80, 0xFF));
        pm.add_partition(make_partition("p1", 0x00, 0x40));
        pm.add_partition(make_partition("p2", 0x40, 0x80));

        let partitions = pm.all_partitions();
        assert_eq!(partitions[0].id, PartitionId("p1".to_string()));
        assert_eq!(partitions[1].id, PartitionId("p2".to_string()));
        assert_eq!(partitions[2].id, PartitionId("p3".to_string()));
    }
}