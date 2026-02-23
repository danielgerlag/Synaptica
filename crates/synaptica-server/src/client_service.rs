use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use tonic::{Request, Response, Status};

use synaptica_core::graph::GraphId;
use synaptica_core::types::Value;
use synaptica_exec::engine::ExecutionEngine;
use synaptica_gql::parser;
use synaptica_gql::planner::QueryPlanner;
use synaptica_storage::engine::StorageEngine;
use synaptica_storage::index::{IndexDefinition, IndexEntityType, IndexManager};

use crate::metrics::{ACTIVE_CONNECTIONS, QUERIES_TOTAL, QUERY_DURATION, REGISTRY};

pub mod proto {
    tonic::include_proto!("synaptica.client.v1");
}

use proto::synaptica_service_server::SynapticaService;
use proto::{
    gql_value, BeginTransactionRequest, BeginTransactionResponse, ClusterStatusRequest,
    ClusterStatusResponse, CommitTransactionRequest, CommitTransactionResponse,
    CreateIndexRequest, CreateIndexResponse, DropIndexRequest, DropIndexResponse,
    GetMetricsRequest, GetMetricsResponse, GetSchemaRequest, GetSchemaResponse, GqlList,
    GqlMap, GqlValue, HealthRequest, HealthResponse, IndexInfo, LabelInfo, LabelSchema,
    ListIndexesRequest, ListIndexesResponse, ListLabelsRequest, ListLabelsResponse,
    QueryRequest, QueryResponse, QueryStats, RollbackTransactionRequest,
    RollbackTransactionResponse, Row,
};

pub struct SynapticaServiceImpl {
    pub storage: Arc<StorageEngine>,
    pub default_graph_id: GraphId,
}

#[tonic::async_trait]
impl SynapticaService for SynapticaServiceImpl {
    #[tracing::instrument(skip(self, request), fields(graph = ?self.default_graph_id))]
    async fn execute_query(
        &self,
        request: Request<QueryRequest>,
    ) -> Result<Response<QueryResponse>, Status> {
        ACTIVE_CONNECTIONS.inc();
        let req = request.into_inner();
        let start = std::time::Instant::now();
        let graph_name = self.default_graph_id.0.to_string();

        tracing::info!(query = %req.query, graph = %graph_name, "executing query");

        // Parse
        let program = match parser::parse(&req.query) {
            Ok(p) => p,
            Err(e) => {
                let elapsed = start.elapsed().as_secs_f64();
                QUERY_DURATION.with_label_values(&[&graph_name]).observe(elapsed);
                QUERIES_TOTAL.with_label_values(&["error"]).inc();
                ACTIVE_CONNECTIONS.dec();
                tracing::error!(error = %e, elapsed_ms = %start.elapsed().as_millis(), "parse error");
                return Ok(Response::new(QueryResponse {
                    columns: vec![],
                    rows: vec![],
                    stats: None,
                    error: Some(format!("parse error: {}", e)),
                }));
            }
        };

        // Plan
        let planner = QueryPlanner::new();
        let plan = match planner.plan(&program) {
            Ok(p) => p,
            Err(e) => {
                let elapsed = start.elapsed().as_secs_f64();
                QUERY_DURATION.with_label_values(&[&graph_name]).observe(elapsed);
                QUERIES_TOTAL.with_label_values(&["error"]).inc();
                ACTIVE_CONNECTIONS.dec();
                tracing::error!(error = %e, elapsed_ms = %start.elapsed().as_millis(), "plan error");
                return Ok(Response::new(QueryResponse {
                    columns: vec![],
                    rows: vec![],
                    stats: None,
                    error: Some(format!("plan error: {}", e)),
                }));
            }
        };

        // Execute
        let engine = ExecutionEngine::new(&self.storage);
        let result_set = match engine.execute_plan(&plan, &self.default_graph_id) {
            Ok(rs) => rs,
            Err(e) => {
                let elapsed = start.elapsed().as_secs_f64();
                QUERY_DURATION.with_label_values(&[&graph_name]).observe(elapsed);
                QUERIES_TOTAL.with_label_values(&["error"]).inc();
                ACTIVE_CONNECTIONS.dec();
                tracing::error!(error = %e, elapsed_ms = %start.elapsed().as_millis(), "execution error");
                return Ok(Response::new(QueryResponse {
                    columns: vec![],
                    rows: vec![],
                    stats: None,
                    error: Some(format!("execution error: {}", e)),
                }));
            }
        };

        let elapsed = start.elapsed();
        QUERY_DURATION
            .with_label_values(&[&graph_name])
            .observe(elapsed.as_secs_f64());
        QUERIES_TOTAL.with_label_values(&["success"]).inc();
        ACTIVE_CONNECTIONS.dec();

        let elapsed_ms = elapsed.as_millis() as i64;
        let rows_returned = result_set.records.len() as i64;

        tracing::info!(
            rows = rows_returned,
            elapsed_ms = elapsed_ms,
            "query completed"
        );

        let columns = result_set.columns.clone();
        let rows: Vec<Row> = result_set
            .records
            .iter()
            .map(|record| Row {
                values: record.values.iter().map(value_to_proto).collect(),
            })
            .collect();

        let stats = QueryStats {
            nodes_created: 0,
            nodes_deleted: 0,
            edges_created: 0,
            edges_deleted: 0,
            properties_set: 0,
            rows_returned,
            execution_time_ms: elapsed_ms,
        };

        Ok(Response::new(QueryResponse {
            columns,
            rows,
            stats: Some(stats),
            error: None,
        }))
    }

    type ExecuteQueryStreamStream =
        tokio_stream::wrappers::ReceiverStream<Result<QueryResponse, Status>>;

    #[tracing::instrument(skip(self, _request))]
    async fn execute_query_stream(
        &self,
        _request: Request<QueryRequest>,
    ) -> Result<Response<Self::ExecuteQueryStreamStream>, Status> {
        Err(Status::unimplemented("execute_query_stream not implemented"))
    }

    #[tracing::instrument(skip(self, _request))]
    async fn begin_transaction(
        &self,
        _request: Request<BeginTransactionRequest>,
    ) -> Result<Response<BeginTransactionResponse>, Status> {
        Err(Status::unimplemented("begin_transaction not implemented"))
    }

    #[tracing::instrument(skip(self, _request))]
    async fn commit_transaction(
        &self,
        _request: Request<CommitTransactionRequest>,
    ) -> Result<Response<CommitTransactionResponse>, Status> {
        Err(Status::unimplemented("commit_transaction not implemented"))
    }

    #[tracing::instrument(skip(self, _request))]
    async fn rollback_transaction(
        &self,
        _request: Request<RollbackTransactionRequest>,
    ) -> Result<Response<RollbackTransactionResponse>, Status> {
        Err(Status::unimplemented(
            "rollback_transaction not implemented",
        ))
    }

    #[tracing::instrument(skip(self, _request))]
    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        Ok(Response::new(HealthResponse {
            status: "ok".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_seconds: 0,
        }))
    }

    #[tracing::instrument(skip(self, _request))]
    async fn cluster_status(
        &self,
        _request: Request<ClusterStatusRequest>,
    ) -> Result<Response<ClusterStatusResponse>, Status> {
        Ok(Response::new(ClusterStatusResponse {
            node_id: "standalone".to_string(),
            role: "leader".to_string(),
            leader_id: "standalone".to_string(),
            nodes: vec![proto::ClusterNode {
                id: "standalone".to_string(),
                address: "localhost".to_string(),
                role: "leader".to_string(),
                is_healthy: true,
            }],
            partition_count: 1,
        }))
    }

    #[tracing::instrument(skip(self, request), fields(graph = ?self.default_graph_id))]
    async fn list_labels(
        &self,
        request: Request<ListLabelsRequest>,
    ) -> Result<Response<ListLabelsResponse>, Status> {
        let _graph_name = request.into_inner().graph_name;
        let graph_id = &self.default_graph_id;

        let nodes = self
            .storage
            .scan_nodes(graph_id)
            .map_err(|e| Status::internal(format!("storage error: {}", e)))?;

        let mut node_label_counts: HashMap<String, i64> = HashMap::new();
        let mut edge_label_counts: HashMap<String, i64> = HashMap::new();

        for node in &nodes {
            for label in &node.labels {
                *node_label_counts.entry(label.0.clone()).or_default() += 1;
            }
            // Collect edge labels by scanning outgoing edges per node
            if let Ok(edges) = self.storage.get_outgoing_edges(graph_id, &node.id, None) {
                for edge in &edges {
                    *edge_label_counts.entry(edge.label.0.clone()).or_default() += 1;
                }
            }
        }

        let node_labels = node_label_counts
            .into_iter()
            .map(|(name, count)| LabelInfo { name, count })
            .collect();
        let edge_labels = edge_label_counts
            .into_iter()
            .map(|(name, count)| LabelInfo { name, count })
            .collect();

        Ok(Response::new(ListLabelsResponse {
            node_labels,
            edge_labels,
        }))
    }

    #[tracing::instrument(skip(self, request), fields(graph = ?self.default_graph_id))]
    async fn get_schema(
        &self,
        request: Request<GetSchemaRequest>,
    ) -> Result<Response<GetSchemaResponse>, Status> {
        let _graph_name = request.into_inner().graph_name;
        let graph_id = &self.default_graph_id;

        let nodes = self
            .storage
            .scan_nodes(graph_id)
            .map_err(|e| Status::internal(format!("storage error: {}", e)))?;

        // label -> (property_keys set, count)
        let mut node_schemas: HashMap<String, (BTreeSet<String>, i64)> = HashMap::new();
        let mut edge_schemas: HashMap<String, (BTreeSet<String>, i64)> = HashMap::new();

        for node in &nodes {
            for label in &node.labels {
                let entry = node_schemas
                    .entry(label.0.clone())
                    .or_insert_with(|| (BTreeSet::new(), 0));
                entry.1 += 1;
                for key in node.properties.keys() {
                    entry.0.insert(key.clone());
                }
            }
            if let Ok(edges) = self.storage.get_outgoing_edges(graph_id, &node.id, None) {
                for edge in &edges {
                    let entry = edge_schemas
                        .entry(edge.label.0.clone())
                        .or_insert_with(|| (BTreeSet::new(), 0));
                    entry.1 += 1;
                    for key in edge.properties.keys() {
                        entry.0.insert(key.clone());
                    }
                }
            }
        }

        let node_schemas = node_schemas
            .into_iter()
            .map(|(label, (keys, count))| LabelSchema {
                label,
                property_keys: keys.into_iter().collect(),
                count,
            })
            .collect();
        let edge_schemas = edge_schemas
            .into_iter()
            .map(|(label, (keys, count))| LabelSchema {
                label,
                property_keys: keys.into_iter().collect(),
                count,
            })
            .collect();

        Ok(Response::new(GetSchemaResponse {
            node_schemas,
            edge_schemas,
        }))
    }

    #[tracing::instrument(skip(self, request), fields(graph = ?self.default_graph_id))]
    async fn list_indexes(
        &self,
        request: Request<ListIndexesRequest>,
    ) -> Result<Response<ListIndexesResponse>, Status> {
        let _graph_name = request.into_inner().graph_name;
        let graph_id = &self.default_graph_id;

        let mgr = IndexManager::new(self.storage.raw_db().clone());
        let defs = mgr
            .list_indexes(graph_id)
            .map_err(|e| Status::internal(format!("storage error: {}", e)))?;

        let indexes = defs
            .into_iter()
            .map(|d| IndexInfo {
                name: d.name,
                entity_type: match d.entity_type {
                    IndexEntityType::Node => "node".to_string(),
                    IndexEntityType::Edge => "edge".to_string(),
                },
                property_names: d.property_names,
                is_unique: d.unique,
            })
            .collect();

        Ok(Response::new(ListIndexesResponse { indexes }))
    }

    #[tracing::instrument(skip(self, request), fields(graph = ?self.default_graph_id))]
    async fn create_index(
        &self,
        request: Request<CreateIndexRequest>,
    ) -> Result<Response<CreateIndexResponse>, Status> {
        let req = request.into_inner();
        let graph_id = self.default_graph_id;

        let entity_type = match req.entity_type.as_str() {
            "node" => IndexEntityType::Node,
            "edge" => IndexEntityType::Edge,
            other => {
                return Ok(Response::new(CreateIndexResponse {
                    success: false,
                    error: Some(format!("unknown entity_type: {}", other)),
                }));
            }
        };

        let def = IndexDefinition {
            name: req.name,
            graph_id,
            entity_type,
            property_names: req.property_names,
            unique: req.is_unique,
        };

        let mgr = IndexManager::new(self.storage.raw_db().clone());
        match mgr.create_index(&def) {
            Ok(()) => Ok(Response::new(CreateIndexResponse {
                success: true,
                error: None,
            })),
            Err(e) => Ok(Response::new(CreateIndexResponse {
                success: false,
                error: Some(format!("{}", e)),
            })),
        }
    }

    #[tracing::instrument(skip(self, request), fields(graph = ?self.default_graph_id))]
    async fn drop_index(
        &self,
        request: Request<DropIndexRequest>,
    ) -> Result<Response<DropIndexResponse>, Status> {
        let req = request.into_inner();
        let graph_id = &self.default_graph_id;

        let mgr = IndexManager::new(self.storage.raw_db().clone());
        match mgr.drop_index(graph_id, &req.name) {
            Ok(()) => Ok(Response::new(DropIndexResponse {
                success: true,
                error: None,
            })),
            Err(e) => Ok(Response::new(DropIndexResponse {
                success: false,
                error: Some(format!("{}", e)),
            })),
        }
    }

    #[tracing::instrument(skip(self, _request))]
    async fn get_metrics(
        &self,
        _request: Request<GetMetricsRequest>,
    ) -> Result<Response<GetMetricsResponse>, Status> {
        let metric_families = REGISTRY.gather();
        let mut gauges: HashMap<String, f64> = HashMap::new();
        let mut counters: HashMap<String, i64> = HashMap::new();

        for mf in &metric_families {
            let name = mf.get_name();
            for m in mf.get_metric() {
                let label_suffix: String = m
                    .get_label()
                    .iter()
                    .map(|l| format!("{}={}", l.get_name(), l.get_value()))
                    .collect::<Vec<_>>()
                    .join(",");
                let full_name = if label_suffix.is_empty() {
                    name.to_string()
                } else {
                    format!("{}[{}]", name, label_suffix)
                };

                if m.has_gauge() {
                    gauges.insert(full_name, m.get_gauge().get_value());
                } else if m.has_counter() {
                    counters.insert(full_name, m.get_counter().get_value() as i64);
                }
            }
        }

        Ok(Response::new(GetMetricsResponse { gauges, counters }))
    }
}

fn value_to_proto(value: &Value) -> GqlValue {
    let kind = match value {
        Value::Null => Some(gql_value::Kind::NullValue(true)),
        Value::Bool(b) => Some(gql_value::Kind::BoolValue(*b)),
        Value::Integer(i) => Some(gql_value::Kind::IntegerValue(*i)),
        Value::Float(f) => Some(gql_value::Kind::FloatValue(*f)),
        Value::String(s) => Some(gql_value::Kind::StringValue(s.clone())),
        Value::Bytes(b) => Some(gql_value::Kind::BytesValue(b.clone())),
        Value::List(items) => {
            let list = GqlList {
                items: items.iter().map(value_to_proto).collect(),
            };
            Some(gql_value::Kind::ListValue(list))
        }
        Value::Map(map) => {
            let entries = map
                .iter()
                .map(|(k, v)| (k.clone(), value_to_proto(v)))
                .collect();
            Some(gql_value::Kind::MapValue(GqlMap { entries }))
        }
        // Date/Time/Timestamp/Duration → string representation
        _ => Some(gql_value::Kind::StringValue(format!("{}", value))),
    };
    GqlValue { kind }
}