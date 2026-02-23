use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::sync::Arc;
use synaptica_core::graph::{Edge, GraphId, Label, Node};
use synaptica_core::types::Value;
use synaptica_storage::engine::{StorageConfig, StorageEngine};
use synaptica_storage::mvcc::{MvccStore, TimestampOracle};

fn make_engine(dir: &std::path::Path) -> StorageEngine {
    StorageEngine::open(dir, &StorageConfig::default()).unwrap()
}

fn bench_node_insert(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path());
    let graph_id = GraphId::new();

    c.bench_function("node_insert_1000", |b| {
        b.iter(|| {
            for i in 0..1000 {
                let mut node = Node::new(graph_id);
                node.add_label("Person");
                node.set_property("name", Value::String(format!("user_{i}")));
                node.set_property("age", Value::Integer(i));
                engine.put_node(black_box(&node)).unwrap();
            }
        });
    });
}

fn bench_node_read(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path());
    let graph_id = GraphId::new();

    let mut node = Node::new(graph_id);
    node.add_label("Person");
    node.set_property("name", Value::String("Alice".into()));
    engine.put_node(&node).unwrap();

    let node_id = node.id;
    c.bench_function("node_read_by_id", |b| {
        b.iter(|| {
            engine
                .get_node(black_box(&graph_id), black_box(&node_id))
                .unwrap();
        });
    });
}

fn bench_node_scan(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path());
    let graph_id = GraphId::new();

    for i in 0..100 {
        let mut node = Node::new(graph_id);
        node.add_label("Person");
        node.set_property("idx", Value::Integer(i));
        engine.put_node(&node).unwrap();
    }

    c.bench_function("node_scan_100", |b| {
        b.iter(|| {
            let nodes = engine.scan_nodes(black_box(&graph_id)).unwrap();
            assert_eq!(nodes.len(), 100);
        });
    });
}

fn bench_label_scan(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path());
    let graph_id = GraphId::new();

    for i in 0..100 {
        let mut node = Node::new(graph_id);
        if i < 50 {
            node.add_label("Person");
        } else {
            node.add_label("Company");
        }
        node.set_property("idx", Value::Integer(i));
        engine.put_node(&node).unwrap();
    }

    let label = Label::new("Person");
    c.bench_function("label_scan_50_of_100", |b| {
        b.iter(|| {
            let nodes = engine
                .scan_nodes_by_label(black_box(&graph_id), black_box(&label))
                .unwrap();
            assert_eq!(nodes.len(), 50);
        });
    });
}

fn bench_edge_insert(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path());
    let graph_id = GraphId::new();

    // Pre-create source and target nodes
    let mut sources = Vec::new();
    let mut targets = Vec::new();
    for _ in 0..100 {
        let src = Node::new(graph_id);
        let tgt = Node::new(graph_id);
        engine.put_node(&src).unwrap();
        engine.put_node(&tgt).unwrap();
        sources.push(src.id);
        targets.push(tgt.id);
    }

    c.bench_function("edge_insert_100", |b| {
        b.iter(|| {
            for i in 0..100 {
                let mut edge = Edge::new(graph_id, sources[i], targets[i], "KNOWS");
                edge.set_property("weight", Value::Integer(i as i64));
                engine.put_edge(black_box(&edge)).unwrap();
            }
        });
    });
}

fn bench_adjacency_scan(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path());
    let graph_id = GraphId::new();

    let hub = Node::new(graph_id);
    engine.put_node(&hub).unwrap();

    for _ in 0..50 {
        let target = Node::new(graph_id);
        engine.put_node(&target).unwrap();
        let edge = Edge::new(graph_id, hub.id, target.id, "FOLLOWS");
        engine.put_edge(&edge).unwrap();
    }

    let hub_id = hub.id;
    c.bench_function("adjacency_scan_50_outgoing", |b| {
        b.iter(|| {
            let edges = engine
                .get_outgoing_edges(black_box(&graph_id), black_box(&hub_id), None)
                .unwrap();
            assert_eq!(edges.len(), 50);
        });
    });
}

fn bench_mvcc_write(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path());
    let ts_oracle = Arc::new(TimestampOracle::new());
    let store = MvccStore::new(engine.raw_db().clone(), ts_oracle.clone());

    c.bench_function("mvcc_write", |b| {
        b.iter(|| {
            let ts = ts_oracle.next();
            store
                .put_at("default", black_box(b"bench/key"), black_box(b"value"), ts)
                .unwrap();
        });
    });
}

fn bench_mvcc_read_at_snapshot(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path());
    let ts_oracle = Arc::new(TimestampOracle::new());
    let store = MvccStore::new(engine.raw_db().clone(), ts_oracle.clone());

    // Write multiple versions
    for i in 0..100u64 {
        let ts = ts_oracle.next();
        let value = format!("version_{i}");
        store
            .put_at("default", b"snap/key", value.as_bytes(), ts)
            .unwrap();
    }

    let snapshot_ts = 50;
    c.bench_function("mvcc_read_at_snapshot", |b| {
        b.iter(|| {
            store
                .get_at("default", black_box(b"snap/key"), black_box(snapshot_ts))
                .unwrap();
        });
    });
}

criterion_group!(
    benches,
    bench_node_insert,
    bench_node_read,
    bench_node_scan,
    bench_label_scan,
    bench_edge_insert,
    bench_adjacency_scan,
    bench_mvcc_write,
    bench_mvcc_read_at_snapshot,
);
criterion_main!(benches);
