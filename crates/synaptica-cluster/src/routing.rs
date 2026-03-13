use crate::partition::{PartitionId, PartitionMap};
use serde::{Deserialize, Serialize};

/// A sub-query that should be executed against a single partition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubQuery {
    pub query: String,
}

/// Routes queries to the appropriate partitions.
#[derive(Debug, Default)]
pub struct QueryRouter;

impl QueryRouter {
    pub fn new() -> Self {
        Self
    }

    /// Route a query to the relevant partitions.
    ///
    /// Stub: routes the entire query to the first partition.
    pub fn route_query(
        &self,
        query: &str,
        partition_map: &PartitionMap,
    ) -> Vec<(PartitionId, SubQuery)> {
        match partition_map.all_partitions().first() {
            Some(p) => vec![(
                p.id.clone(),
                SubQuery {
                    query: query.to_string(),
                },
            )],
            None => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::partition::{Partition, PartitionRange};

    fn make_partition(id: &str, start: u8, end: u8) -> Partition {
        Partition {
            id: PartitionId(id.to_string()),
            range: PartitionRange {
                start: vec![start],
                end: vec![end],
            },
            leader_node: "leader".to_string(),
            replicas: vec![],
        }
    }

    #[test]
    fn test_route_to_first_partition() {
        let mut pm = PartitionMap::new();
        pm.add_partition(make_partition("p1", 0x00, 0x40)).unwrap();
        pm.add_partition(make_partition("p2", 0x40, 0x80)).unwrap();
        pm.add_partition(make_partition("p3", 0x80, 0xFF)).unwrap();

        let router = QueryRouter::new();
        let result = router.route_query("hello", &pm);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, PartitionId("p1".to_string()));
    }

    #[test]
    fn test_route_empty_partition_map() {
        let pm = PartitionMap::new();
        let router = QueryRouter::new();
        let result = router.route_query("hello", &pm);
        assert!(result.is_empty());
    }

    #[test]
    fn test_route_preserves_query_text() {
        let mut pm = PartitionMap::new();
        pm.add_partition(make_partition("p1", 0x00, 0xFF)).unwrap();

        let router = QueryRouter::new();
        let result = router.route_query("my search query", &pm);
        assert_eq!(result[0].1.query, "my search query");
    }
}
