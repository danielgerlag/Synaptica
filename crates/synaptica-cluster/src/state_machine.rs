use std::sync::Arc;

use synaptica_core::graph::GraphId;
use synaptica_exec::engine::ExecutionEngine;
use synaptica_gql::parser;
use synaptica_gql::planner::QueryPlanner;
use synaptica_storage::engine::StorageEngine;

use crate::raft::{RaftRequest, RaftResponse};

/// Applies Raft-committed mutations to the local storage engine.
///
/// This is called for each committed log entry containing a `RaftRequest::WriteQuery`.
/// It parses the GQL query, plans, and executes it against the local StorageEngine,
/// ensuring all nodes in the cluster apply the same mutations in the same order.
pub struct StateMachineApplier {
    storage: Arc<StorageEngine>,
}

impl StateMachineApplier {
    pub fn new(storage: Arc<StorageEngine>) -> Self {
        Self { storage }
    }

    /// Apply a single write request to the local storage.
    pub fn apply(&self, request: &RaftRequest) -> RaftResponse {
        match request {
            RaftRequest::WriteQuery { query, graph_name } => {
                self.apply_write_query(query, graph_name)
            }
        }
    }

    fn apply_write_query(&self, query: &str, graph_name: &str) -> RaftResponse {
        let graph_id = GraphId::from_name(graph_name);

        // Parse
        let program = match parser::parse(query) {
            Ok(p) => p,
            Err(e) => {
                return RaftResponse {
                    success: false,
                    error: Some(format!("parse error: {}", e)),
                    rows_affected: 0,
                };
            }
        };

        // Plan
        let planner = QueryPlanner::new();
        let plan = match planner.plan(&program) {
            Ok(p) => p,
            Err(e) => {
                return RaftResponse {
                    success: false,
                    error: Some(format!("plan error: {}", e)),
                    rows_affected: 0,
                };
            }
        };

        // Execute
        let engine = ExecutionEngine::new(&self.storage);
        match engine.execute_plan(&plan, &graph_id) {
            Ok(result_set) => RaftResponse {
                success: true,
                error: None,
                rows_affected: result_set.records.len() as i64,
            },
            Err(e) => RaftResponse {
                success: false,
                error: Some(format!("execution error: {}", e)),
                rows_affected: 0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synaptica_storage::engine::StorageConfig;

    #[test]
    fn test_apply_insert() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap());
        let graph_id = GraphId::from_name("test");
        let meta = synaptica_core::graph::GraphMeta {
            id: graph_id,
            name: "test".to_string(),
            graph_type: None,
        };
        storage.put_graph_meta(&meta).unwrap();

        let applier = StateMachineApplier::new(storage.clone());
        let req = RaftRequest::WriteQuery {
            query: "INSERT (:Person {name: 'Alice'})".to_string(),
            graph_name: "test".to_string(),
        };
        let resp = applier.apply(&req);
        assert!(resp.success, "error: {:?}", resp.error);
    }

    #[test]
    fn test_apply_invalid_query() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap());

        let applier = StateMachineApplier::new(storage);
        let req = RaftRequest::WriteQuery {
            query: "THIS IS NOT VALID GQL".to_string(),
            graph_name: "test".to_string(),
        };
        let resp = applier.apply(&req);
        assert!(!resp.success);
        assert!(resp.error.is_some());
    }
}
