//! Raft membership change tests.
//!
//! Tests 31–45: Adding learners, promoting voters, removing voters,
//! membership consistency, and edge cases.

use std::collections::BTreeSet;
use std::time::Instant;

use openraft::ServerState;
use synaptica_cluster::membership::{ClusterMembership, ClusterNode};
use synaptica_cluster::raft::NodeId;

use crate::cluster_battle::harness::TestCluster;

// ---------------------------------------------------------------------------
// 31. Add learner node to running 3-node cluster succeeds
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_31_add_learner_to_running_cluster() {
    let mut cluster = TestCluster::new(3).await;

    cluster.add_learner(4).await;

    // The new node must exist in the cluster
    assert!(
        cluster.nodes.contains_key(&4),
        "node 4 must be in the cluster"
    );

    // Verify learner is tracked in membership (learners appear in metrics)
    let leader = cluster.get_leader().expect("must have leader");
    let metrics = cluster.metrics(leader);
    let membership = metrics.membership_config.membership();

    // Node 4 should be a learner, not a voter
    if let Some(voters) = membership.get_joint_config().first() {
        assert!(!voters.contains(&4), "node 4 should not be a voter yet");
    }

    // Write data and verify learner receives it
    cluster
        .write("INSERT (:Person {name: 'Learner'})")
        .await
        .unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    let count = cluster.count_nodes_on(4);
    assert!(count >= 1, "learner should replicate data, got {count}");

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 32. Promote learner to voter via change_membership
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_32_promote_learner_to_voter() {
    let mut cluster = TestCluster::new(3).await;

    // Add node 4 as learner then promote it to voter
    cluster.add_learner(4).await;
    let new_voters: BTreeSet<NodeId> = [1, 2, 3, 4].into_iter().collect();
    cluster.change_membership(new_voters.clone()).await;

    // Verify node 4 is now a voter
    let leader = cluster.get_leader().expect("must have leader");
    let voters = cluster
        .metrics(leader)
        .membership_config
        .membership()
        .get_joint_config()
        .first()
        .cloned()
        .unwrap_or_default();
    assert!(
        voters.contains(&4),
        "node 4 must be a voter after promotion"
    );
    assert_eq!(voters.len(), 4, "should have 4 voters");

    // Verify writes still work after membership change
    let res = cluster.write("INSERT (:Fruit {name: 'apple'})").await;
    assert!(res.is_ok(), "writes must succeed after promotion");

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 33. Remove voter from 5-node cluster without losing availability
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_33_remove_voter_keeps_availability() {
    let cluster = TestCluster::new(5).await;

    // Remove node 5 from voter set
    let reduced: BTreeSet<NodeId> = [1, 2, 3, 4].into_iter().collect();
    cluster.change_membership(reduced).await;

    // Give time for membership to propagate
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    // Verify node 5 is no longer a voter
    let leader = cluster.get_leader().expect("must still have leader");
    let voters = cluster
        .metrics(leader)
        .membership_config
        .membership()
        .get_joint_config()
        .first()
        .cloned()
        .unwrap_or_default();
    assert!(!voters.contains(&5), "node 5 must be removed from voters");

    // Writes must still succeed
    let res = cluster.write("INSERT (:City {name: 'Oslo'})").await;
    assert!(res.is_ok(), "writes must succeed after removing one voter");

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 34. Add two learners simultaneously to cluster
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_34_add_two_learners_simultaneously() {
    let mut cluster = TestCluster::new(3).await;

    cluster.add_learner(4).await;
    cluster.add_learner(5).await;

    assert!(cluster.nodes.contains_key(&4), "node 4 must exist");
    assert!(cluster.nodes.contains_key(&5), "node 5 must exist");

    // Both learners should replicate data
    cluster
        .write("INSERT (:Animal {name: 'cat'})")
        .await
        .unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let c4 = cluster.count_nodes_on(4);
    let c5 = cluster.count_nodes_on(5);
    assert!(c4 >= 1, "learner 4 should have data, got {c4}");
    assert!(c5 >= 1, "learner 5 should have data, got {c5}");

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 35. Membership change rejected when no leader exists
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_35_membership_change_fails_without_leader() {
    let cluster = TestCluster::new(3).await;

    // Block enough nodes to lose quorum (block all 3 = no majority)
    cluster.router.block_node(1).await;
    cluster.router.block_node(2).await;
    cluster.router.block_node(3).await;

    // Wait long enough for the leader to step down
    tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;

    // Attempting change_membership should fail because there is no reachable leader.
    // We use timeout to prevent hanging forever.
    let new_voters: BTreeSet<NodeId> = [1, 2, 3].into_iter().collect();
    let result = tokio::time::timeout(tokio::time::Duration::from_millis(3000), async {
        // Drive the raft directly to demonstrate the error
        let node = cluster.nodes.get(&1).unwrap();
        node.raft.change_membership(new_voters, false).await
    })
    .await;

    match result {
        Ok(Ok(_)) => panic!("membership change should not succeed without quorum"),
        Ok(Err(_)) => { /* expected: Raft returned an error */ }
        Err(_) => { /* expected: timed out because no leader can commit */ }
    }

    // Heal partition for cleanup
    cluster.router.unblock_node(1).await;
    cluster.router.unblock_node(2).await;
    cluster.router.unblock_node(3).await;
    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 36. Cluster reconfiguration from 3 to 5 voters
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_36_scale_up_3_to_5_voters() {
    let mut cluster = TestCluster::new(3).await;

    // Add two new learners
    cluster.add_learner(4).await;
    cluster.add_learner(5).await;

    // Promote both to voters
    let five_voters: BTreeSet<NodeId> = [1, 2, 3, 4, 5].into_iter().collect();
    cluster.change_membership(five_voters.clone()).await;

    // Verify all 5 are voters
    let leader = cluster.get_leader().expect("must have leader");
    let voters = cluster
        .metrics(leader)
        .membership_config
        .membership()
        .get_joint_config()
        .first()
        .cloned()
        .unwrap_or_default();
    assert_eq!(voters.len(), 5, "must have 5 voters");
    for id in 1..=5u64 {
        assert!(voters.contains(&id), "node {id} must be a voter");
    }

    // Verify writes work with new membership
    let res = cluster.write("INSERT (:Color {name: 'blue'})").await;
    assert!(res.is_ok(), "writes must succeed with 5 voters");

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 37. Cluster downsizing from 5 to 3 voters
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_37_downsize_5_to_3_voters() {
    let cluster = TestCluster::new(5).await;

    // Downsize to 3 voters
    let three_voters: BTreeSet<NodeId> = [1, 2, 3].into_iter().collect();
    cluster.change_membership(three_voters.clone()).await;

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let leader = cluster
        .get_leader()
        .expect("must still have leader after downsizing");
    let voters = cluster
        .metrics(leader)
        .membership_config
        .membership()
        .get_joint_config()
        .first()
        .cloned()
        .unwrap_or_default();
    assert_eq!(voters.len(), 3, "must have 3 voters after downsizing");
    assert!(!voters.contains(&4), "node 4 should no longer be a voter");
    assert!(!voters.contains(&5), "node 5 should no longer be a voter");

    // Cluster is still operational
    let res = cluster.write("INSERT (:Planet {name: 'Mars'})").await;
    assert!(res.is_ok(), "writes must succeed after downsizing");

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 38. Adding duplicate node ID is handled gracefully
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_38_add_duplicate_learner_graceful() {
    let mut cluster = TestCluster::new(3).await;

    // Add node 4 as learner
    cluster.add_learner(4).await;

    // Adding the same node ID again via the raft API should not panic
    let leader = cluster.get_leader().expect("must have leader");
    let leader_raft = &cluster.nodes.get(&leader).unwrap().raft;
    let result = leader_raft
        .add_learner(
            4,
            openraft::BasicNode {
                addr: "127.0.0.1:9194".to_string(),
            },
            true,
        )
        .await;

    // openraft handles duplicate add_learner gracefully (succeeds or returns Ok)
    assert!(
        result.is_ok(),
        "duplicate add_learner should not error: {result:?}"
    );

    // Cluster should still work
    let res = cluster.write("INSERT (:Item {name: 'dup'})").await;
    assert!(
        res.is_ok(),
        "writes must still work after duplicate add_learner"
    );

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 39. Membership change with invalid/unknown node ID fails
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_39_membership_change_unknown_node_fails() {
    let cluster = TestCluster::new(3).await;

    // Try to include a node that was never added (node 99)
    let bad_voters: BTreeSet<NodeId> = [1, 2, 3, 99].into_iter().collect();
    let leader = cluster.get_leader().expect("must have leader");
    let leader_raft = &cluster.nodes.get(&leader).unwrap().raft;

    let result = leader_raft.change_membership(bad_voters, false).await;
    assert!(
        result.is_err(),
        "membership change including unknown node 99 must fail"
    );

    // Original membership unaffected; writes still work
    let res = cluster.write("INSERT (:Mineral {name: 'quartz'})").await;
    assert!(
        res.is_ok(),
        "writes must succeed after failed membership change"
    );

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 40. Membership state consistent across all nodes after change
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_40_membership_consistent_across_nodes() {
    let mut cluster = TestCluster::new(3).await;

    // Scale from 3 to 4 voters
    cluster.add_learner(4).await;
    let four_voters: BTreeSet<NodeId> = [1, 2, 3, 4].into_iter().collect();
    cluster.change_membership(four_voters.clone()).await;

    // Allow replication to propagate
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Every node should report the same voter set
    for id in 1..=4u64 {
        let voters = cluster
            .metrics(id)
            .membership_config
            .membership()
            .get_joint_config()
            .first()
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            voters, four_voters,
            "node {id} must report 4-voter membership, got {voters:?}"
        );
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 41. New voter participates in elections after promotion
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_41_promoted_voter_participates_in_elections() {
    let mut cluster = TestCluster::new(3).await;

    // Add and promote node 4
    cluster.add_learner(4).await;
    let voters: BTreeSet<NodeId> = [1, 2, 3, 4].into_iter().collect();
    cluster.change_membership(voters).await;

    // Let replication settle
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    // Block the current leader to force re-election
    let old_leader = cluster.get_leader().expect("must have leader");
    cluster.router.block_node(old_leader).await;

    // Wait for a new leader among the remaining voters
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(5000);
    let mut new_leader = None;
    while tokio::time::Instant::now() < deadline {
        for &id in &[1u64, 2, 3, 4] {
            if id == old_leader {
                continue;
            }
            let m = cluster.metrics(id);
            if m.current_leader.is_some() && m.current_leader != Some(old_leader) {
                new_leader = m.current_leader;
            }
        }
        if new_leader.is_some() {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    assert!(new_leader.is_some(), "new leader must be elected");
    // The promoted node 4 is eligible — it may or may not win, but the election
    // must succeed proving it participates.
    assert_ne!(
        new_leader.unwrap(),
        old_leader,
        "new leader must differ from old"
    );

    cluster.router.unblock_node(old_leader).await;
    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 42. Removed voter stops receiving new log entries
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_42_removed_voter_stops_receiving_entries() {
    let cluster = TestCluster::new(5).await;

    // Write initial data so we have a baseline
    cluster
        .write("INSERT (:Base {name: 'baseline'})")
        .await
        .unwrap();
    cluster.wait_for_convergence(5000).await;

    let initial_count = cluster.count_nodes_on(5);
    assert!(initial_count >= 1, "node 5 should have initial data");

    // Remove node 5 from voters
    let reduced: BTreeSet<NodeId> = [1, 2, 3, 4].into_iter().collect();
    cluster.change_membership(reduced).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Write more data AFTER removing node 5
    cluster
        .write("INSERT (:Post {name: 'afterRemoval1'})")
        .await
        .unwrap();
    cluster
        .write("INSERT (:Post {name: 'afterRemoval2'})")
        .await
        .unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Voters should have more data
    let voter_count = cluster.count_nodes_on(1);
    let removed_count = cluster.count_nodes_on(5);

    // The removed node should have fewer entries than voters
    assert!(
        voter_count > removed_count,
        "voter (node 1) should have more entries ({voter_count}) than removed node 5 ({removed_count})"
    );

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 43. Learner receives replicated entries but cannot be elected leader
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_43_learner_receives_data_but_not_elected() {
    let mut cluster = TestCluster::new(3).await;

    // Add node 4 as learner (NOT promoted to voter)
    cluster.add_learner(4).await;

    // Write data and verify learner receives it
    cluster
        .write("INSERT (:Food {name: 'bread'})")
        .await
        .unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let learner_count = cluster.count_nodes_on(4);
    assert!(
        learner_count >= 1,
        "learner must receive replicated data, got {learner_count}"
    );

    // Block current leader to force re-election among voters only
    let old_leader = cluster.get_leader().expect("must have leader");
    cluster.router.block_node(old_leader).await;

    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(5000);
    let mut new_leader = None;
    while tokio::time::Instant::now() < deadline {
        for &id in &[1u64, 2, 3] {
            if id == old_leader {
                continue;
            }
            let m = cluster.metrics(id);
            if m.current_leader.is_some() && m.current_leader != Some(old_leader) {
                new_leader = m.current_leader;
            }
        }
        if new_leader.is_some() {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    assert!(
        new_leader.is_some(),
        "new leader must be elected among voters"
    );
    assert_ne!(
        new_leader.unwrap(),
        4,
        "learner (node 4) must NOT become leader"
    );

    // Verify node 4 is not in Leader state
    let m4 = cluster.metrics(4);
    assert_ne!(
        m4.state,
        ServerState::Leader,
        "learner must not be in Leader state"
    );

    cluster.router.unblock_node(old_leader).await;
    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 44. Membership persisted in log survives (verify via metrics)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_44_membership_persisted_in_metrics() {
    let mut cluster = TestCluster::new(3).await;

    // Record initial membership
    let leader = cluster.get_leader().expect("must have leader");
    let initial_voters = cluster
        .metrics(leader)
        .membership_config
        .membership()
        .get_joint_config()
        .first()
        .cloned()
        .unwrap_or_default();
    let expected_initial: BTreeSet<NodeId> = [1, 2, 3].into_iter().collect();
    assert_eq!(
        initial_voters, expected_initial,
        "initial membership must be {{1,2,3}}"
    );

    // Change membership to include a new voter
    cluster.add_learner(4).await;
    let expanded: BTreeSet<NodeId> = [1, 2, 3, 4].into_iter().collect();
    cluster.change_membership(expanded.clone()).await;

    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    // Write some more data to advance the log
    cluster.write("INSERT (:Log {entry: 'a'})").await.unwrap();
    cluster.write("INSERT (:Log {entry: 'b'})").await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    // Membership must still reflect the 4-voter config
    let leader = cluster.get_leader().expect("must have leader after writes");
    let final_voters = cluster
        .metrics(leader)
        .membership_config
        .membership()
        .get_joint_config()
        .first()
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        final_voters, expanded,
        "membership must persist as {{1,2,3,4}} after additional log entries"
    );

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 45. Rapid add/remove cycles don't corrupt membership
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_45_rapid_add_remove_cycles() {
    let mut cluster = TestCluster::new(3).await;

    // Rapidly add and then remove 3 nodes
    for extra_id in 4..=6u64 {
        cluster.add_learner(extra_id).await;
        let mut voters: BTreeSet<NodeId> = [1, 2, 3].into_iter().collect();
        voters.insert(extra_id);
        cluster.change_membership(voters).await;

        // Small pause to let Raft commit the config
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        // Remove the node we just added
        let original: BTreeSet<NodeId> = [1, 2, 3].into_iter().collect();
        cluster.change_membership(original).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

    // Verify final membership is exactly {1,2,3}
    let leader = cluster.get_leader().expect("must have leader after cycles");
    let voters = cluster
        .metrics(leader)
        .membership_config
        .membership()
        .get_joint_config()
        .first()
        .cloned()
        .unwrap_or_default();
    let expected: BTreeSet<NodeId> = [1, 2, 3].into_iter().collect();
    assert_eq!(
        voters, expected,
        "membership must be {{1,2,3}} after rapid add/remove, got {voters:?}"
    );

    // Cluster must still be fully operational
    let res = cluster.write("INSERT (:Stable {state: 'ok'})").await;
    assert!(res.is_ok(), "writes must succeed after rapid cycles");

    // Wait for replication to the 3 original voters
    let start = tokio::time::Instant::now();
    loop {
        if start.elapsed() > tokio::time::Duration::from_secs(10) {
            panic!("timeout waiting for original voters to converge");
        }
        let c1 = cluster.count_nodes_on(1);
        let c2 = cluster.count_nodes_on(2);
        let c3 = cluster.count_nodes_on(3);
        if c1 >= 1 && c1 == c2 && c2 == c3 {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // Verify all original voters have the data
    let count_1 = cluster.count_nodes_on(1);
    let count_2 = cluster.count_nodes_on(2);
    let count_3 = cluster.count_nodes_on(3);
    assert!(count_1 >= 1, "node 1 must have data");
    assert_eq!(count_1, count_2, "nodes 1 and 2 must converge");
    assert_eq!(count_2, count_3, "nodes 2 and 3 must converge");

    cluster.shutdown().await;
}
