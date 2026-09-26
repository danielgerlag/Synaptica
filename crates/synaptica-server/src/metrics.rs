use lazy_static::lazy_static;
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, IntGauge, Opts, Registry, TextEncoder,
};
use synaptica_storage::engine::StorageEngine;

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

/// Set node and edge gauges from the graphs currently stored.
pub fn refresh_storage_gauges(storage: &StorageEngine) {
    let graphs = match storage.list_graphs() {
        Ok(graphs) => graphs,
        Err(_) => return,
    };
    let mut nodes: i64 = 0;
    let mut edges: i64 = 0;
    for graph in &graphs {
        let Ok(stored_nodes) = storage.scan_nodes(&graph.id) else {
            continue;
        };
        nodes += stored_nodes.len() as i64;
        for node in &stored_nodes {
            if let Ok(outgoing) = storage.get_outgoing_edges(&graph.id, &node.id, None) {
                edges += outgoing.len() as i64;
            }
        }
    }
    NODES_TOTAL.set(nodes);
    EDGES_TOTAL.set(edges);
}

pub async fn metrics_handler() -> String {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use synaptica_core::graph::{Edge, GraphId, GraphMeta, Node};
    use synaptica_storage::engine::StorageConfig;

    #[test]
    fn gauges_count_stored_nodes_and_outgoing_edges() {
        let dir = tempfile::tempdir().unwrap();
        let storage = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let graph_id = GraphId::from_name("default");
        storage
            .put_graph_meta(&GraphMeta {
                id: graph_id,
                name: "default".to_string(),
                graph_type: None,
            })
            .unwrap();
        let mut alice = Node::new(graph_id);
        alice.add_label("Person");
        let mut bob = Node::new(graph_id);
        bob.add_label("Person");
        storage.put_node(&alice).unwrap();
        storage.put_node(&bob).unwrap();
        storage
            .put_edge(&Edge::new(graph_id, alice.id, bob.id, "KNOWS"))
            .unwrap();

        refresh_storage_gauges(&storage);
        assert_eq!(NODES_TOTAL.get(), 2);
        assert_eq!(EDGES_TOTAL.get(), 1);
    }
}
