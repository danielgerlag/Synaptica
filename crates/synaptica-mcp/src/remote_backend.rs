use async_trait::async_trait;
use tonic::transport::Channel;

use crate::backend::*;

mod proto {
    tonic::include_proto!("synaptica.client.v1");
}

use proto::synaptica_service_client::SynapticaServiceClient;
use proto::{
    gql_value, ClusterStatusRequest, CreateBackupRequest, CreateIndexRequest, DeleteBackupRequest,
    DropIndexRequest, GetSchemaRequest, HealthRequest, ListBackupsRequest, ListGraphsRequest,
    ListIndexesRequest, ListLabelsRequest, QueryRequest,
};

/// Remote backend — connects to a running Synaptica server via gRPC.
pub struct RemoteBackend {
    client: SynapticaServiceClient<Channel>,
    default_graph: String,
}

impl RemoteBackend {
    pub async fn connect(server_url: &str, default_graph: &str) -> anyhow::Result<Self> {
        let url = if server_url.starts_with("http") {
            server_url.to_string()
        } else {
            format!("http://{}", server_url)
        };
        let client = SynapticaServiceClient::connect(url).await?;
        Ok(Self {
            client,
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

fn proto_value_to_json(val: &proto::GqlValue) -> serde_json::Value {
    match &val.kind {
        Some(gql_value::Kind::NullValue(_)) => serde_json::Value::Null,
        Some(gql_value::Kind::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(gql_value::Kind::IntegerValue(i)) => serde_json::json!(*i),
        Some(gql_value::Kind::FloatValue(f)) => serde_json::json!(*f),
        Some(gql_value::Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(gql_value::Kind::BytesValue(b)) => {
            serde_json::json!(format!("0x{}", hex::encode(b)))
        }
        Some(gql_value::Kind::ListValue(list)) => {
            serde_json::Value::Array(list.items.iter().map(proto_value_to_json).collect())
        }
        Some(gql_value::Kind::MapValue(map)) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .entries
                .iter()
                .map(|(k, v)| (k.clone(), proto_value_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        Some(gql_value::Kind::NodeValue(node)) => {
            let props: serde_json::Map<String, serde_json::Value> = node
                .properties
                .iter()
                .map(|(k, v)| (k.clone(), proto_value_to_json(v)))
                .collect();
            serde_json::json!({
                "_type": "node",
                "id": node.id,
                "labels": node.labels,
                "properties": props,
            })
        }
        Some(gql_value::Kind::EdgeValue(edge)) => {
            let props: serde_json::Map<String, serde_json::Value> = edge
                .properties
                .iter()
                .map(|(k, v)| (k.clone(), proto_value_to_json(v)))
                .collect();
            serde_json::json!({
                "_type": "edge",
                "id": edge.id,
                "label": edge.label,
                "source_id": edge.source_id,
                "target_id": edge.target_id,
                "properties": props,
            })
        }
        None => serde_json::Value::Null,
    }
}

#[async_trait]
impl SynapticaBackend for RemoteBackend {
    async fn execute_query(&self, query: &str, graph: &str) -> anyhow::Result<QueryResult> {
        let graph_name = self.resolve_graph(graph);
        let mut client = self.client.clone();

        let resp = client
            .execute_query(QueryRequest {
                query: query.to_string(),
                graph_name,
                parameters: Default::default(),
                transaction_id: None,
            })
            .await?
            .into_inner();

        if let Some(ref err) = resp.error {
            return Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                stats: QueryStats {
                    rows_returned: 0,
                    execution_time_ms: 0.0,
                    nodes_created: 0,
                    edges_created: 0,
                    nodes_deleted: 0,
                    edges_deleted: 0,
                    properties_set: 0,
                },
                error: Some(err.clone()),
            });
        }

        let columns = resp.columns.clone();
        let rows: Vec<Vec<serde_json::Value>> = resp
            .rows
            .iter()
            .map(|row| row.values.iter().map(proto_value_to_json).collect())
            .collect();

        let stats = resp.stats.as_ref();
        Ok(QueryResult {
            columns,
            rows: rows.clone(),
            stats: QueryStats {
                rows_returned: rows.len() as u64,
                execution_time_ms: stats.map(|s| s.execution_time_ms as f64).unwrap_or(0.0),
                nodes_created: stats.map(|s| s.nodes_created as u64).unwrap_or(0),
                edges_created: stats.map(|s| s.edges_created as u64).unwrap_or(0),
                nodes_deleted: stats.map(|s| s.nodes_deleted as u64).unwrap_or(0),
                edges_deleted: stats.map(|s| s.edges_deleted as u64).unwrap_or(0),
                properties_set: stats.map(|s| s.properties_set as u64).unwrap_or(0),
            },
            error: None,
        })
    }

    async fn get_schema(&self, graph: &str) -> anyhow::Result<SchemaInfo> {
        let graph_name = self.resolve_graph(graph);
        let mut client = self.client.clone();

        let resp = client
            .get_schema(GetSchemaRequest { graph_name })
            .await?
            .into_inner();

        Ok(SchemaInfo {
            node_labels: resp
                .node_schemas
                .iter()
                .map(|s| LabelSchema {
                    label: s.label.clone(),
                    property_keys: s.property_keys.clone(),
                    count: s.count as u64,
                })
                .collect(),
            edge_labels: resp
                .edge_schemas
                .iter()
                .map(|s| LabelSchema {
                    label: s.label.clone(),
                    property_keys: s.property_keys.clone(),
                    count: s.count as u64,
                })
                .collect(),
        })
    }

    async fn list_labels(&self, graph: &str) -> anyhow::Result<Vec<LabelInfo>> {
        let graph_name = self.resolve_graph(graph);
        let mut client = self.client.clone();

        let resp = client
            .list_labels(ListLabelsRequest { graph_name })
            .await?
            .into_inner();

        let mut labels = Vec::new();
        for l in &resp.node_labels {
            labels.push(LabelInfo {
                name: l.name.clone(),
                count: l.count as u64,
                kind: "node".to_string(),
            });
        }
        for l in &resp.edge_labels {
            labels.push(LabelInfo {
                name: l.name.clone(),
                count: l.count as u64,
                kind: "edge".to_string(),
            });
        }
        Ok(labels)
    }

    async fn list_indexes(&self, graph: &str) -> anyhow::Result<Vec<IndexInfo>> {
        let graph_name = self.resolve_graph(graph);
        let mut client = self.client.clone();

        let resp = client
            .list_indexes(ListIndexesRequest { graph_name })
            .await?
            .into_inner();

        Ok(resp
            .indexes
            .iter()
            .map(|i| IndexInfo {
                label: i.name.clone(),
                property: i.property_names.join(", "),
            })
            .collect())
    }

    async fn create_index(
        &self,
        label: &str,
        property: &str,
        graph: &str,
    ) -> anyhow::Result<String> {
        let graph_name = self.resolve_graph(graph);
        let mut client = self.client.clone();

        let resp = client
            .create_index(CreateIndexRequest {
                graph_name,
                name: format!("idx_{}_{}", label, property),
                entity_type: "node".to_string(),
                property_names: vec![property.to_string()],
                is_unique: false,
            })
            .await?
            .into_inner();

        if resp.success {
            Ok(format!("Index created on :{}({})", label, property))
        } else {
            anyhow::bail!("Failed to create index: {}", resp.error.unwrap_or_default())
        }
    }

    async fn drop_index(&self, label: &str, property: &str, graph: &str) -> anyhow::Result<String> {
        let graph_name = self.resolve_graph(graph);
        let mut client = self.client.clone();

        let resp = client
            .drop_index(DropIndexRequest {
                graph_name,
                name: format!("idx_{}_{}", label, property),
            })
            .await?
            .into_inner();

        if resp.success {
            Ok(format!("Index dropped on :{}({})", label, property))
        } else {
            anyhow::bail!("Failed to drop index: {}", resp.error.unwrap_or_default())
        }
    }

    async fn health(&self) -> anyhow::Result<HealthInfo> {
        let mut client = self.client.clone();
        let resp = client.health(HealthRequest {}).await?.into_inner();

        Ok(HealthInfo {
            status: resp.status,
            version: resp.version,
        })
    }

    async fn cluster_status(&self) -> anyhow::Result<Option<ClusterInfo>> {
        let mut client = self.client.clone();
        let resp = client
            .cluster_status(ClusterStatusRequest {})
            .await?
            .into_inner();

        Ok(Some(ClusterInfo {
            node_id: resp.node_id,
            role: resp.role,
            leader_id: resp.leader_id,
            nodes: resp
                .nodes
                .iter()
                .map(|n| ClusterNodeInfo {
                    node_id: n.id.clone(),
                    address: n.address.clone(),
                    role: n.role.clone(),
                })
                .collect(),
        }))
    }

    async fn list_graphs(&self) -> anyhow::Result<Vec<GraphSummary>> {
        let mut client = self.client.clone();
        let resp = client.list_graphs(ListGraphsRequest {}).await?.into_inner();

        Ok(resp
            .graphs
            .into_iter()
            .map(|g| GraphSummary {
                name: g.name,
                id: g.id,
            })
            .collect())
    }

    async fn create_backup(&self, label: &str) -> anyhow::Result<String> {
        let mut client = self.client.clone();
        let resp = client
            .create_backup(CreateBackupRequest {
                label: label.to_string(),
            })
            .await?
            .into_inner();
        Ok(format!(
            "Backup '{}' created at {}",
            resp.backup_name, resp.created_at
        ))
    }

    async fn list_backups(&self) -> anyhow::Result<Vec<BackupSummary>> {
        let mut client = self.client.clone();
        let resp = client
            .list_backups(ListBackupsRequest {})
            .await?
            .into_inner();
        Ok(resp
            .backups
            .into_iter()
            .map(|b| BackupSummary {
                name: b.name,
                label: b.label,
                created_at: b.created_at,
                size_bytes: b.size_bytes,
            })
            .collect())
    }

    async fn delete_backup(&self, name: &str) -> anyhow::Result<String> {
        let mut client = self.client.clone();
        let resp = client
            .delete_backup(DeleteBackupRequest {
                backup_name: name.to_string(),
            })
            .await?
            .into_inner();
        if resp.success {
            Ok(resp.message)
        } else {
            anyhow::bail!(resp.message)
        }
    }
}
