use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use synaptica_cluster::raft::{RaftRequest, SynapticaRaft};
use synaptica_cluster::state_machine::StateMachineApplier;
use synaptica_core::graph::GraphId;
use synaptica_core::types::Value;
use synaptica_exec::engine::ExecutionEngine;
use synaptica_gql::parser;
use synaptica_gql::planner::QueryPlanner;
use synaptica_storage::backup::BackupManager;
use synaptica_storage::engine::{StorageEngine, StorageError};
use synaptica_storage::index::{IndexDefinition, IndexEntityType};

use crate::metrics::{ACTIVE_CONNECTIONS, QUERIES_TOTAL, QUERY_DURATION, REGISTRY};

pub mod proto {
    tonic::include_proto!("synaptica.client.v1");
}

use proto::synaptica_service_server::SynapticaService;
use proto::{
    gql_value, BackupInfo, BeginTransactionRequest, BeginTransactionResponse, ClusterStatusRequest,
    ClusterStatusResponse, CommitTransactionRequest, CommitTransactionResponse,
    CreateBackupRequest, CreateBackupResponse, CreateIndexRequest, CreateIndexResponse,
    DeleteBackupRequest, DeleteBackupResponse, DropIndexRequest, DropIndexResponse, ExportChunk,
    ExportGraphRequest, GetMetricsRequest, GetMetricsResponse, GetSchemaRequest, GetSchemaResponse,
    GqlList, GqlMap, GqlValue, GraphInfo, HealthRequest, HealthResponse, ImportGraphRequest,
    ImportGraphResponse, IndexInfo, LabelInfo, LabelSchema, ListBackupsRequest,
    ListBackupsResponse, ListGraphsRequest, ListGraphsResponse, ListIndexesRequest,
    ListIndexesResponse, ListLabelsRequest, ListLabelsResponse, QueryRequest, QueryResponse,
    QueryStats, RollbackTransactionRequest, RollbackTransactionResponse, Row,
};

pub struct SynapticaServiceImpl {
    pub storage: Arc<StorageEngine>,
    pub default_graph_id: GraphId,
    pub data_dir: String,
    /// Raft instance for cluster mode; None for standalone.
    pub raft: Option<SynapticaRaft>,
    /// State machine applier for executing replicated mutations.
    pub applier: Option<Arc<StateMachineApplier>>,
    /// This node's ID in the cluster.
    pub node_id: Option<u64>,
}

impl SynapticaServiceImpl {
    /// Resolve a graph_name from a request to a GraphId.
    /// Falls back to the server's default graph if the name is empty.
    fn resolve_graph_id(&self, graph_name: &str) -> GraphId {
        if graph_name.is_empty() {
            self.default_graph_id
        } else {
            GraphId::from_name(graph_name)
        }
    }

    /// Get the display name for a graph_name request field.
    fn resolve_graph_name(&self, graph_name: &str) -> String {
        if graph_name.is_empty() {
            self.default_graph_id.to_string()
        } else {
            graph_name.to_string()
        }
    }
}

/// RAII guard for ACTIVE_CONNECTIONS gauge — decrements on drop.
struct ConnectionGuard;
impl ConnectionGuard {
    fn new() -> Self {
        ACTIVE_CONNECTIONS.inc();
        Self
    }
}
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        ACTIVE_CONNECTIONS.dec();
    }
}

#[tonic::async_trait]
impl SynapticaService for SynapticaServiceImpl {
    #[tracing::instrument(skip(self, request), fields(graph = ?self.default_graph_id))]
    async fn execute_query(
        &self,
        request: Request<QueryRequest>,
    ) -> Result<Response<QueryResponse>, Status> {
        let _conn_guard = ConnectionGuard::new();
        let req = request.into_inner();
        let start = std::time::Instant::now();
        let graph_name = self.resolve_graph_name(&req.graph_name);
        let graph_id = self.resolve_graph_id(&req.graph_name);

        tracing::info!(query = %req.query, graph = %graph_name, "executing query");

        // Parse to determine if this is a write query
        let program = match parser::parse(&req.query) {
            Ok(p) => p,
            Err(e) => {
                let elapsed = start.elapsed().as_secs_f64();
                QUERY_DURATION
                    .with_label_values(&[&graph_name])
                    .observe(elapsed);
                QUERIES_TOTAL.with_label_values(&["error"]).inc();
                tracing::error!(error = %e, elapsed_ms = %start.elapsed().as_millis(), "parse error");
                return Ok(Response::new(QueryResponse {
                    columns: vec![],
                    rows: vec![],
                    stats: None,
                    error: Some(format!("parse error: {}", e)),
                }));
            }
        };

        // In cluster mode, route writes through Raft consensus
        if let Some(ref raft) = self.raft {
            if is_write_query(&program) {
                return self
                    .execute_write_via_raft(raft, &req.query, &graph_name, start)
                    .await;
            }
        }

        // Read path (or standalone mode): execute locally
        self.execute_local(&program, &graph_name, &graph_id, start)
    }

    type ExecuteQueryStreamStream =
        tokio_stream::wrappers::ReceiverStream<Result<QueryResponse, Status>>;

    #[tracing::instrument(skip(self, _request))]
    async fn execute_query_stream(
        &self,
        _request: Request<QueryRequest>,
    ) -> Result<Response<Self::ExecuteQueryStreamStream>, Status> {
        Err(Status::unimplemented(
            "execute_query_stream not implemented",
        ))
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
        if let Some(ref raft) = self.raft {
            let metrics = raft.metrics().borrow().clone();
            let node_id = self.node_id.unwrap_or(0);
            let leader_id = metrics.current_leader.unwrap_or(0);
            let role = if metrics.current_leader == Some(node_id) {
                "leader"
            } else {
                "follower"
            };

            // Build node list from membership config
            let mut nodes = Vec::new();
            if let Some(joint) = metrics
                .membership_config
                .membership()
                .get_joint_config()
                .first()
            {
                for &nid in joint.iter() {
                    let node_role = if nid == leader_id {
                        "leader"
                    } else {
                        "follower"
                    };
                    nodes.push(proto::ClusterNode {
                        id: nid.to_string(),
                        address: String::new(),
                        role: node_role.to_string(),
                        is_healthy: true,
                    });
                }
            }

            Ok(Response::new(ClusterStatusResponse {
                node_id: node_id.to_string(),
                role: role.to_string(),
                leader_id: leader_id.to_string(),
                nodes,
                partition_count: 1,
            }))
        } else {
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
    }

    #[tracing::instrument(skip(self, request), fields(graph = ?self.default_graph_id))]
    async fn list_labels(
        &self,
        request: Request<ListLabelsRequest>,
    ) -> Result<Response<ListLabelsResponse>, Status> {
        let req = request.into_inner();
        let graph_id = self.resolve_graph_id(&req.graph_name);
        let graph_id = &graph_id;

        let nodes = self
            .storage
            .scan_nodes(graph_id)
            .map_err(|e| Status::internal(format!("storage error: {}", e)))?;

        let mut node_label_counts: HashMap<String, i64> = HashMap::new();
        let mut edge_label_counts: HashMap<String, i64> = HashMap::new();

        for node in &nodes {
            for label in &node.labels {
                *node_label_counts.entry(label.to_string()).or_default() += 1;
            }
            // Collect edge labels by scanning outgoing edges per node
            if let Ok(edges) = self.storage.get_outgoing_edges(graph_id, &node.id, None) {
                for edge in &edges {
                    *edge_label_counts.entry(edge.label.to_string()).or_default() += 1;
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
        let req = request.into_inner();
        let graph_id = self.resolve_graph_id(&req.graph_name);
        let graph_id = &graph_id;

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
                    .entry(label.to_string())
                    .or_insert_with(|| (BTreeSet::new(), 0));
                entry.1 += 1;
                for key in node.properties.keys() {
                    entry.0.insert(key.clone());
                }
            }
            if let Ok(edges) = self.storage.get_outgoing_edges(graph_id, &node.id, None) {
                for edge in &edges {
                    let entry = edge_schemas
                        .entry(edge.label.to_string())
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
        let req = request.into_inner();
        let graph_id = self.resolve_graph_id(&req.graph_name);
        let graph_id = &graph_id;

        let defs = self
            .storage
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
        let graph_id = self.resolve_graph_id(&req.graph_name);

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

        match self.storage.create_index(&def) {
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
        let graph_id = self.resolve_graph_id(&req.graph_name);
        let graph_id = &graph_id;

        match self.storage.drop_index(graph_id, &req.name) {
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

    #[tracing::instrument(skip(self, _request))]
    async fn list_graphs(
        &self,
        _request: Request<ListGraphsRequest>,
    ) -> Result<Response<ListGraphsResponse>, Status> {
        let metas = self
            .storage
            .list_graphs()
            .map_err(|e| Status::internal(format!("storage error: {}", e)))?;

        let graphs = metas
            .into_iter()
            .map(|m| GraphInfo {
                name: m.name,
                id: m.id.to_string(),
            })
            .collect();

        Ok(Response::new(ListGraphsResponse { graphs }))
    }

    async fn create_backup(
        &self,
        request: Request<CreateBackupRequest>,
    ) -> Result<Response<CreateBackupResponse>, Status> {
        let label = request.into_inner().label;
        let backup = BackupManager::new(&self.data_dir)
            .create(self.storage.as_ref(), &label)
            .map_err(|e| Status::internal(format!("backup failed: {}", e)))?;

        Ok(Response::new(CreateBackupResponse {
            backup_name: backup.name,
            created_at: backup.created_at,
        }))
    }

    async fn list_backups(
        &self,
        _request: Request<ListBackupsRequest>,
    ) -> Result<Response<ListBackupsResponse>, Status> {
        let backups = BackupManager::new(&self.data_dir)
            .list()
            .map_err(|e| Status::internal(format!("failed to list backups: {}", e)))?
            .into_iter()
            .map(|backup| BackupInfo {
                name: backup.name,
                label: backup.label,
                created_at: backup.created_at,
                size_bytes: backup.size_bytes,
            })
            .collect();

        Ok(Response::new(ListBackupsResponse { backups }))
    }

    async fn delete_backup(
        &self,
        request: Request<DeleteBackupRequest>,
    ) -> Result<Response<DeleteBackupResponse>, Status> {
        let name = request.into_inner().backup_name;

        match BackupManager::new(&self.data_dir).delete(&name) {
            Ok(()) => Ok(Response::new(DeleteBackupResponse {
                success: true,
                message: format!("backup '{}' deleted", name),
            })),
            Err(StorageError::NotFound(_)) => Ok(Response::new(DeleteBackupResponse {
                success: false,
                message: format!("backup '{}' not found", name),
            })),
            Err(e) => Err(Status::internal(format!("failed to delete backup: {}", e))),
        }
    }

    type ExportGraphStream = ReceiverStream<Result<ExportChunk, Status>>;

    async fn export_graph(
        &self,
        request: Request<ExportGraphRequest>,
    ) -> Result<Response<Self::ExportGraphStream>, Status> {
        let graph_name = request.into_inner().graph_name;
        let graph_id = self.resolve_graph_id(&graph_name);

        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let storage = self.storage.clone();

        tokio::task::spawn_blocking(move || {
            let mut buf = Vec::new();
            match storage.export_graph(&graph_id, &mut buf) {
                Ok(_) => {
                    let data = String::from_utf8_lossy(&buf).to_string();
                    // Send in chunks of ~64KB
                    for chunk in data.as_bytes().chunks(65536) {
                        let chunk_str = String::from_utf8_lossy(chunk).to_string();
                        let _ = tx.blocking_send(Ok(ExportChunk { data: chunk_str }));
                    }
                }
                Err(e) => {
                    let _ =
                        tx.blocking_send(Err(Status::internal(format!("export failed: {}", e))));
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn import_graph(
        &self,
        request: Request<ImportGraphRequest>,
    ) -> Result<Response<ImportGraphResponse>, Status> {
        let inner = request.into_inner();
        let graph_name = inner.graph_name;
        let gql_data = inner.gql_data;
        let graph_id = self.resolve_graph_id(&graph_name);

        let mut executed = 0i64;

        for line in gql_data.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") || line.starts_with("--") {
                continue;
            }

            match parser::parse(line) {
                Ok(program) => {
                    let planner = QueryPlanner::new();
                    match planner.plan(&program) {
                        Ok(plan) => {
                            let engine = ExecutionEngine::new(&self.storage);
                            match engine.execute_plan(&plan, &graph_id) {
                                Ok(_) => executed += 1,
                                Err(e) => {
                                    return Ok(Response::new(ImportGraphResponse {
                                        statements_executed: executed,
                                        error: Some(format!(
                                            "execution error at statement {}: {}",
                                            executed + 1,
                                            e
                                        )),
                                    }));
                                }
                            }
                        }
                        Err(e) => {
                            return Ok(Response::new(ImportGraphResponse {
                                statements_executed: executed,
                                error: Some(format!(
                                    "planning error at statement {}: {}",
                                    executed + 1,
                                    e
                                )),
                            }));
                        }
                    }
                }
                Err(e) => {
                    return Ok(Response::new(ImportGraphResponse {
                        statements_executed: executed,
                        error: Some(format!("parse error at statement {}: {}", executed + 1, e)),
                    }));
                }
            }
        }

        Ok(Response::new(ImportGraphResponse {
            statements_executed: executed,
            error: None,
        }))
    }
}

impl SynapticaServiceImpl {
    /// Execute a query locally (reads, or standalone mode writes).
    fn execute_local(
        &self,
        program: &synaptica_gql::ast::GqlProgram,
        graph_name: &str,
        graph_id: &GraphId,
        start: std::time::Instant,
    ) -> Result<Response<QueryResponse>, Status> {
        let planner = QueryPlanner::new();
        let plan = match planner.plan(program) {
            Ok(p) => p,
            Err(e) => {
                let elapsed = start.elapsed().as_secs_f64();
                QUERY_DURATION
                    .with_label_values(&[graph_name])
                    .observe(elapsed);
                QUERIES_TOTAL.with_label_values(&["error"]).inc();
                return Ok(Response::new(QueryResponse {
                    columns: vec![],
                    rows: vec![],
                    stats: None,
                    error: Some(format!("plan error: {}", e)),
                }));
            }
        };

        let engine = ExecutionEngine::new(&self.storage);
        let result_set = match engine.execute_plan(&plan, graph_id) {
            Ok(rs) => rs,
            Err(e) => {
                let elapsed = start.elapsed().as_secs_f64();
                QUERY_DURATION
                    .with_label_values(&[graph_name])
                    .observe(elapsed);
                QUERIES_TOTAL.with_label_values(&["error"]).inc();
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
            .with_label_values(&[graph_name])
            .observe(elapsed.as_secs_f64());
        QUERIES_TOTAL.with_label_values(&["success"]).inc();

        let elapsed_ms = std::cmp::max(1, elapsed.as_millis() as i64);
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

    /// Execute a write query through Raft consensus.
    async fn execute_write_via_raft(
        &self,
        raft: &SynapticaRaft,
        query: &str,
        graph_name: &str,
        start: std::time::Instant,
    ) -> Result<Response<QueryResponse>, Status> {
        let raft_req = RaftRequest::WriteQuery {
            query: query.to_string(),
            graph_name: graph_name.to_string(),
        };

        // Submit write to Raft — this replicates the entry to all nodes
        match raft.client_write(raft_req.clone()).await {
            Ok(_raft_resp) => {
                // Entry is committed. Apply to local state machine.
                if let Some(ref applier) = self.applier {
                    let result = applier.apply(&raft_req);

                    let elapsed = start.elapsed();
                    QUERY_DURATION
                        .with_label_values(&[graph_name])
                        .observe(elapsed.as_secs_f64());

                    if result.success {
                        QUERIES_TOTAL.with_label_values(&["success"]).inc();
                        Ok(Response::new(QueryResponse {
                            columns: vec!["result".to_string()],
                            rows: vec![],
                            stats: Some(QueryStats {
                                nodes_created: 0,
                                nodes_deleted: 0,
                                edges_created: 0,
                                edges_deleted: 0,
                                properties_set: 0,
                                rows_returned: result.rows_affected,
                                execution_time_ms: std::cmp::max(1, elapsed.as_millis() as i64),
                            }),
                            error: None,
                        }))
                    } else {
                        QUERIES_TOTAL.with_label_values(&["error"]).inc();
                        Ok(Response::new(QueryResponse {
                            columns: vec![],
                            rows: vec![],
                            stats: None,
                            error: result.error,
                        }))
                    }
                } else {
                    Err(Status::internal("no state machine applier configured"))
                }
            }
            Err(e) => {
                let elapsed = start.elapsed().as_secs_f64();
                QUERY_DURATION
                    .with_label_values(&[graph_name])
                    .observe(elapsed);
                QUERIES_TOTAL.with_label_values(&["error"]).inc();

                // If not leader, the error may contain leader info for redirect
                let err_msg = format!("raft error: {}", e);
                tracing::warn!(error = %err_msg, "write via raft failed");
                Ok(Response::new(QueryResponse {
                    columns: vec![],
                    rows: vec![],
                    stats: None,
                    error: Some(err_msg),
                }))
            }
        }
    }
}

/// Determine if a parsed GQL program contains write operations.
fn is_write_query(program: &synaptica_gql::ast::GqlProgram) -> bool {
    use synaptica_gql::ast::GqlStatement;
    program.statements.iter().any(|stmt| {
        matches!(
            stmt,
            GqlStatement::Insert(_)
                | GqlStatement::Set(_)
                | GqlStatement::Delete(_)
                | GqlStatement::Remove(_)
                | GqlStatement::CreateGraph(_)
                | GqlStatement::DropGraph(_)
                | GqlStatement::CreateGraphType(_)
                | GqlStatement::CreateIndex(_)
                | GqlStatement::DropIndex(_)
        )
    })
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
        Value::Node {
            id,
            labels,
            properties,
        } => {
            let proto_props = properties
                .iter()
                .map(|(k, v)| (k.clone(), value_to_proto(v)))
                .collect();
            Some(gql_value::Kind::NodeValue(proto::GqlNode {
                id: id.clone(),
                labels: labels.clone(),
                properties: proto_props,
            }))
        }
        Value::Edge {
            id,
            label,
            source_id,
            target_id,
            properties,
        } => {
            let proto_props = properties
                .iter()
                .map(|(k, v)| (k.clone(), value_to_proto(v)))
                .collect();
            Some(gql_value::Kind::EdgeValue(proto::GqlEdge {
                id: id.clone(),
                label: label.clone(),
                source_id: source_id.clone(),
                target_id: target_id.clone(),
                properties: proto_props,
            }))
        }
        // Date/Time/Timestamp/Duration → string representation
        _ => Some(gql_value::Kind::StringValue(format!("{}", value))),
    };
    GqlValue { kind }
}

/// Recursively compute the total size of a directory in bytes.
fn dir_size(path: &std::path::Path) -> std::io::Result<u64> {
    let mut total = 0;
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let p = entry.path();
            if p.is_dir() {
                total += dir_size(&p)?;
            } else {
                total += entry.metadata()?.len();
            }
        }
    }
    Ok(total)
}
