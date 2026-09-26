use std::sync::Arc;

use synaptica_core::graph::GraphId;
use synaptica_exec::engine::{execute_program, ExecOptions, ProgramError};
use synaptica_gql::parser;
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

        match execute_program(&self.storage, &graph_id, &program, &ExecOptions::default()) {
            Ok(result_set) => RaftResponse {
                success: true,
                error: None,
                rows_affected: result_set.records.len() as i64,
            },
            Err(ProgramError::Plan(e)) => RaftResponse {
                success: false,
                error: Some(format!("plan error: {}", e)),
                rows_affected: 0,
            },
            Err(ProgramError::Execution(e)) => RaftResponse {
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

    #[test]
    fn test_applier_matches_local_execute() {
        let query = "INSERT (:Person {name: 'Alice', age: 30})";
        let graph_name = "test";

        let dir_a = tempfile::tempdir().unwrap();
        let storage_a =
            Arc::new(StorageEngine::open(dir_a.path(), &StorageConfig::default()).unwrap());
        let dir_b = tempfile::tempdir().unwrap();
        let storage_b =
            Arc::new(StorageEngine::open(dir_b.path(), &StorageConfig::default()).unwrap());

        let graph_id = GraphId::from_name(graph_name);
        for storage in [&storage_a, &storage_b] {
            storage
                .put_graph_meta(&synaptica_core::graph::GraphMeta {
                    id: graph_id,
                    name: graph_name.to_string(),
                    graph_type: None,
                })
                .unwrap();
        }

        let applier = StateMachineApplier::new(storage_a.clone());
        let resp = applier.apply(&RaftRequest::WriteQuery {
            query: query.to_string(),
            graph_name: graph_name.to_string(),
        });
        assert!(resp.success, "applier error: {:?}", resp.error);

        let program = parser::parse(query).unwrap();
        execute_program(&storage_b, &graph_id, &program, &ExecOptions::default()).unwrap();

        let applied = storage_a.scan_nodes(&graph_id).unwrap();
        let local = storage_b.scan_nodes(&graph_id).unwrap();
        assert_eq!(applied.len(), 1, "raft apply must insert one node");
        assert_eq!(local.len(), 1, "local execute must insert one node");
        assert_eq!(applied[0].labels, local[0].labels);
        assert_eq!(applied[0].properties, local[0].properties);
    }
}
