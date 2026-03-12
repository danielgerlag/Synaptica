//! Raft consensus and leader election tests.

use openraft::ServerState;
use synaptica_cluster::raft::NodeId;

use crate::cluster_battle::harness::TestCluster;

// ---------------------------------------------------------------------------
// 1. Three-node cluster elects exactly one leader within timeout
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_01_three_node_elects_one_leader() {
    let cluster = TestCluster::new(3).await;

    let leader = cluster.get_leader();
    assert!(leader.is_some(), "cluster must elect a leader");

    let leader_id = leader.unwrap();
    let leader_count: usize = (1..=3)
        .filter(|&id| {
            let m = cluster.metrics(id);
            m.state == ServerState::Leader
        })
        .count();
    assert_eq!(leader_count, 1, "exactly one node must be leader, found {leader_count}");

    // The leader must believe it is its own leader
    let lm = cluster.metrics(leader_id);
    assert_eq!(lm.current_leader, Some(leader_id), "leader must report itself as current_leader");

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 2. Five-node cluster elects leader and all others are followers
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_02_five_node_leader_and_followers() {
    let cluster = TestCluster::new(5).await;

    let leader_id = cluster.get_leader().expect("cluster must elect a leader");

    for id in 1..=5u64 {
        let m = cluster.metrics(id);
        if id == leader_id {
            assert_eq!(m.state, ServerState::Leader, "node {id} should be Leader");
        } else {
            assert_eq!(m.state, ServerState::Follower, "node {id} should be Follower");
        }
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 3. Follower that stops receiving heartbeats triggers new election
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_03_block_leader_triggers_new_election() {
    let cluster = TestCluster::new(3).await;

    let old_leader = cluster.get_leader().expect("must have initial leader");

    // Partition the leader so followers stop receiving heartbeats
    cluster.router.block_node(old_leader).await;

    // Wait for a NEW leader to be elected among the non-blocked nodes
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(5000);
    let mut new_leader = None;
    while tokio::time::Instant::now() < deadline {
        for (&id, node) in &cluster.nodes {
            if id == old_leader {
                continue;
            }
            let m = node.raft.metrics().borrow().clone();
            if m.state == ServerState::Leader && m.current_leader == Some(id) {
                new_leader = Some(id);
            }
        }
        if new_leader.is_some() {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    assert!(new_leader.is_some(), "a new leader must be elected after blocking old leader {old_leader}");
    assert_ne!(new_leader.unwrap(), old_leader, "new leader must differ from blocked leader");

    cluster.router.unblock_node(old_leader).await;
    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 4. Node with higher term wins — after re-election, term increases
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_04_higher_term_after_reelection() {
    let cluster = TestCluster::new(3).await;

    let first_leader = cluster.get_leader().expect("must have initial leader");
    let term_before = cluster.metrics(first_leader).current_term;

    // Force a re-election by blocking the leader
    cluster.router.block_node(first_leader).await;

    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(5000);
    let mut new_leader = None;
    while tokio::time::Instant::now() < deadline {
        for (&id, node) in &cluster.nodes {
            if id == first_leader {
                continue;
            }
            let m = node.raft.metrics().borrow().clone();
            if m.state == ServerState::Leader && m.current_leader == Some(id) {
                new_leader = Some(id);
            }
        }
        if new_leader.is_some() {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }
    let new_leader = new_leader.expect("new leader must be elected");

    let term_after = cluster.metrics(new_leader).current_term;
    assert!(
        term_after > term_before,
        "term must increase after re-election: before={term_before}, after={term_after}"
    );

    cluster.router.unblock_node(first_leader).await;
    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 5. Follower rejects vote from candidate with lower term (verified via
//    metrics — after a clean election the follower term matches the leader)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_05_follower_term_matches_leader_after_election() {
    let cluster = TestCluster::new(3).await;

    let leader_id = cluster.get_leader().expect("must have leader");
    let leader_term = cluster.metrics(leader_id).current_term;

    // Every follower must be in the same term as the leader, meaning they
    // accepted the vote from the winning candidate (with the highest term)
    // and would have rejected any candidate with a lower term.
    for id in 1..=3u64 {
        let m = cluster.metrics(id);
        assert_eq!(
            m.current_term, leader_term,
            "node {id} term {} must equal leader term {leader_term} — a lower-term vote would not be accepted",
            m.current_term
        );
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 6. Node grants vote only once per term — single leader per term
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_06_single_leader_per_term() {
    let cluster = TestCluster::new(5).await;

    let leader_id = cluster.get_leader().expect("must have leader");
    let leader_term = cluster.metrics(leader_id).current_term;

    // Verify only one node is Leader in this term
    let leaders_in_term: Vec<NodeId> = (1..=5u64)
        .filter(|&id| {
            let m = cluster.metrics(id);
            m.state == ServerState::Leader && m.current_term == leader_term
        })
        .collect();

    assert_eq!(
        leaders_in_term.len(),
        1,
        "exactly one leader must exist for term {leader_term}, found {:?}",
        leaders_in_term
    );
    assert_eq!(leaders_in_term[0], leader_id);

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 7. Leader steps down when discovering higher-term message
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_07_leader_steps_down_on_higher_term() {
    let cluster = TestCluster::new(3).await;

    let old_leader = cluster.get_leader().expect("must have initial leader");

    // Block the leader so remaining nodes elect a new one in a higher term
    cluster.router.block_node(old_leader).await;

    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(5000);
    loop {
        let mut found_new = false;
        for (&id, node) in &cluster.nodes {
            if id == old_leader {
                continue;
            }
            let m = node.raft.metrics().borrow().clone();
            if m.state == ServerState::Leader && m.current_leader == Some(id) {
                found_new = true;
            }
        }
        if found_new || tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    // Unblock old leader — it will receive higher-term messages and step down
    cluster.router.unblock_node(old_leader).await;

    // Wait for old leader to become Follower
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(5000);
    let mut stepped_down = false;
    while tokio::time::Instant::now() < deadline {
        let m = cluster.metrics(old_leader);
        if m.state == ServerState::Follower {
            stepped_down = true;
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    assert!(stepped_down, "old leader {old_leader} must step down to Follower after seeing higher term");

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 8. Candidate converts to follower on receiving valid append_entries from
//    leader — verified by cluster converging to single leader naturally
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_08_candidate_converts_to_follower() {
    // With multiple nodes, several may become candidates simultaneously.
    // openraft guarantees that once a leader is established, the losing
    // candidates convert to Follower upon receiving the leader's
    // append_entries. We verify this end-state.
    let cluster = TestCluster::new(5).await;

    let leader_id = cluster.get_leader().expect("must have leader");

    let non_leaders: Vec<NodeId> = (1..=5u64).filter(|&id| id != leader_id).collect();
    for id in &non_leaders {
        let m = cluster.metrics(*id);
        assert!(
            m.state == ServerState::Follower,
            "node {} should be Follower (not {:?}) — candidates must convert on leader's append_entries",
            id,
            m.state
        );
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 9. Re-election after leader shutdown produces new leader
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_09_reelection_after_leader_shutdown() {
    let mut cluster = TestCluster::new(3).await;

    let old_leader = cluster.get_leader().expect("must have initial leader");

    // Shut down the leader's Raft instance and remove it from the cluster
    let leader_node = cluster.nodes.remove(&old_leader).unwrap();
    leader_node.raft.shutdown().await.ok();
    cluster.router.unregister(old_leader).await;

    // Wait for a new leader among remaining nodes
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(5000);
    let mut new_leader = None;
    while tokio::time::Instant::now() < deadline {
        for (_, node) in &cluster.nodes {
            let m = node.raft.metrics().borrow().clone();
            if let Some(l) = m.current_leader {
                if l != old_leader && cluster.nodes.contains_key(&l) {
                    new_leader = Some(l);
                }
            }
        }
        if new_leader.is_some() {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    assert!(new_leader.is_some(), "new leader must be elected after shutting down leader {old_leader}");
    assert_ne!(new_leader.unwrap(), old_leader);

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 10. Cluster of 3 remains available when 1 node is down
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_10_three_node_tolerates_one_failure() {
    let cluster = TestCluster::new(3).await;

    // Find a follower and block it
    let leader = cluster.get_leader().expect("must have leader");
    let follower = (1..=3u64).find(|&id| id != leader).unwrap();
    cluster.router.block_node(follower).await;

    // Cluster should still accept writes (2/3 quorum)
    let resp = cluster.write("INSERT (:Person {name: 'Alice'})").await;
    assert!(resp.is_ok(), "write must succeed with 2/3 quorum: {:?}", resp.err());
    let resp = resp.unwrap();
    assert!(resp.success, "write response must indicate success");

    cluster.router.unblock_node(follower).await;
    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 11. Cluster of 5 remains available when 2 nodes are down
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_11_five_node_tolerates_two_failures() {
    let cluster = TestCluster::new(5).await;

    let leader = cluster.get_leader().expect("must have leader");

    // Block 2 followers
    let followers: Vec<NodeId> = (1..=5u64).filter(|&id| id != leader).take(2).collect();
    for f in &followers {
        cluster.router.block_node(*f).await;
    }

    // Cluster should still accept writes (3/5 quorum)
    let resp = cluster.write("INSERT (:Person {name: 'Bob'})").await;
    assert!(resp.is_ok(), "write must succeed with 3/5 quorum: {:?}", resp.err());
    let resp = resp.unwrap();
    assert!(resp.success, "write response must indicate success");

    for f in &followers {
        cluster.router.unblock_node(*f).await;
    }
    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 12. Cluster of 5 loses quorum when 3 nodes are down (write should fail)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_12_five_node_loses_quorum_with_three_down() {
    let cluster = TestCluster::new(5).await;

    let leader = cluster.get_leader().expect("must have leader");

    // Block 3 non-leader nodes — leader + 1 remain, but that is only 2/5
    let to_block: Vec<NodeId> = (1..=5u64).filter(|&id| id != leader).take(3).collect();
    for f in &to_block {
        cluster.router.block_node(*f).await;
    }

    // Write should fail or time out (no quorum)
    let result = tokio::time::timeout(
        tokio::time::Duration::from_millis(3000),
        cluster.write("INSERT (:Person {name: 'Charlie'})"),
    )
    .await;

    let write_failed = match result {
        Err(_) => true,                       // timed out
        Ok(Err(_)) => true,                   // Raft returned error
        Ok(Ok(resp)) => !resp.success,        // write rejected
    };
    assert!(write_failed, "write must fail when quorum is lost (3 of 5 nodes down)");

    for f in &to_block {
        cluster.router.unblock_node(*f).await;
    }
    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 13. Multiple rapid leader changes maintain data consistency
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_13_rapid_leader_changes_keep_data_consistent() {
    let cluster = TestCluster::new(5).await;

    let mut total_inserts = 0u64;

    for round in 0..3u64 {
        // Write data via current leader
        let query = format!("INSERT (:Item {{round: {}}})", round);
        let resp = cluster.write(&query).await;
        assert!(resp.is_ok(), "write round {round} must succeed: {:?}", resp.err());
        assert!(resp.unwrap().success, "write round {round} must report success");
        total_inserts += 1;

        // Force re-election by blocking current leader
        let current_leader = cluster.get_leader().expect("must have leader");
        cluster.router.block_node(current_leader).await;

        // Wait for new leader among non-blocked nodes
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(5000);
        let mut new_leader = None;
        while tokio::time::Instant::now() < deadline {
            for (&id, node) in &cluster.nodes {
                if id == current_leader {
                    continue;
                }
                let m = node.raft.metrics().borrow().clone();
                if m.state == ServerState::Leader && m.current_leader == Some(id) {
                    new_leader = Some(id);
                }
            }
            if new_leader.is_some() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
        assert!(new_leader.is_some(), "new leader must emerge in round {round}");

        // Unblock old leader so it can rejoin as follower
        cluster.router.unblock_node(current_leader).await;

        // Allow cluster to stabilise before next round
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    }

    // Wait for convergence across all (now-unblocked) nodes
    cluster
        .wait_for_convergence_count(total_inserts as usize, 5000)
        .await;

    // Verify every node has the expected count
    for id in 1..=5u64 {
        let count = cluster.count_nodes_on(id);
        assert_eq!(
            count, total_inserts as usize,
            "node {id} has {count} nodes, expected {total_inserts} after 3 leader changes"
        );
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 14. Election with all nodes starting simultaneously produces single leader
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_14_simultaneous_start_produces_single_leader() {
    // TestCluster::new starts all nodes together and waits for leader,
    // which inherently validates that simultaneous startup converges.
    let cluster = TestCluster::new(5).await;

    let leader = cluster.get_leader();
    assert!(leader.is_some(), "cluster must elect a single leader from simultaneous start");

    let leader_count = (1..=5u64)
        .filter(|&id| cluster.metrics(id).state == ServerState::Leader)
        .count();
    assert_eq!(leader_count, 1, "exactly one leader must exist after simultaneous start");

    // All non-leader nodes must be followers (not candidates stuck mid-election)
    let follower_count = (1..=5u64)
        .filter(|&id| cluster.metrics(id).state == ServerState::Follower)
        .count();
    assert_eq!(follower_count, 4, "all 4 non-leader nodes must be followers");

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 15. Leader's metrics correctly report current_leader as self
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_15_leader_metrics_report_self() {
    let cluster = TestCluster::new(3).await;

    let leader_id = cluster.get_leader().expect("must have leader");
    let m = cluster.metrics(leader_id);

    assert_eq!(m.state, ServerState::Leader, "leader node must report Leader state");
    assert_eq!(
        m.current_leader,
        Some(leader_id),
        "leader's current_leader metric must be its own id ({leader_id})"
    );
    assert!(m.current_term > 0, "leader's term must be positive");

    // Followers should also agree on who the leader is
    for id in 1..=3u64 {
        if id != leader_id {
            let fm = cluster.metrics(id);
            assert_eq!(
                fm.current_leader,
                Some(leader_id),
                "follower {id} must report current_leader = {leader_id}"
            );
        }
    }

    cluster.shutdown().await;
}
