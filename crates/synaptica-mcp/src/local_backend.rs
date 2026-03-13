use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use synaptica_core::graph::{GraphId, GraphMeta};
use synaptica_core::types::Value;
use synaptica_exec::engine::ExecutionEngine;
use synaptica_gql::parser;
use synaptica_gql::planner::QueryPlanner;
use synaptica_storage::backup::BackupManager;
use synaptica_storage::engine::{StorageConfig, StorageEngine};

use crate::backend::*;

/// Embedded local backend — runs the full query engine in-process.
pub struct LocalBackend {
    storage: Arc<StorageEngine>,
    data_dir: String,
    default_graph: String,
}

impl LocalBackend {
    pub fn new(data_dir: &str, default_graph: &str) -> anyhow::Result<Self> {
        let storage = StorageEngine::open(data_dir, &StorageConfig::default())?;
        let storage = Arc::new(storage);

        let graph_id = GraphId::from_name(default_graph);
        let meta = GraphMeta {
            id: graph_id,
            name: default_graph.to_string(),
            graph_type: None,
        };
        let _ = storage.put_graph_meta(&meta);

        Ok(Self {
            storage,
            data_dir: data_dir.to_string(),
            default_graph: default_graph.to_string(),
        })
    }

    fn resolve_graph(&self, graph: &str) -> String {
        if graph.is_empty() {
            self.default_graph.clone()
        } else {
            graph.to_string()
        }
    }
}

fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Integer(i) => serde_json::json!(*i),
        Value::Float(f) => serde_json::json!(*f),
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Bytes(b) => serde_json::json!(format!("0x{}", hex::encode(b))),
        Value::List(items) => serde_json::Value::Array(items.iter().map(value_to_json).collect()),
        Value::Map(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), value_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        Value::Node {
            id,
            labels,
            properties,
        } => {
            let props: serde_json::Map<String, serde_json::Value> = properties
                .iter()
                .map(|(k, v)| (k.clone(), value_to_json(v)))
                .collect();
            serde_json::json!({
                "_type": "node",
                "id": id,
                "labels": labels,
                "properties": props,
            })
        }
        Value::Edge {
            id,
            label,
            source_id,
            target_id,
            properties,
        } => {
            let props: serde_json::Map<String, serde_json::Value> = properties
                .iter()
                .map(|(k, v)| (k.clone(), value_to_json(v)))
                .collect();
            serde_json::json!({
                "_type": "edge",
                "id": id,
                "label": label,
                "source_id": source_id,
                "target_id": target_id,
                "properties": props,
            })
        }
        _ => serde_json::Value::String(format!("{}", value)),
    }
}

#[async_trait]
impl SynapticaBackend for LocalBackend {
    async fn execute_query(&self, query: &str, graph: &str) -> anyhow::Result<QueryResult> {
        let graph_name = self.resolve_graph(graph);
        let graph_id = GraphId::from_name(&graph_name);
        let start = std::time::Instant::now();

        let program =
            parser::parse(query).map_err(|e| anyhow::anyhow!("Parse error: {}", e.message))?;

        let planner = QueryPlanner;
        let plan = planner
            .plan(&program)
            .map_err(|e| anyhow::anyhow!("Plan error: {}", e))?;

        let engine = ExecutionEngine::new(&self.storage);
        let result_set = engine
            .execute_plan(&plan, &graph_id)
            .map_err(|e| anyhow::anyhow!("Execution error: {}", e))?;

        let elapsed = start.elapsed().as_secs_f64() * 1000.0;

        let rows: Vec<Vec<serde_json::Value>> = result_set
            .records
            .iter()
            .map(|record| record.values.iter().map(value_to_json).collect())
            .collect();

        Ok(QueryResult {
            columns: result_set.columns.clone(),
            rows: rows.clone(),
            stats: QueryStats {
                rows_returned: rows.len() as u64,
                execution_time_ms: elapsed,
                nodes_created: 0,
                edges_created: 0,
                nodes_deleted: 0,
                edges_deleted: 0,
                properties_set: 0,
            },
            error: None,
        })
    }

    async fn get_schema(&self, graph: &str) -> anyhow::Result<SchemaInfo> {
        let graph_name = self.resolve_graph(graph);
        let graph_id = GraphId::from_name(&graph_name);

        let nodes = self.storage.scan_nodes(&graph_id)?;
        let mut node_schemas: BTreeMap<String, (Vec<String>, u64)> = BTreeMap::new();
        let mut edge_schemas: BTreeMap<String, (Vec<String>, u64)> = BTreeMap::new();

        for node in &nodes {
            for label in &node.labels {
                let entry = node_schemas
                    .entry(label.to_string())
                    .or_insert_with(|| (Vec::new(), 0));
                entry.1 += 1;
                for key in node.properties.keys() {
                    if !entry.0.contains(key) {
                        entry.0.push(key.clone());
                    }
                }
            }

            let edges = self.storage.get_outgoing_edges(&graph_id, &node.id, None)?;
            for edge in &edges {
                let entry = edge_schemas
                    .entry(edge.label.to_string())
                    .or_insert_with(|| (Vec::new(), 0));
                entry.1 += 1;
                for key in edge.properties.keys() {
                    if !entry.0.contains(key) {
                        entry.0.push(key.clone());
                    }
                }
            }
        }

        Ok(SchemaInfo {
            node_labels: node_schemas
                .into_iter()
                .map(|(label, (keys, count))| LabelSchema {
                    label,
                    property_keys: keys,
                    count,
                })
                .collect(),
            edge_labels: edge_schemas
                .into_iter()
                .map(|(label, (keys, count))| LabelSchema {
                    label,
                    property_keys: keys,
                    count,
                })
                .collect(),
        })
    }

    async fn list_labels(&self, graph: &str) -> anyhow::Result<Vec<LabelInfo>> {
        let schema = self.get_schema(graph).await?;
        let mut labels = Vec::new();
        for ls in schema.node_labels {
            labels.push(LabelInfo {
                name: ls.label,
                count: ls.count,
                kind: "node".to_string(),
            });
        }
        for ls in schema.edge_labels {
            labels.push(LabelInfo {
                name: ls.label,
                count: ls.count,
                kind: "edge".to_string(),
            });
        }
        Ok(labels)
    }

    async fn list_indexes(&self, graph: &str) -> anyhow::Result<Vec<IndexInfo>> {
        let graph_name = self.resolve_graph(graph);
        let graph_id = GraphId::from_name(&graph_name);
        let defs = self.storage.list_indexes(&graph_id)?;
        Ok(defs
            .into_iter()
            .map(|d| {
                // Index names follow convention: idx_{label}_{property}
                let parts: Vec<&str> = d.name.splitn(3, '_').collect();
                let label = if parts.len() >= 2 {
                    parts[1].to_string()
                } else {
                    d.name.clone()
                };
                IndexInfo {
                    label,
                    property: d.property_names.join(", "),
                }
            })
            .collect())
    }

    async fn create_index(
        &self,
        label: &str,
        property: &str,
        graph: &str,
    ) -> anyhow::Result<String> {
        let idx_name = format!("idx_{}_{}", label, property);
        let query = format!(
            "CREATE INDEX {} FOR (v:{}) ON (v.{})",
            idx_name, label, property
        );
        let result = self.execute_query(&query, graph).await?;
        if let Some(err) = result.error {
            anyhow::bail!(err);
        }
        Ok(format!("Index created on :{}({})", label, property))
    }

    async fn drop_index(&self, label: &str, property: &str, graph: &str) -> anyhow::Result<String> {
        let idx_name = format!("idx_{}_{}", label, property);
        let query = format!("DROP INDEX {}", idx_name);
        let result = self.execute_query(&query, graph).await?;
        if let Some(err) = result.error {
            anyhow::bail!(err);
        }
        Ok(format!("Index dropped on :{}({})", label, property))
    }

    async fn health(&self) -> anyhow::Result<HealthInfo> {
        Ok(HealthInfo {
            status: "ok".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        })
    }

    async fn cluster_status(&self) -> anyhow::Result<Option<ClusterInfo>> {
        Ok(None) // local mode has no cluster
    }

    async fn list_graphs(&self) -> anyhow::Result<Vec<GraphSummary>> {
        let metas = self.storage.list_graphs()?;
        Ok(metas
            .into_iter()
            .map(|m| GraphSummary {
                name: m.name,
                id: m.id.to_string(),
            })
            .collect())
    }

    async fn create_backup(&self, label: &str) -> anyhow::Result<String> {
        let backup = BackupManager::new(&self.data_dir).create(self.storage.as_ref(), label)?;
        Ok(format!("Backup '{}' created", backup.name))
    }

    async fn list_backups(&self) -> anyhow::Result<Vec<BackupSummary>> {
        Ok(BackupManager::new(&self.data_dir)
            .list()?
            .into_iter()
            .map(|backup| BackupSummary {
                name: backup.name,
                label: backup.label,
                created_at: backup.created_at,
                size_bytes: backup.size_bytes,
            })
            .collect())
    }

    async fn delete_backup(&self, name: &str) -> anyhow::Result<String> {
        BackupManager::new(&self.data_dir).delete(name)?;
        Ok(format!("Backup '{}' deleted", name))
    }
}
