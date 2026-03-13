use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Unified result types shared across local and remote backends.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub stats: QueryStats,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryStats {
    pub rows_returned: u64,
    pub execution_time_ms: f64,
    pub nodes_created: u64,
    pub edges_created: u64,
    pub nodes_deleted: u64,
    pub edges_deleted: u64,
    pub properties_set: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaInfo {
    pub node_labels: Vec<LabelSchema>,
    pub edge_labels: Vec<LabelSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelSchema {
    pub label: String,
    pub property_keys: Vec<String>,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelInfo {
    pub name: String,
    pub count: u64,
    pub kind: String, // "node" or "edge"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexInfo {
    pub label: String,
    pub property: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthInfo {
    pub status: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterInfo {
    pub node_id: String,
    pub role: String,
    pub leader_id: String,
    pub nodes: Vec<ClusterNodeInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterNodeInfo {
    pub node_id: String,
    pub address: String,
    pub role: String,
}

/// GraphSummary for MCP list_graphs responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSummary {
    pub name: String,
    pub id: String,
}

/// BackupSummary for MCP backup listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupSummary {
    pub name: String,
    pub label: String,
    pub created_at: String,
    pub size_bytes: u64,
}

/// Abstraction over local embedded and remote gRPC backends.
#[async_trait]
pub trait SynapticaBackend: Send + Sync {
    async fn execute_query(&self, query: &str, graph: &str) -> anyhow::Result<QueryResult>;
    async fn get_schema(&self, graph: &str) -> anyhow::Result<SchemaInfo>;
    async fn list_labels(&self, graph: &str) -> anyhow::Result<Vec<LabelInfo>>;
    async fn list_indexes(&self, graph: &str) -> anyhow::Result<Vec<IndexInfo>>;
    async fn create_index(
        &self,
        label: &str,
        property: &str,
        graph: &str,
    ) -> anyhow::Result<String>;
    async fn drop_index(&self, label: &str, property: &str, graph: &str) -> anyhow::Result<String>;
    async fn health(&self) -> anyhow::Result<HealthInfo>;
    async fn cluster_status(&self) -> anyhow::Result<Option<ClusterInfo>>;
    async fn list_graphs(&self) -> anyhow::Result<Vec<GraphSummary>>;
    async fn create_backup(&self, label: &str) -> anyhow::Result<String>;
    async fn list_backups(&self) -> anyhow::Result<Vec<BackupSummary>>;
    async fn delete_backup(&self, name: &str) -> anyhow::Result<String>;
}
