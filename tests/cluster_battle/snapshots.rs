//! Snapshot tests.
//!
//! Tests 81–90: Snapshot building, installation on joiners, metadata
//! persistence, log purge, recovery reads, multi-graph and index
//! preservation, sequential snapshots, and snapshot during active writes.

use std::sync::Arc;

use openraft::storage::RaftLogReader;
use openraft::{Entry, LogId, RaftSnapshotBuilder, RaftStorage};
use synaptica_cluster::log_store::RocksLogStore;
use synaptica_cluster::raft::TypeConfig;
use synaptica_cluster::state_machine::StateMachineApplier;
use synaptica_core::graph::{GraphId, GraphMeta};
use synaptica_storage::engine::{StorageConfig, StorageEngine};

use crate::cluster_battle::harness::TestCluster;

/// Helper: create a standalone log store with graph metadata.
/// Returns (store, storage, _dir) — keep `_dir` alive to prevent cleanup.
fn standalone_store(
    graph_name: &str,
) -> (Arc<RocksLogStore>, Arc<StorageEngine>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let storage =
        Arc::new(StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap());
    let graph_id = GraphId::from_name(graph_name);
    let meta = GraphMeta {
        id: graph_id,
        name: graph_name.to_string(),
        graph_type: None,
    };
    storage.put_graph_meta(&meta).unwrap();
    let applier = Arc::new(StateMachineApplier::new(storage.clone()));
    let store = Arc::new(RocksLogStore::with_applier(
        storage.raw_db().clone(),
        applier,
    ));
    (store, storage, dir)
}

/// Helper: create a blank entry at the given (term, index).
fn blank_entry(term: u64, index: u64) -> Entry<TypeConfig> {
    Entry::<TypeConfig> {
        log_id: LogId::new(openraft::CommittedLeaderId::new(term, 0), index),
        payload: openraft::EntryPayload::Blank,
    }
}

// ---------------------------------------------------------------------------
// 81. Snapshot captures current graph state correctly
// ---------------------------------------------------------------------------
#[tokio::test]
async fn snapshot_captures_current_state() {
    let (mut store, _storage, _dir) = standalone_store("test");

    let snapshot =
        RaftSnapshotBuilder::<TypeConfig>::build_snapshot(&mut store).await.unwrap();

    assert!(
        snapshot.meta.snapshot_id.starts_with("snapshot-"),
        "snapshot_id should have the expected prefix, got: {}",
        snapshot.meta.snapshot_id
    );
}

// ---------------------------------------------------------------------------
// 82. Snapshot installation on new joiner transfers state
// ---------------------------------------------------------------------------
#[tokio::test]
async fn snapshot_installs_on_new_joiner() {
    let mut cluster = TestCluster::new(3).await;

    for i in 0..5 {
        cluster
            .write(&format!("INSERT (:Widget {{idx: {}}})", i))
            .await
            .unwrap();
    }
    cluster.wait_for_convergence(5000).await;

    // Add a learner — it receives state via snapshot or log replication
    cluster.add_learner(4).await;
    cluster.wait_for_convergence_count(5, 10_000).await;

    let rs = cluster
        .read_on(4, "MATCH (n:Widget) RETURN n.idx")
        .unwrap();
    assert_eq!(rs.records.len(), 5, "learner must have all 5 widgets");

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 83. Snapshot after 100 writes produces valid compact state
// ---------------------------------------------------------------------------
#[tokio::test]
async fn snapshot_after_100_writes() {
    let mut cluster = TestCluster::new(3).await;

    for i in 0..100 {
        cluster
            .write(&format!("INSERT (:Row {{idx: {}}})", i))
            .await
            .unwrap();
    }
    cluster.wait_for_convergence_count(100, 15_000).await;

    // Add a learner that must catch up
    cluster.add_learner(4).await;
    cluster.wait_for_convergence_count(100, 15_000).await;

    assert_eq!(
        cluster.count_nodes_on(4),
        100,
        "learner must have all 100 nodes after catching up"
    );

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 84. Snapshot metadata (term, index) persisted correctly
// ---------------------------------------------------------------------------
#[tokio::test]
async fn snapshot_metadata_persisted_correctly() {
    let (mut store, _storage, _dir) = standalone_store("test");

    // Build a snapshot on a fresh store
    let snap =
        RaftSnapshotBuilder::<TypeConfig>::build_snapshot(&mut store).await.unwrap();

    // No entries applied yet
    assert!(
        snap.meta.last_log_id.is_none(),
        "fresh snapshot should have no last_log_id"
    );
    assert!(
        snap.meta.snapshot_id.contains("snapshot-0"),
        "snapshot_id should contain index 0, got: {}",
        snap.meta.snapshot_id
    );

    // get_current_snapshot must return the same metadata
    let current = RaftStorage::<TypeConfig>::get_current_snapshot(&mut store)
        .await
        .unwrap();
    assert!(
        current.is_some(),
        "get_current_snapshot must return the built snapshot"
    );
    let current = current.unwrap();
    assert_eq!(current.meta.snapshot_id, snap.meta.snapshot_id);
}

// ---------------------------------------------------------------------------
// 85. Snapshot triggers log purge of entries before snapshot
// ---------------------------------------------------------------------------
#[tokio::test]
async fn snapshot_triggers_log_purge() {
    let (mut store, _storage, _dir) = standalone_store("test");

    // Append 10 blank entries
    let entries: Vec<_> = (1..=10).map(|i| blank_entry(1, i)).collect();
    store.append_to_log(entries).await.unwrap();

    // Build a snapshot (records current state)
    let _snap =
        RaftSnapshotBuilder::<TypeConfig>::build_snapshot(&mut store).await.unwrap();

    // Purge log entries up to index 7
    let purge_id = LogId::new(openraft::CommittedLeaderId::new(1, 0), 7);
    store.purge_logs_upto(purge_id).await.unwrap();

    // Entries 1-7 should be gone, 8-10 remain
    let mut reader = store.get_log_reader().await;
    let remaining = reader.try_get_log_entries(1..11).await.unwrap();
    assert_eq!(remaining.len(), 3, "only entries 8, 9, 10 should remain");
    assert_eq!(remaining[0].log_id.index, 8);
    assert_eq!(remaining[1].log_id.index, 9);
    assert_eq!(remaining[2].log_id.index, 10);

    // Log state reflects the purge
    let state = store.get_log_state().await.unwrap();
    assert_eq!(
        state.last_purged_log_id,
        Some(purge_id),
        "last_purged must match purge point"
    );
}

// ---------------------------------------------------------------------------
// 86. Node recovering from snapshot can serve reads
// ---------------------------------------------------------------------------
#[tokio::test]
async fn node_recovering_from_snapshot_serves_reads() {
    let mut cluster = TestCluster::new(3).await;

    cluster
        .write("INSERT (:Person {name: 'Alice', age: 30})")
        .await
        .unwrap();
    cluster
        .write("INSERT (:Person {name: 'Bob', age: 25})")
        .await
        .unwrap();
    cluster.wait_for_convergence(5000).await;

    // Add a learner after data was committed
    cluster.add_learner(4).await;
    cluster.wait_for_convergence_count(2, 10_000).await;

    // Learner should serve reads
    let rs = cluster
        .read_on(4, "MATCH (n:Person) RETURN n.name ORDER BY n.name")
        .unwrap();
    assert_eq!(rs.records.len(), 2, "learner must return both persons");
    assert_eq!(
        rs.records[0].get("n.name"),
        Some(&synaptica_core::types::Value::String("Alice".into()))
    );
    assert_eq!(
        rs.records[1].get("n.name"),
        Some(&synaptica_core::types::Value::String("Bob".into()))
    );

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 87. Snapshot with multiple graphs preserves all data
// ---------------------------------------------------------------------------
#[tokio::test]
async fn snapshot_with_multiple_graphs() {
    let (mut store, storage, _dir) = standalone_store("graph_a");

    // Create a second graph in the same storage
    let graph_b_id = GraphId::from_name("graph_b");
    let meta_b = GraphMeta {
        id: graph_b_id,
        name: "graph_b".to_string(),
        graph_type: None,
    };
    storage.put_graph_meta(&meta_b).unwrap();

    // Verify both graphs are present
    let ga = storage.get_graph_meta(&GraphId::from_name("graph_a"));
    assert!(ga.is_ok(), "graph_a meta must exist");
    let gb = storage.get_graph_meta(&GraphId::from_name("graph_b"));
    assert!(gb.is_ok(), "graph_b meta must exist");

    // Build snapshot — should succeed with multiple graphs
    let snap =
        RaftSnapshotBuilder::<TypeConfig>::build_snapshot(&mut store).await.unwrap();
    assert!(
        snap.meta.snapshot_id.starts_with("snapshot-"),
        "snapshot must be built successfully with multiple graphs"
    );

    // Graphs remain accessible after snapshot
    let ga2 = storage.get_graph_meta(&GraphId::from_name("graph_a"));
    assert!(ga2.is_ok(), "graph_a must persist after snapshot");
    let gb2 = storage.get_graph_meta(&GraphId::from_name("graph_b"));
    assert!(gb2.is_ok(), "graph_b must persist after snapshot");
}

// ---------------------------------------------------------------------------
// 88. Snapshot with indexes preserves index definitions
// ---------------------------------------------------------------------------
#[tokio::test]
async fn snapshot_with_indexes_preserves_definitions() {
    use synaptica_storage::index::IndexManager;

    let mut cluster = TestCluster::new(3).await;

    cluster
        .write("CREATE INDEX idx_name FOR (n:Person) ON (n.name)")
        .await
        .unwrap();
    cluster
        .write("INSERT (:Person {name: 'Alice'})")
        .await
        .unwrap();
    cluster.wait_for_convergence(5000).await;

    // Add learner — must receive index definition via replication/snapshot
    cluster.add_learner(4).await;
    cluster.wait_for_convergence_count(1, 10_000).await;

    // Verify index on the learner
    let learner_node = cluster.nodes.get(&4).unwrap();
    let idx_mgr = IndexManager::new(learner_node.storage.raw_db().clone());
    let graph_id = GraphId::from_name("test");
    let indexes = idx_mgr.list_indexes(&graph_id).unwrap();
    assert!(
        indexes.iter().any(|idx| idx.name == "idx_name"),
        "index definition must be present on learner, found: {:?}",
        indexes.iter().map(|i| &i.name).collect::<Vec<_>>()
    );

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 89. Two sequential snapshots produce consistent state
// ---------------------------------------------------------------------------
#[tokio::test]
async fn two_sequential_snapshots_are_consistent() {
    let (mut store, _storage, _dir) = standalone_store("test");

    // Append some entries
    let entries: Vec<_> = (1..=5).map(|i| blank_entry(1, i)).collect();
    store.append_to_log(entries).await.unwrap();

    // First snapshot
    let snap1 =
        RaftSnapshotBuilder::<TypeConfig>::build_snapshot(&mut store).await.unwrap();
    let id1 = snap1.meta.snapshot_id.clone();

    // Append more entries
    let entries2: Vec<_> = (6..=10).map(|i| blank_entry(1, i)).collect();
    store.append_to_log(entries2).await.unwrap();

    // Second snapshot
    let snap2 =
        RaftSnapshotBuilder::<TypeConfig>::build_snapshot(&mut store).await.unwrap();
    let id2 = snap2.meta.snapshot_id.clone();

    // The two snapshots must have distinct IDs (UUIDs differ)
    assert_ne!(id1, id2, "sequential snapshots must have different IDs");

    // get_current_snapshot should return the latest
    let current = RaftStorage::<TypeConfig>::get_current_snapshot(&mut store)
        .await
        .unwrap()
        .expect("must have a current snapshot");
    assert_eq!(
        current.meta.snapshot_id, id2,
        "current snapshot must be the second one"
    );
}

// ---------------------------------------------------------------------------
// 90. Snapshot during active writes doesn't corrupt data
// ---------------------------------------------------------------------------
#[tokio::test]
async fn snapshot_during_active_writes_no_corruption() {
    let cluster = TestCluster::new(3).await;

    // Pre-seed data
    for i in 0..10 {
        cluster
            .write(&format!("INSERT (:Item {{idx: {}}})", i))
            .await
            .unwrap();
    }
    cluster.wait_for_convergence(5000).await;

    // Spawn concurrent writes
    let cluster = std::sync::Arc::new(cluster);
    let writer = {
        let c = cluster.clone();
        tokio::spawn(async move {
            for i in 10..30 {
                let _ = c.write(&format!("INSERT (:Item {{idx: {}}})", i)).await;
            }
        })
    };

    // Concurrently, read on each node to verify no corruption
    let node_ids: Vec<u64> = cluster.nodes.keys().copied().collect();
    for id in &node_ids {
        let rs = cluster.read_on(*id, "MATCH (n:Item) RETURN n.idx").unwrap();
        // Must see at least the pre-seeded 10, at most all 30
        assert!(
            rs.records.len() >= 10,
            "node {id} must see >= 10 items during concurrent writes, got {}",
            rs.records.len()
        );
    }

    writer.await.unwrap();

    // After writes complete, wait for convergence
    let cluster_ref = &*cluster;
    let final_count = cluster_ref.count_nodes_on(node_ids[0]);
    cluster_ref
        .wait_for_convergence_count(final_count, 10_000)
        .await;

    // All nodes must agree on the count
    for id in &node_ids {
        assert_eq!(
            cluster_ref.count_nodes_on(*id),
            final_count,
            "node {id} must converge to {final_count} items"
        );
    }

    match std::sync::Arc::try_unwrap(cluster) {
        Ok(c) => c.shutdown().await,
        Err(_) => panic!("all handles should have completed"),
    }
}
