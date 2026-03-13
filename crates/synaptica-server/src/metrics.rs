use lazy_static::lazy_static;
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, IntGauge, Opts, Registry, TextEncoder,
};

lazy_static! {
    pub static ref REGISTRY: Registry = Registry::new();
    pub static ref QUERIES_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "synaptica_queries_total",
            "Total number of queries executed"
        ),
        &["status"],
    )
    .unwrap();
    pub static ref QUERY_DURATION: HistogramVec = HistogramVec::new(
        HistogramOpts::new(
            "synaptica_query_duration_seconds",
            "Query execution duration in seconds",
        ),
        &["graph"],
    )
    .unwrap();
    pub static ref ACTIVE_CONNECTIONS: IntGauge = IntGauge::new(
        "synaptica_active_connections",
        "Number of active connections"
    )
    .unwrap();
    pub static ref NODES_TOTAL: IntGauge = IntGauge::new(
        "synaptica_nodes_total",
        "Total number of nodes in the database"
    )
    .unwrap();
    pub static ref EDGES_TOTAL: IntGauge = IntGauge::new(
        "synaptica_edges_total",
        "Total number of edges in the database"
    )
    .unwrap();
}

pub fn register_metrics() {
    REGISTRY
        .register(Box::new(QUERIES_TOTAL.clone()))
        .expect("failed to register QUERIES_TOTAL");
    REGISTRY
        .register(Box::new(QUERY_DURATION.clone()))
        .expect("failed to register QUERY_DURATION");
    REGISTRY
        .register(Box::new(ACTIVE_CONNECTIONS.clone()))
        .expect("failed to register ACTIVE_CONNECTIONS");
    REGISTRY
        .register(Box::new(NODES_TOTAL.clone()))
        .expect("failed to register NODES_TOTAL");
    REGISTRY
        .register(Box::new(EDGES_TOTAL.clone()))
        .expect("failed to register EDGES_TOTAL");
}

pub async fn metrics_handler() -> String {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}
