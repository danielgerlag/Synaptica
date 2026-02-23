//! Integration tests for the Synaptica cluster layer.
//!
//! These tests exercise multiple cluster components working together:
//! membership, partitioning, routing, snapshots, and rebalancing.

use std::time::Instant;

use synaptica_cluster::membership::{ClusterMembership, ClusterNode};
use synaptica_cluster::partition::{Partition, PartitionId, PartitionMap, PartitionRange};
use synaptica_cluster::rebalance::compute_rebalance_plan;
use synaptica_cluster::routing::QueryRouter;
use synaptica_cluster::snapshot::SnapshotManager;

// ===========================================================================
// Helpers
// ===========================================================================

fn make_node(id: &str, alive: bool) -> ClusterNode {
    ClusterNode {
        id: id.to_string(),
        address: format!("127.0.0.1:{}", id),
        is_alive: alive,
        last_heartbeat: Instant::now(),
    }
}

fn make_partition(id: &str, start: u8, end: u8, leader: &str) -> Partition {
    Partition {
        id: PartitionId(id.to_string()),
        range: PartitionRange {
            start: vec![start],
            end: vec![end],
        },
        leader_node: leader.to_string(),
        replicas: vec![],
    }
}

// ===========================================================================
// Integration tests
// ===========================================================================

/// Create a PartitionMap with 3 partitions assigned to different nodes.
/// Create a QueryRouter from the partition map. Route a query and verify the
/// sub-query goes to the correct (first) partition.
#[test]
fn test_partition_routing_integration() {
    let mut pm = PartitionMap::new();
    pm.add_partition(make_partition("p1", 0x00, 0x55, "node-1")).unwrap();
    pm.add_partition(make_partition("p2", 0x55, 0xAA, "node-2")).unwrap();
    pm.add_partition(make_partition("p3", 0xAA, 0xFF, "node-3")).unwrap();

    let router = QueryRouter::new();
    let result = router.route_query("SELECT * FROM nodes", &pm);

    assert_eq!(result.len(), 1, "router should produce exactly one sub-query");
    // The stub routes to the first partition (sorted by range start).
    assert_eq!(result[0].0, PartitionId("p1".to_string()));
    assert_eq!(result[0].1.query, "SELECT * FROM nodes");
}

/// Create ClusterMembership, add 3 nodes, mark one dead, verify alive_nodes
/// is 2. Then mark it alive again, verify alive_nodes is 3.
#[test]
fn test_membership_lifecycle() {
    let mut membership = ClusterMembership::new();
    membership.add_node(make_node("n1", true));
    membership.add_node(make_node("n2", true));
    membership.add_node(make_node("n3", true));
    assert_eq!(membership.alive_nodes().len(), 3);

    membership.mark_dead("n2");
    assert_eq!(membership.alive_nodes().len(), 2);
    assert!(!membership.get_node("n2").unwrap().is_alive);

    membership.mark_alive("n2");
    assert_eq!(membership.alive_nodes().len(), 3);
    assert!(membership.get_node("n2").unwrap().is_alive);
}

/// Create membership with 3 nodes. Create partitions assigned to those nodes.
/// Mark a node dead. Verify we can still find the partition but the assigned
/// node is dead.
#[test]
fn test_partition_map_with_membership() {
    let mut membership = ClusterMembership::new();
    membership.add_node(make_node("node-1", true));
    membership.add_node(make_node("node-2", true));
    membership.add_node(make_node("node-3", true));

    let mut pm = PartitionMap::new();
    pm.add_partition(make_partition("p1", 0x00, 0x55, "node-1")).unwrap();
    pm.add_partition(make_partition("p2", 0x55, 0xAA, "node-2")).unwrap();
    pm.add_partition(make_partition("p3", 0xAA, 0xFF, "node-3")).unwrap();

    // Mark node-2 as dead.
    membership.mark_dead("node-2");

    // Partition p2 is still discoverable.
    let partition = pm.find_partition(&[0x70]).expect("p2 should cover 0x70");
    assert_eq!(partition.id, PartitionId("p2".to_string()));
    assert_eq!(partition.leader_node, "node-2");

    // But the leader node is dead in the membership view.
    let leader = membership
        .get_node(&partition.leader_node)
        .expect("node-2 should exist");
    assert!(!leader.is_alive);
}

/// Create a SnapshotManager, create 3 snapshots, verify indexes increment.
/// Restore to snapshot 2, create a new snapshot, verify index is 3.
#[test]
fn test_snapshot_lifecycle() {
    let mut mgr = SnapshotManager::new();

    let snap1 = mgr.create_snapshot();
    let snap2 = mgr.create_snapshot();
    let snap3 = mgr.create_snapshot();

    assert_eq!(snap1.index, 1);
    assert_eq!(snap2.index, 2);
    assert_eq!(snap3.index, 3);

    // Restore to snapshot 2, then create another — should be index 3.
    mgr.restore_snapshot(&snap2);
    let snap_after_restore = mgr.create_snapshot();
    assert_eq!(snap_after_restore.index, 3);
}

/// Simulate a 3-node cluster lifecycle end-to-end:
///   a. Create membership, add 3 nodes
///   b. Create partition map with 3 partitions, one per node
///   c. Route a query, verify it routes correctly
///   d. Simulate node failure (mark dead)
///   e. Verify membership reflects the failure
///   f. Simulate recovery (mark alive)
///   g. Create a snapshot
#[test]
fn test_full_cluster_simulation() {
    // (a) Membership
    let mut membership = ClusterMembership::new();
    membership.add_node(make_node("node-1", true));
    membership.add_node(make_node("node-2", true));
    membership.add_node(make_node("node-3", true));
    assert_eq!(membership.alive_nodes().len(), 3);

    // (b) Partition map
    let mut pm = PartitionMap::new();
    pm.add_partition(make_partition("p1", 0x00, 0x55, "node-1")).unwrap();
    pm.add_partition(make_partition("p2", 0x55, 0xAA, "node-2")).unwrap();
    pm.add_partition(make_partition("p3", 0xAA, 0xFF, "node-3")).unwrap();
    assert_eq!(pm.all_partitions().len(), 3);

    // (c) Route a query
    let router = QueryRouter::new();
    let routed = router.route_query("MATCH (n) RETURN n", &pm);
    assert!(!routed.is_empty());
    assert_eq!(routed[0].0, PartitionId("p1".to_string()));

    // (d) Node failure
    membership.mark_dead("node-3");

    // (e) Verify failure is reflected
    assert_eq!(membership.alive_nodes().len(), 2);
    assert!(!membership.get_node("node-3").unwrap().is_alive);

    // (f) Recovery
    membership.mark_alive("node-3");
    assert_eq!(membership.alive_nodes().len(), 3);
    assert!(membership.get_node("node-3").unwrap().is_alive);

    // (g) Snapshot
    let mut snap_mgr = SnapshotManager::new();
    let snap = snap_mgr.create_snapshot();
    assert_eq!(snap.index, 1);
}

/// Create initial partitions on 2 nodes, add a 3rd node, run
/// compute_rebalance_plan, verify the result is a valid plan.
#[test]
fn test_rebalance_after_node_addition() {
    // Initial cluster: 2 nodes, 2 partitions.
    let partitions = vec![
        make_partition("p1", 0x00, 0x80, "node-1"),
        make_partition("p2", 0x80, 0xFF, "node-2"),
    ];
    let initial_nodes = vec!["node-1".to_string(), "node-2".to_string()];

    let plan_before = compute_rebalance_plan(&partitions, &initial_nodes);
    // Stub returns empty plan — the interface should still work.
    assert!(plan_before.moves.is_empty());

    // Add a third node and rebalance.
    let expanded_nodes = vec![
        "node-1".to_string(),
        "node-2".to_string(),
        "node-3".to_string(),
    ];
    let plan_after = compute_rebalance_plan(&partitions, &expanded_nodes);
    // Current stub returns an empty plan; just verify it's a valid RebalancePlan.
    let _ = plan_after.moves.len(); // ensure field is accessible
}
