use std::io::Cursor;

use serde::{Deserialize, Serialize};

pub type NodeId = u64;

openraft::declare_raft_types!(
    pub TypeConfig:
        D = RaftRequest,
        R = RaftResponse,
);

pub type SynapticaRaft = openraft::Raft<TypeConfig>;

/// A write request replicated through Raft consensus.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum RaftRequest {
    /// Execute a GQL write query (INSERT, SET, DELETE, CREATE/DROP INDEX, etc.)
    WriteQuery { query: String, graph_name: String },
}

/// Response returned after a Raft entry is applied to the state machine.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RaftResponse {
    pub success: bool,
    pub error: Option<String>,
    pub rows_affected: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialization() {
        let req = RaftRequest::WriteQuery {
            query: "INSERT (:Person {name: 'Alice'})".to_string(),
            graph_name: "default".to_string(),
        };
        let bytes = bincode::serialize(&req).unwrap();
        let deserialized: RaftRequest = bincode::deserialize(&bytes).unwrap();
        match deserialized {
            RaftRequest::WriteQuery { query, graph_name } => {
                assert_eq!(query, "INSERT (:Person {name: 'Alice'})");
                assert_eq!(graph_name, "default");
            }
        }
    }

    #[test]
    fn test_response_serialization() {
        let resp = RaftResponse {
            success: true,
            error: None,
            rows_affected: 5,
        };
        let bytes = bincode::serialize(&resp).unwrap();
        let deserialized: RaftResponse = bincode::deserialize(&bytes).unwrap();
        assert!(deserialized.success);
        assert_eq!(deserialized.rows_affected, 5);
    }
}
