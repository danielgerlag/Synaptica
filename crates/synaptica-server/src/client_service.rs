use std::sync::Arc;

use tonic::{Request, Response, Status};

use synaptica_core::graph::GraphId;
use synaptica_core::types::Value;
use synaptica_exec::engine::ExecutionEngine;
use synaptica_gql::parser;
use synaptica_gql::planner::QueryPlanner;
use synaptica_storage::engine::StorageEngine;

use crate::metrics::{ACTIVE_CONNECTIONS, QUERIES_TOTAL, QUERY_DURATION};

pub mod proto {
    tonic::include_proto!("synaptica.client.v1");
}

use proto::synaptica_service_server::SynapticaService;
use proto::{
    gql_value, BeginTransactionRequest, BeginTransactionResponse, ClusterStatusRequest,
    ClusterStatusResponse, CommitTransactionRequest, CommitTransactionResponse, GqlList,
    GqlMap, GqlValue, HealthRequest, HealthResponse, QueryRequest, QueryResponse,
    QueryStats, RollbackTransactionRequest, RollbackTransactionResponse, Row,
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
        Err(Status::unimplemented("cluster_status not implemented"))
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