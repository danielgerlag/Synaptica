use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ProtocolVersion, ServerCapabilities, ServerInfo};
use rmcp::schemars;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use serde::Deserialize;

use crate::backend::SynapticaBackend;

#[derive(Clone)]
pub struct SynapticaMcpServer {
    backend: Arc<dyn SynapticaBackend>,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryParams {
    /// The GQL query to execute
    pub query: String,
    /// Graph name (optional, uses default if omitted)
    pub graph: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GraphParams {
    /// Graph name (optional, uses default if omitted)
    pub graph: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IndexParams {
    /// Node label to index (e.g. 'Person')
    pub label: String,
    /// Property name to index (e.g. 'name')
    pub property: String,
    /// Graph name (optional)
    pub graph: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateGraphParams {
    /// Name of the graph to create
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BackupParams {
    /// Label for this backup (e.g. 'before-migration', 'daily')
    pub label: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteBackupParams {
    /// Name of the backup to delete
    pub name: String,
}

#[tool_router]
impl SynapticaMcpServer {
    pub fn new(backend: Arc<dyn SynapticaBackend>) -> Self {
        Self {
            backend,
            tool_router: Self::tool_router(),
        }
    }

    /// Execute a GQL (Graph Query Language) query against the Synaptica graph database.
    /// Supports MATCH (reads), INSERT (create nodes/edges), SET (update), DELETE,
    /// CREATE/DROP INDEX. Returns columns and rows as a table.
    /// Use MATCH (n:Label) RETURN n to explore data.
    #[tool(name = "query")]
    async fn query(&self, Parameters(params): Parameters<QueryParams>) -> String {
        let graph = params.graph.unwrap_or_default();
        match self.backend.execute_query(&params.query, &graph).await {
            Ok(result) => {
                if let Some(err) = &result.error {
                    return format!("Error: {}", err);
                }
                if result.columns.is_empty() && result.rows.is_empty() {
                    return format!(
                        "Query executed successfully in {:.1}ms. {}",
                        result.stats.execution_time_ms,
                        format_mutation_stats(&result.stats),
                    );
                }
                format_query_result(&result)
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Get the graph schema — lists all node and edge labels with their property keys and counts.
    /// Use this to understand the structure of data in the database before writing queries.
    #[tool(name = "get_schema")]
    async fn get_schema(&self, Parameters(params): Parameters<GraphParams>) -> String {
        let graph = params.graph.unwrap_or_default();
        match self.backend.get_schema(&graph).await {
            Ok(schema) => {
                let mut out = String::new();
                out.push_str("=== Graph Schema ===\n\n");

                if schema.node_labels.is_empty() && schema.edge_labels.is_empty() {
                    out.push_str("(empty graph — no nodes or edges)\n");
                    return out;
                }

                if !schema.node_labels.is_empty() {
                    out.push_str("Node Labels:\n");
                    for ls in &schema.node_labels {
                        out.push_str(&format!(
                            "  :{} ({} nodes) — properties: {}\n",
                            ls.label,
                            ls.count,
                            if ls.property_keys.is_empty() {
                                "(none)".to_string()
                            } else {
                                ls.property_keys.join(", ")
                            }
                        ));
                    }
                }

                if !schema.edge_labels.is_empty() {
                    out.push_str("\nEdge Labels:\n");
                    for ls in &schema.edge_labels {
                        out.push_str(&format!(
                            "  :{} ({} edges) — properties: {}\n",
                            ls.label,
                            ls.count,
                            if ls.property_keys.is_empty() {
                                "(none)".to_string()
                            } else {
                                ls.property_keys.join(", ")
                            }
                        ));
                    }
                }

                out
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    /// List all node and edge labels in the graph with their counts.
    #[tool(name = "list_labels")]
    async fn list_labels(&self, Parameters(params): Parameters<GraphParams>) -> String {
        let graph = params.graph.unwrap_or_default();
        match self.backend.list_labels(&graph).await {
            Ok(labels) => {
                if labels.is_empty() {
                    return "No labels found (empty graph).".to_string();
                }
                let mut out = String::new();
                for l in &labels {
                    out.push_str(&format!("  :{} ({}, {} count)\n", l.name, l.kind, l.count));
                }
                out
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    /// List all secondary property indexes in the graph.
    #[tool(name = "list_indexes")]
    async fn list_indexes(&self, Parameters(params): Parameters<GraphParams>) -> String {
        let graph = params.graph.unwrap_or_default();
        match self.backend.list_indexes(&graph).await {
            Ok(indexes) => {
                if indexes.is_empty() {
                    return "No indexes defined.".to_string();
                }
                let mut out = String::from("Indexes:\n");
                for idx in &indexes {
                    out.push_str(&format!("  :{} ({})\n", idx.label, idx.property));
                }
                out
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Create a secondary property index on a node label and property for faster lookups.
    #[tool(name = "create_index")]
    async fn create_index(&self, Parameters(params): Parameters<IndexParams>) -> String {
        let graph = params.graph.unwrap_or_default();
        match self
            .backend
            .create_index(&params.label, &params.property, &graph)
            .await
        {
            Ok(msg) => msg,
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Drop a secondary property index.
    #[tool(name = "drop_index")]
    async fn drop_index(&self, Parameters(params): Parameters<IndexParams>) -> String {
        let graph = params.graph.unwrap_or_default();
        match self
            .backend
            .drop_index(&params.label, &params.property, &graph)
            .await
        {
            Ok(msg) => msg,
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Check the health of the Synaptica database.
    #[tool(name = "health")]
    async fn health(&self) -> String {
        match self.backend.health().await {
            Ok(h) => format!("Status: {}\nVersion: {}", h.status, h.version),
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Get Raft cluster status: leader, role, and member nodes.
    /// Only available in remote mode connected to a clustered server.
    #[tool(name = "cluster_status")]
    async fn cluster_status(&self) -> String {
        match self.backend.cluster_status().await {
            Ok(Some(info)) => {
                let mut out = format!(
                    "Node: {} ({})\nLeader: {}\n\nMembers:\n",
                    info.node_id, info.role, info.leader_id
                );
                for n in &info.nodes {
                    out.push_str(&format!("  {} — {} ({})\n", n.node_id, n.address, n.role));
                }
                out
            }
            Ok(None) => "Not running in cluster mode.".to_string(),
            Err(e) => format!("Error: {}", e),
        }
    }

    /// List all graphs available in the database.
    #[tool(name = "list_graphs")]
    async fn list_graphs(&self) -> String {
        match self.backend.list_graphs().await {
            Ok(graphs) => {
                if graphs.is_empty() {
                    return "No graphs found.".to_string();
                }
                let mut out = String::from("Available graphs:\n");
                for g in &graphs {
                    out.push_str(&format!("  {} (id: {})\n", g.name, g.id));
                }
                out
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Create a new named graph in the database.
    /// Use this to create isolated graph spaces for different datasets.
    #[tool(name = "create_graph")]
    async fn create_graph(&self, Parameters(params): Parameters<CreateGraphParams>) -> String {
        let query = format!("CREATE GRAPH {}", params.name);
        match self.backend.execute_query(&query, "").await {
            Ok(result) => {
                if let Some(err) = &result.error {
                    format!("Error: {}", err)
                } else {
                    format!("Graph '{}' created successfully.", params.name)
                }
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Create a point-in-time backup of the entire database.
    /// The backup is a RocksDB checkpoint — near-instant, consistent snapshot.
    #[tool(name = "create_backup")]
    async fn create_backup(&self, Parameters(params): Parameters<BackupParams>) -> String {
        let label = params.label.unwrap_or_else(|| "manual".to_string());
        match self.backend.create_backup(&label).await {
            Ok(msg) => msg,
            Err(e) => format!("Error: {}", e),
        }
    }

    /// List all available database backups with their labels, timestamps, and sizes.
    #[tool(name = "list_backups")]
    async fn list_backups(&self) -> String {
        match self.backend.list_backups().await {
            Ok(backups) => {
                if backups.is_empty() {
                    return "No backups found.".to_string();
                }
                let mut out = String::from("Backups:\n");
                for b in &backups {
                    out.push_str(&format!(
                        "  {} (label: {}, created: {}, size: {} bytes)\n",
                        b.name, b.label, b.created_at, b.size_bytes
                    ));
                }
                out
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Delete a specific backup by name.
    #[tool(name = "delete_backup")]
    async fn delete_backup(&self, Parameters(params): Parameters<DeleteBackupParams>) -> String {
        match self.backend.delete_backup(&params.name).await {
            Ok(msg) => msg,
            Err(e) => format!("Error: {}", e),
        }
    }
}

#[tool_handler]
impl ServerHandler for SynapticaMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.protocol_version = ProtocolVersion::V_2025_03_26;
        info.instructions = Some(
            "Synaptica is a high-performance graph database implementing the GQL standard (ISO/IEC 39075:2024). \
             Use the 'query' tool to execute GQL queries. Use 'get_schema' to discover the graph structure first. \
             Supported operations: MATCH (read), INSERT (create), SET (update), DELETE (remove), \
             CREATE/DROP INDEX (indexing). Edge creation requires MATCH first: \
             MATCH (a:Label1), (b:Label2) INSERT (a)-[:REL]->(b). \
             Multi-graph: Use 'list_graphs' to see available graphs, 'create_graph' to create new ones, \
             and pass the 'graph' parameter to any tool to target a specific graph."
                .to_string(),
        );
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }
}

fn format_query_result(result: &crate::backend::QueryResult) -> String {
    let mut out = String::new();

    if !result.columns.is_empty() {
        let mut widths: Vec<usize> = result.columns.iter().map(|c| c.len()).collect();
        for row in &result.rows {
            for (i, val) in row.iter().enumerate() {
                if i < widths.len() {
                    let s = format_json_value(val);
                    widths[i] = widths[i].max(s.len().min(60));
                }
            }
        }

        for (i, col) in result.columns.iter().enumerate() {
            if i > 0 {
                out.push_str(" | ");
            }
            out.push_str(&format!("{:width$}", col, width = widths[i]));
        }
        out.push('\n');

        for (i, w) in widths.iter().enumerate() {
            if i > 0 {
                out.push_str("-+-");
            }
            out.push_str(&"-".repeat(*w));
        }
        out.push('\n');

        for row in &result.rows {
            for (i, val) in row.iter().enumerate() {
                if i > 0 {
                    out.push_str(" | ");
                }
                let s = format_json_value(val);
                let width = if i < widths.len() { widths[i] } else { 0 };
                out.push_str(&format!("{:width$}", s, width = width));
            }
            out.push('\n');
        }
    }

    out.push_str(&format!(
        "\n{} row(s) returned in {:.1}ms",
        result.stats.rows_returned, result.stats.execution_time_ms
    ));
    out
}

fn format_json_value(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(format_json_value).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(obj) => {
            if obj.get("_type").and_then(|v| v.as_str()) == Some("node") {
                let labels = obj
                    .get("labels")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str())
                            .map(|s| format!(":{}", s))
                            .collect::<Vec<_>>()
                            .join("")
                    })
                    .unwrap_or_default();
                let props = obj
                    .get("properties")
                    .and_then(|v| v.as_object())
                    .map(|m| {
                        m.iter()
                            .map(|(k, v)| format!("{}: {}", k, format_json_value(v)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                format!("({} {{{}}})", labels, props)
            } else if obj.get("_type").and_then(|v| v.as_str()) == Some("edge") {
                let label = obj.get("label").and_then(|v| v.as_str()).unwrap_or("");
                format!("-[:{}]->", label)
            } else {
                serde_json::to_string(obj).unwrap_or_default()
            }
        }
    }
}

fn format_mutation_stats(stats: &crate::backend::QueryStats) -> String {
    let mut parts = Vec::new();
    if stats.nodes_created > 0 {
        parts.push(format!("{} node(s) created", stats.nodes_created));
    }
    if stats.edges_created > 0 {
        parts.push(format!("{} edge(s) created", stats.edges_created));
    }
    if stats.nodes_deleted > 0 {
        parts.push(format!("{} node(s) deleted", stats.nodes_deleted));
    }
    if stats.edges_deleted > 0 {
        parts.push(format!("{} edge(s) deleted", stats.edges_deleted));
    }
    if stats.properties_set > 0 {
        parts.push(format!("{} property(ies) set", stats.properties_set));
    }
    if parts.is_empty() {
        "No changes.".to_string()
    } else {
        parts.join(", ") + "."
    }
}
