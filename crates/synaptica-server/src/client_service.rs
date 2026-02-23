use std::sync::Arc;

use tonic::{Request, Response, Status};

use synaptica_core::graph::GraphId;
use synaptica_core::types::Value;
use synaptica_exec::engine::ExecutionEngine;
use synaptica_gql::parser;
use synaptica_gql::planner::QueryPlanner;
use synaptica_storage::engine::StorageEngine;

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
    async fn execute_query(
        &self,
        request: Request<QueryRequest>,
    ) -> Result<Response<QueryResponse>, Status> {
        let req = request.into_inner();
        let start = std::time::Instant::now();

        // Parse
        let program = match parser::parse(&req.query) {
            Ok(p) => p,
            Err(e) => {
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
                return Ok(Response::new(QueryResponse {
                    columns: vec![],
                    rows: vec![],
                    stats: None,
                    error: Some(format!("execution error: {}", e)),
                }));
            }
        };

        let elapsed_ms = start.elapsed().as_millis() as i64;
        let rows_returned = result_set.records.len() as i64;

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

    async fn execute_query_stream(
        &self,
        _request: Request<QueryRequest>,
    ) -> Result<Response<Self::ExecuteQueryStreamStream>, Status> {
        Err(Status::unimplemented("execute_query_stream not implemented"))
    }

    async fn begin_transaction(
        &self,
        _request: Request<BeginTransactionRequest>,
    ) -> Result<Response<BeginTransactionResponse>, Status> {
        Err(Status::unimplemented("begin_transaction not implemented"))
    }

    async fn commit_transaction(
        &self,
        _request: Request<CommitTransactionRequest>,
    ) -> Result<Response<CommitTransactionResponse>, Status> {
        Err(Status::unimplemented("commit_transaction not implemented"))
    }

    async fn rollback_transaction(
        &self,
        _request: Request<RollbackTransactionRequest>,
    ) -> Result<Response<RollbackTransactionResponse>, Status> {
        Err(Status::unimplemented(
            "rollback_transaction not implemented",
        ))
    }

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