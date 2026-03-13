//! Failure and recovery tests.

use crate::cluster_battle::harness::TestCluster;

/// Test 91: Leader crash triggers re-election within timeout.
///
/// Block the current leader and verify that the remaining nodes elect
/// a new, different leader within a bounded time window.
#[tokio::test]
async fn test_91_leader_crash_triggers_reelection() {
    let cluster = TestCluster::new(3).await;

    let old_leader = cluster.get_leader().unwrap();
    cluster.router.block_node(old_leader).await;

    // Wait for new leader (different from old)
    let start = tokio::time::Instant::now();
    loop {
        if start.elapsed() > tokio::time::Duration::from_secs(5) {
            panic!("no new leader elected within timeout");
        }
        if let Some(new_leader) = cluster.get_leader() {
            if new_leader != old_leader {
                break;
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    let new_leader = cluster.get_leader().unwrap();
    assert_ne!(new_leader, old_leader);

    cluster.router.unblock_node(old_leader).await;
    cluster.shutdown().await;
}

/// Test 92: Follower crash and restart rejoins cluster and catches up.
///
/// Block a follower, write data while it is down, unblock it, then
/// verify the follower replicates the missed writes.
#[tokio::test]
async fn test_92_follower_crash_restart_catches_up() {
    let cluster = TestCluster::new(3).await;

    let leader = cluster.get_leader().unwrap();
    let follower = (1..=3u64).find(|&id| id != leader).unwrap();

    cluster.router.block_node(follower).await;

    let resp = cluster
        .write("INSERT (:Person {name: 'Alice'})")
        .await
        .unwrap();
    assert!(resp.success);

    cluster.router.unblock_node(follower).await;

    cluster.wait_for_convergence_count(1, 10_000).await;

    assert_eq!(cluster.count_nodes_on(follower), 1);

    cluster.shutdown().await;
}

/// Test 93: Network timeout between nodes produces a clean error, not a panic.
///
/// Block every node so no quorum exists, then attempt a write. The
/// operation should return an error or time out — never panic.
#[tokio::test]
async fn test_93_network_timeout_clean_error() {
    let cluster = TestCluster::new(3).await;

    for id in 1..=3u64 {
        cluster.router.block_node(id).await;
    }

    let result = tokio::time::timeout(
        tokio::time::Duration::from_secs(3),
        cluster.write("INSERT (:X {})"),
    )
    .await;

    match result {
        Ok(Ok(resp)) => {
            assert!(
                !resp.success || resp.error.is_some(),
                "write should not succeed when all nodes are blocked"
            );
        }
        Ok(Err(_)) => {} // write returned an error — expected
        Err(_) => {}     // timeout elapsed — also acceptable
    }

    for id in 1..=3u64 {
        cluster.router.unblock_node(id).await;
    }
    cluster.shutdown().await;
}

/// Test 94: Two-node minority partition cannot elect a leader.
///
/// In a 5-node cluster, block 3 nodes (including the current leader).
/// The remaining 2 nodes lack quorum (need 3/5) and must not elect.
#[tokio::test]
async fn test_94_minority_partition_cannot_elect_leader() {
    let cluster = TestCluster::new(5).await;

    let old_leader = cluster.get_leader().unwrap();

    // Block the leader plus two others — leaves exactly 2 unblocked.
    let mut blocked = vec![old_leader];
    for id in 1..=5u64 {
        if id != old_leader && blocked.len() < 3 {
            blocked.push(id);
        }
    }
    for &id in &blocked {
        cluster.router.block_node(id).await;
    }

    let unblocked: Vec<u64> = (1..=5u64).filter(|id| !blocked.contains(id)).collect();

    // Give the minority time to attempt (and fail) an election.
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // No unblocked node should consider itself leader.
    for &id in &unblocked {
        let metrics = cluster.metrics(id);
        assert!(
            metrics.current_leader != Some(id),
            "node {} in the minority partition should not be leader",
            id
        );
    }

    for &id in &blocked {
        cluster.router.unblock_node(id).await;
    }
    cluster.shutdown().await;
}

/// Test 95: Healed network partition allows stale leader to step down.
///
/// Partition the leader from the majority, wait for a new leader,
/// then heal the partition and verify the old leader stepped down.
#[tokio::test]
async fn test_95_healed_partition_stale_leader_steps_down() {
    let cluster = TestCluster::new(5).await;

    let old_leader = cluster.get_leader().unwrap();
    cluster.router.block_node(old_leader).await;

    // Wait for a new leader among the remaining 4 nodes.
    let new_leader = {
        let start = tokio::time::Instant::now();
        loop {
            if start.elapsed() > tokio::time::Duration::from_secs(10) {
                panic!("no new leader elected after partitioning old leader");
            }
            if let Some(leader) = cluster.get_leader() {
                if leader != old_leader {
                    break leader;
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    };
    assert_ne!(new_leader, old_leader);

    // Heal the partition.
    cluster.router.unblock_node(old_leader).await;

    // Allow the old leader to discover the higher term and step down.
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    let current_leader = cluster.get_leader().unwrap();
    assert_ne!(
        current_leader, old_leader,
        "stale leader should have stepped down after partition healed"
    );

    cluster.shutdown().await;
}

/// Test 96: Writes committed before a crash survive recovery.
///
/// Write data, replicate it, crash a follower, write more data,
/// then recover the follower and confirm the original data persists.
#[tokio::test]
async fn test_96_committed_writes_survive_recovery() {
    let cluster = TestCluster::new(3).await;

    let resp = cluster
        .write("INSERT (:Person {name: 'PreCrash'})")
        .await
        .unwrap();
    assert!(resp.success);
    cluster.wait_for_convergence(5000).await;

    let leader = cluster.get_leader().unwrap();
    let follower = (1..=3u64).find(|&id| id != leader).unwrap();

    assert_eq!(cluster.count_nodes_on(follower), 1);

    cluster.router.block_node(follower).await;

    let resp2 = cluster
        .write("INSERT (:Person {name: 'DuringCrash'})")
        .await
        .unwrap();
    assert!(resp2.success);

    cluster.router.unblock_node(follower).await;

    cluster.wait_for_convergence_count(2, 10_000).await;

    // Pre-crash data must survive.
    assert!(
        cluster.count_nodes_on(follower) >= 1,
        "pre-crash data must survive recovery"
    );
    // Both writes should eventually be on the follower.
    assert_eq!(cluster.count_nodes_on(follower), 2);

    cluster.shutdown().await;
}

/// Test 97: Uncommitted writes during quorum loss are not visible.
///
/// Block 2 of 3 nodes so the remaining leader cannot commit. Attempt
/// a write (which should fail), then unblock and verify no ghost data.
#[tokio::test]
async fn test_97_uncommitted_writes_not_visible() {
    let cluster = TestCluster::new(3).await;

    let leader = cluster.get_leader().unwrap();
    let followers: Vec<u64> = (1..=3u64).filter(|&id| id != leader).collect();

    for &f in &followers {
        cluster.router.block_node(f).await;
    }

    // Attempt a write — should fail or time out (no quorum).
    let write_result = tokio::time::timeout(
        tokio::time::Duration::from_secs(3),
        cluster.write("INSERT (:Ghost {name: 'ShouldNotExist'})"),
    )
    .await;

    let write_succeeded = matches!(write_result, Ok(Ok(ref r)) if r.success);
    assert!(!write_succeeded, "write without quorum should not succeed");

    for &f in &followers {
        cluster.router.unblock_node(f).await;
    }

    cluster.wait_for_leader(5000).await;
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    for id in 1..=3u64 {
        assert_eq!(
            cluster.count_nodes_on(id),
            0,
            "uncommitted write should not be visible on node {}",
            id
        );
    }

    cluster.shutdown().await;
}

/// Test 98: Rapid leader kill/restart cycle maintains data integrity.
///
/// Repeatedly block and unblock the leader 5 times, writing between
/// each cycle. All writes must be present on every node at the end.
#[tokio::test]
async fn test_98_rapid_leader_kill_restart_data_integrity() {
    let cluster = TestCluster::new(3).await;

    for i in 0..5 {
        cluster.wait_for_leader(5000).await;

        let query = format!("INSERT (:Item {{cycle: {}}})", i);
        let resp = cluster.write(&query).await.unwrap();
        assert!(resp.success, "write failed on cycle {}", i);

        let leader = cluster.get_leader().unwrap();
        cluster.router.block_node(leader).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        cluster.router.unblock_node(leader).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    cluster.wait_for_leader(5000).await;
    cluster.wait_for_convergence_count(5, 15_000).await;

    for id in 1..=3u64 {
        assert_eq!(
            cluster.count_nodes_on(id),
            5,
            "node {} should have all 5 writes after rapid kill/restart cycles",
            id
        );
    }

    cluster.shutdown().await;
}

/// Test 99: All 5 nodes crash and restart — cluster reforms correctly.
///
/// Block all 5 nodes (Raft instances stay in memory), unblock them
/// in staggered fashion, and verify the cluster elects a leader.
#[tokio::test]
async fn test_99_all_nodes_crash_restart_reforms() {
    let cluster = TestCluster::new(5).await;

    // Write some data before the crash
    cluster.write("INSERT (:PreCrash {v: 1})").await.unwrap();
    cluster.wait_for_convergence(5000).await;

    // Block all nodes
    for id in 1..=5u64 {
        cluster.router.block_node(id).await;
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Unblock all nodes at once
    for id in 1..=5u64 {
        cluster.router.unblock_node(id).await;
    }

    // Give extra time for 5-node election after total partition
    cluster.wait_for_leader(20_000).await;
    // Extra stabilization time for all nodes to learn the new leader
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Try write — may need to find the actual leader
    let mut write_ok = false;
    for _ in 0..10 {
        let leader = cluster.get_leader();
        if leader.is_none() {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            continue;
        }
        match cluster.write("INSERT (:Survivor {})").await {
            Ok(resp) if resp.success => {
                write_ok = true;
                break;
            }
            _ => {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }
    }
    assert!(write_ok, "cluster should accept writes after full restart");

    // Verify pre-crash data survived and new write replicated
    let start = tokio::time::Instant::now();
    loop {
        if start.elapsed() > tokio::time::Duration::from_secs(15) {
            let counts: Vec<_> = (1..=5u64)
                .map(|id| (id, cluster.count_nodes_on(id)))
                .collect();
            panic!("convergence timeout after restart. counts: {:?}", counts);
        }
        let all_have_2 = (1..=5u64).all(|id| cluster.count_nodes_on(id) == 2);
        if all_have_2 {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    cluster.shutdown().await;
}

/// Test 100: Cluster handles 1000 writes then survives leader failover
/// without data loss.
///
/// Insert 1000 graph nodes, wait for replication, kill the leader,
/// verify a new leader is elected and it has all 1000 nodes.
#[tokio::test]
async fn test_100_1000_writes_leader_failover_no_data_loss() {
    let cluster = TestCluster::new(3).await;

    for i in 0..1000 {
        let query = format!("INSERT (:N {{i: {}}})", i);
        let resp = cluster.write(&query).await.unwrap();
        assert!(resp.success, "write {} failed", i);
    }

    cluster.wait_for_convergence_count(1000, 30_000).await;

    let old_leader = cluster.get_leader().unwrap();
    cluster.router.block_node(old_leader).await;

    let new_leader = {
        let start = tokio::time::Instant::now();
        loop {
            if start.elapsed() > tokio::time::Duration::from_secs(5) {
                panic!("no new leader elected after killing leader");
            }
            if let Some(leader) = cluster.get_leader() {
                if leader != old_leader {
                    break leader;
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
    };
    assert_ne!(new_leader, old_leader);

    assert_eq!(
        cluster.count_nodes_on(new_leader),
        1000,
        "new leader must have all 1000 writes after failover"
    );

    cluster.router.unblock_node(old_leader).await;
    cluster.shutdown().await;
}
