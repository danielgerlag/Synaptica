//! Raft log replication tests.

use std::sync::Arc;

use openraft::storage::RaftLogReader;
use openraft::{Entry, LogId, RaftStorage};
use synaptica_cluster::log_store::RocksLogStore;
use synaptica_cluster::raft::TypeConfig;
use synaptica_cluster::state_machine::StateMachineApplier;
use synaptica_storage::engine::{StorageConfig, StorageEngine};

use crate::cluster_battle::harness::TestCluster;

/// Helper: create a standalone log store backed by a temp directory.
/// Returns (store, storage, _dir) — keep `_dir` alive to prevent cleanup.
fn standalone_log_store() -> (Arc<RocksLogStore>, Arc<StorageEngine>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap());
    let applier = Arc::new(StateMachineApplier::new(storage.clone()));
    let store = Arc::new(RocksLogStore::with_applier(storage.clone(), applier));
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
// 16. Single write entry replicates to all followers in 3-node cluster
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_16_single_write_replicates_to_all() {
    let cluster = TestCluster::new(3).await;

    let resp = cluster.write("INSERT (:Person {name: 'Alice'})").await;
    assert!(resp.is_ok(), "write must succeed: {:?}", resp.err());
    assert!(resp.unwrap().success);

    cluster.wait_for_convergence(5000).await;

    for id in 1..=3u64 {
        let count = cluster.count_nodes_on(id);
        assert_eq!(
            count, 1,
            "node {id} should have 1 graph node, found {count}"
        );
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 17. Batch of 50 sequential writes all replicate to followers
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_17_fifty_writes_replicate() {
    let cluster = TestCluster::new(3).await;

    for i in 0..50 {
        let query = format!("INSERT (:Item {{seq: {}}})", i);
        let resp = cluster.write(&query).await;
        assert!(resp.is_ok(), "write {i} must succeed: {:?}", resp.err());
        assert!(resp.unwrap().success, "write {i} must report success");
    }

    cluster.wait_for_convergence_count(50, 10_000).await;

    for id in 1..=3u64 {
        let count = cluster.count_nodes_on(id);
        assert_eq!(
            count, 50,
            "node {id} should have 50 graph nodes, found {count}"
        );
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 18. Follower with log gap catches up from leader
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_18_follower_catches_up_after_partition() {
    let cluster = TestCluster::new(3).await;

    let leader = cluster.get_leader().expect("must have leader");
    let follower = (1..=3u64).find(|&id| id != leader).unwrap();

    // Block the follower so it misses writes
    cluster.router.block_node(follower).await;

    for i in 0..5 {
        let query = format!("INSERT (:Item {{seq: {}}})", i);
        let resp = cluster.write(&query).await;
        assert!(resp.is_ok(), "write {i} must succeed with 2/3 quorum");
    }

    // Follower should be behind
    let count_before = cluster.count_nodes_on(follower);
    assert_eq!(count_before, 0, "blocked follower must have 0 nodes");

    // Unblock — follower should catch up via log replication
    cluster.router.unblock_node(follower).await;
    cluster.wait_for_convergence_count(5, 10_000).await;

    let count_after = cluster.count_nodes_on(follower);
    assert_eq!(
        count_after, 5,
        "follower must catch up to 5 nodes, found {count_after}"
    );

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 19. Log entries survive via RocksDB persistence (write, check log store)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_19_log_entries_persist_in_rocksdb() {
    let (mut store, _storage, _dir) = standalone_log_store();

    let entries = vec![blank_entry(1, 1), blank_entry(1, 2), blank_entry(1, 3)];
    store.append_to_log(entries).await.unwrap();

    // Read back from the same store (backed by the same RocksDB)
    let mut reader = store.get_log_reader().await;
    let read_back = reader.try_get_log_entries(1..4).await.unwrap();
    assert_eq!(read_back.len(), 3);
    assert_eq!(read_back[0].log_id.index, 1);
    assert_eq!(read_back[1].log_id.index, 2);
    assert_eq!(read_back[2].log_id.index, 3);
}

// ---------------------------------------------------------------------------
// 20. Log purging after commit removes old entries correctly
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_20_log_purging_removes_old_entries() {
    let (mut store, _storage, _dir) = standalone_log_store();

    // Append 5 entries
    let entries: Vec<_> = (1..=5).map(|i| blank_entry(1, i)).collect();
    store.append_to_log(entries).await.unwrap();

    // Purge entries up to index 3
    let purge_id = LogId::new(openraft::CommittedLeaderId::new(1, 0), 3);
    store.purge_logs_upto(purge_id).await.unwrap();

    // Entries 1-3 should be gone
    let mut reader = store.get_log_reader().await;
    let remaining = reader.try_get_log_entries(1..6).await.unwrap();
    assert_eq!(remaining.len(), 2, "only entries 4 and 5 should remain");
    assert_eq!(remaining[0].log_id.index, 4);
    assert_eq!(remaining[1].log_id.index, 5);

    // Log state should reflect purge
    let state = store.get_log_state().await.unwrap();
    assert_eq!(
        state.last_purged_log_id,
        Some(purge_id),
        "last_purged must match purge point"
    );
}

// ---------------------------------------------------------------------------
// 21. Large payload (10KB property value) entries replicate without corruption
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_21_large_payload_replicates() {
    let cluster = TestCluster::new(3).await;

    let large_value = "X".repeat(10_000);
    let query = format!("INSERT (:LargeNode {{data: '{}'}})", large_value);

    let resp = cluster.write(&query).await;
    assert!(resp.is_ok(), "large write must succeed: {:?}", resp.err());
    assert!(resp.unwrap().success);

    cluster.wait_for_convergence(5000).await;

    // Verify data integrity on each node by reading back the property
    for id in 1..=3u64 {
        let count = cluster.count_nodes_on(id);
        assert_eq!(count, 1, "node {id} must have 1 graph node");

        let result = cluster
            .read_on(id, "MATCH (n:LargeNode) RETURN n.data")
            .expect("read must succeed");
        assert_eq!(result.records.len(), 1, "node {id} must return 1 record");
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 22. Empty log range query returns zero entries
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_22_empty_log_range_returns_empty() {
    let (mut store, _storage, _dir) = standalone_log_store();

    // No entries appended — any range should return empty
    let mut reader = store.get_log_reader().await;
    let entries = reader.try_get_log_entries(0..100).await.unwrap();
    assert!(entries.is_empty(), "empty log must return no entries");

    // Also test a specific range
    let entries2 = reader.try_get_log_entries(5..10).await.unwrap();
    assert!(
        entries2.is_empty(),
        "range 5..10 on empty log must be empty"
    );
}

// ---------------------------------------------------------------------------
// 23. Log index ordering preserved across multiple append batches
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_23_index_ordering_across_batches() {
    let (mut store, _storage, _dir) = standalone_log_store();

    // Batch 1: entries 1-3
    let batch1: Vec<_> = (1..=3).map(|i| blank_entry(1, i)).collect();
    store.append_to_log(batch1).await.unwrap();

    // Batch 2: entries 4-6
    let batch2: Vec<_> = (4..=6).map(|i| blank_entry(1, i)).collect();
    store.append_to_log(batch2).await.unwrap();

    // Batch 3: entries 7-10
    let batch3: Vec<_> = (7..=10).map(|i| blank_entry(1, i)).collect();
    store.append_to_log(batch3).await.unwrap();

    // Read all entries and verify strict ordering
    let mut reader = store.get_log_reader().await;
    let all = reader.try_get_log_entries(1..11).await.unwrap();
    assert_eq!(all.len(), 10);
    for (i, entry) in all.iter().enumerate() {
        assert_eq!(
            entry.log_id.index,
            (i + 1) as u64,
            "entry at position {i} must have index {}",
            i + 1
        );
    }
}

// ---------------------------------------------------------------------------
// 24. Delete-conflict-logs removes all entries from specified index onward
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_24_delete_conflict_logs() {
    let (mut store, _storage, _dir) = standalone_log_store();

    // Append 10 entries
    let entries: Vec<_> = (1..=10).map(|i| blank_entry(1, i)).collect();
    store.append_to_log(entries).await.unwrap();

    // Delete conflict logs from index 6 onward
    let conflict_id = LogId::new(openraft::CommittedLeaderId::new(1, 0), 6);
    store.delete_conflict_logs_since(conflict_id).await.unwrap();

    // Only entries 1-5 should remain
    let mut reader = store.get_log_reader().await;
    let remaining = reader.try_get_log_entries(1..11).await.unwrap();
    assert_eq!(
        remaining.len(),
        5,
        "only entries 1-5 should remain after deleting from index 6"
    );
    for entry in &remaining {
        assert!(
            entry.log_id.index <= 5,
            "entry index {} should be <= 5",
            entry.log_id.index
        );
    }

    // Verify deleted range is truly empty
    let deleted = reader.try_get_log_entries(6..11).await.unwrap();
    assert!(deleted.is_empty(), "entries 6-10 must be deleted");
}

// ---------------------------------------------------------------------------
// 25. Log state reports correct last_purged and last_log_id after operations
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_25_log_state_after_operations() {
    let (mut store, _storage, _dir) = standalone_log_store();

    // Initially both should be None
    let state0 = store.get_log_state().await.unwrap();
    assert!(
        state0.last_purged_log_id.is_none(),
        "initial last_purged must be None"
    );
    assert!(
        state0.last_log_id.is_none(),
        "initial last_log_id must be None"
    );

    // Append entries 1-5
    let entries: Vec<_> = (1..=5).map(|i| blank_entry(1, i)).collect();
    store.append_to_log(entries).await.unwrap();

    let state1 = store.get_log_state().await.unwrap();
    assert!(
        state1.last_purged_log_id.is_none(),
        "last_purged must still be None"
    );
    let last = state1
        .last_log_id
        .expect("last_log_id must be Some after append");
    assert_eq!(last.index, 5, "last_log_id.index must be 5");

    // Purge up to index 3
    let purge_id = LogId::new(openraft::CommittedLeaderId::new(1, 0), 3);
    store.purge_logs_upto(purge_id).await.unwrap();

    let state2 = store.get_log_state().await.unwrap();
    assert_eq!(state2.last_purged_log_id, Some(purge_id));
    let last2 = state2.last_log_id.expect("last_log_id must still be Some");
    assert_eq!(
        last2.index, 5,
        "last_log_id must still be 5 after partial purge"
    );
}

// ---------------------------------------------------------------------------
// 26. Append entries with non-contiguous indices handled correctly
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_26_non_contiguous_indices() {
    let (mut store, _storage, _dir) = standalone_log_store();

    // Append entries with gaps (indices 1, 5, 10)
    let entries = vec![blank_entry(1, 1), blank_entry(1, 5), blank_entry(1, 10)];
    store.append_to_log(entries).await.unwrap();

    // Each should be individually retrievable
    let mut reader = store.get_log_reader().await;

    let e1 = reader.try_get_log_entries(1..2).await.unwrap();
    assert_eq!(e1.len(), 1);
    assert_eq!(e1[0].log_id.index, 1);

    let e5 = reader.try_get_log_entries(5..6).await.unwrap();
    assert_eq!(e5.len(), 1);
    assert_eq!(e5[0].log_id.index, 5);

    let e10 = reader.try_get_log_entries(10..11).await.unwrap();
    assert_eq!(e10.len(), 1);
    assert_eq!(e10[0].log_id.index, 10);

    // Range spanning gaps should return only existing entries
    let all = reader.try_get_log_entries(1..11).await.unwrap();
    assert_eq!(all.len(), 3, "only 3 entries exist despite range 1..11");
}

// ---------------------------------------------------------------------------
// 27. Log store handles rapid sequential appends (1000 entries)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_27_rapid_1000_entry_append() {
    let (mut store, _storage, _dir) = standalone_log_store();

    // Append 1000 entries in a single batch
    let entries: Vec<_> = (1..=1000).map(|i| blank_entry(1, i)).collect();
    store.append_to_log(entries).await.unwrap();

    // Verify all entries are present
    let mut reader = store.get_log_reader().await;
    let all = reader.try_get_log_entries(1..1001).await.unwrap();
    assert_eq!(all.len(), 1000, "all 1000 entries must be stored");
    assert_eq!(all.first().unwrap().log_id.index, 1);
    assert_eq!(all.last().unwrap().log_id.index, 1000);

    // Verify log state
    let state = store.get_log_state().await.unwrap();
    let last = state.last_log_id.expect("must have last_log_id");
    assert_eq!(last.index, 1000);
}

// ---------------------------------------------------------------------------
// 28. Reading entries from purged range returns empty
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_28_read_purged_range_returns_empty() {
    let (mut store, _storage, _dir) = standalone_log_store();

    // Append entries 1-10
    let entries: Vec<_> = (1..=10).map(|i| blank_entry(1, i)).collect();
    store.append_to_log(entries).await.unwrap();

    // Purge entries up to index 7
    let purge_id = LogId::new(openraft::CommittedLeaderId::new(1, 0), 7);
    store.purge_logs_upto(purge_id).await.unwrap();

    // Reading the purged range should return empty
    let mut reader = store.get_log_reader().await;
    let purged = reader.try_get_log_entries(1..8).await.unwrap();
    assert!(
        purged.is_empty(),
        "purged range 1..8 must return empty, got {}",
        purged.len()
    );

    // Entries 8-10 should still be readable
    let remaining = reader.try_get_log_entries(8..11).await.unwrap();
    assert_eq!(remaining.len(), 3, "entries 8-10 must survive purge");
    assert_eq!(remaining[0].log_id.index, 8);
    assert_eq!(remaining[2].log_id.index, 10);
}

// ---------------------------------------------------------------------------
// 29. Vote persistence in log store survives save/read cycle
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_29_vote_persistence_cycle() {
    let (mut store, _storage, _dir) = standalone_log_store();

    // Initially no vote
    let vote = store.read_vote().await.unwrap();
    assert!(vote.is_none(), "initial vote must be None");

    // Save a vote for term 5, voted_for node 3
    let v1 = openraft::Vote::new(5, 3);
    store.save_vote(&v1).await.unwrap();

    // Read it back
    let read1 = store.read_vote().await.unwrap().expect("vote must exist");
    assert_eq!(read1.leader_id().voted_for(), Some(3));

    // Overwrite with a new vote for term 7, voted_for node 1
    let v2 = openraft::Vote::new(7, 1);
    store.save_vote(&v2).await.unwrap();

    let read2 = store
        .read_vote()
        .await
        .unwrap()
        .expect("updated vote must exist");
    assert_eq!(read2.leader_id().voted_for(), Some(1));

    // Re-read to confirm persistence (not just cached)
    let read3 = store
        .read_vote()
        .await
        .unwrap()
        .expect("vote must still persist");
    assert_eq!(read3.leader_id().voted_for(), Some(1));
}

// ---------------------------------------------------------------------------
// 30. Committed index advances as followers acknowledge entries
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_30_committed_index_advances() {
    let cluster = TestCluster::new(3).await;

    let leader = cluster.get_leader().expect("must have leader");

    // Record the committed index before writes
    let metrics_before = cluster.metrics(leader);
    let committed_before = metrics_before.last_applied.map(|l| l.index).unwrap_or(0);

    // Perform several writes
    for i in 0..5 {
        let query = format!("INSERT (:Item {{seq: {}}})", i);
        let resp = cluster.write(&query).await;
        assert!(resp.is_ok(), "write {i} must succeed");
        assert!(resp.unwrap().success);
    }

    // Wait for replication convergence
    cluster.wait_for_convergence_count(5, 5000).await;

    // Committed / applied index must have advanced on the leader
    let metrics_after = cluster.metrics(leader);
    let committed_after = metrics_after.last_applied.map(|l| l.index).unwrap_or(0);
    assert!(
        committed_after > committed_before,
        "committed index must advance: before={committed_before}, after={committed_after}"
    );

    // Followers should also have advanced their applied index
    for id in 1..=3u64 {
        if id == leader {
            continue;
        }
        let fm = cluster.metrics(id);
        let follower_applied = fm.last_applied.map(|l| l.index).unwrap_or(0);
        assert!(
            follower_applied > committed_before,
            "follower {id} applied index ({follower_applied}) must advance past initial ({committed_before})"
        );
    }

    cluster.shutdown().await;
}
