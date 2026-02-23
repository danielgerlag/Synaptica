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