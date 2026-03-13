//! Query read tests.
//!
//! Tests 71–80: Local MATCH reads on followers, consistency with leader,
//! filtered/sorted queries, aggregations, path traversal, schema queries,
//! index-backed scans, and concurrent reads.

use crate::cluster_battle::harness::TestCluster;
use synaptica_core::types::Value;

// ---------------------------------------------------------------------------
// 71. MATCH query executes locally on follower without Raft
// ---------------------------------------------------------------------------
#[tokio::test]
async fn match_query_executes_on_follower() {
    let cluster = TestCluster::new(3).await;

    cluster
        .write("INSERT (:Person {name: 'Alice'})")
        .await
        .unwrap();
    cluster.wait_for_convergence(5000).await;

    let leader = cluster.get_leader().unwrap();
    let follower = (1..=3u64).find(|&id| id != leader).unwrap();

    let rs = cluster
        .read_on(follower, "MATCH (n:Person) RETURN n.name")
        .unwrap();
    assert_eq!(rs.records.len(), 1, "follower should return 1 record");
    assert_eq!(
        rs.records[0].get("n.name"),
        Some(&Value::String("Alice".into()))
    );

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 72. Read on follower returns same data as leader after replication
// ---------------------------------------------------------------------------
#[tokio::test]
async fn read_on_follower_matches_leader() {
    let cluster = TestCluster::new(3).await;

    cluster
        .write("INSERT (:Person {name: 'Alice', age: 30})")
        .await
        .unwrap();
    cluster
        .write("INSERT (:Person {name: 'Bob', age: 25})")
        .await
        .unwrap();
    cluster.wait_for_convergence(5000).await;

    let leader = cluster.get_leader().unwrap();
    let follower = (1..=3u64).find(|&id| id != leader).unwrap();

    let query = "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.name";
    let leader_rs = cluster.read_on(leader, query).unwrap();
    let follower_rs = cluster.read_on(follower, query).unwrap();

    assert_eq!(
        leader_rs.records.len(),
        follower_rs.records.len(),
        "leader and follower must return same number of records"
    );
    for (i, (lr, fr)) in leader_rs
        .records
        .iter()
        .zip(follower_rs.records.iter())
        .enumerate()
    {
        assert_eq!(lr.get("n.name"), fr.get("n.name"), "row {i} name mismatch");
        assert_eq!(lr.get("n.age"), fr.get("n.age"), "row {i} age mismatch");
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 73. MATCH with WHERE filter works on follower
// ---------------------------------------------------------------------------
#[tokio::test]
async fn match_with_where_filter_on_follower() {
    let cluster = TestCluster::new(3).await;

    cluster
        .write("INSERT (:Person {name: 'Alice', age: 30})")
        .await
        .unwrap();
    cluster
        .write("INSERT (:Person {name: 'Bob', age: 25})")
        .await
        .unwrap();
    cluster
        .write("INSERT (:Person {name: 'Charlie', age: 35})")
        .await
        .unwrap();
    cluster.wait_for_convergence(5000).await;

    let leader = cluster.get_leader().unwrap();
    let follower = (1..=3u64).find(|&id| id != leader).unwrap();

    let rs = cluster
        .read_on(
            follower,
            "MATCH (n:Person) WHERE n.age > 28 RETURN n.name ORDER BY n.name",
        )
        .unwrap();
    assert_eq!(
        rs.records.len(),
        2,
        "should match Alice (30) and Charlie (35)"
    );
    assert_eq!(
        rs.records[0].get("n.name"),
        Some(&Value::String("Alice".into()))
    );
    assert_eq!(
        rs.records[1].get("n.name"),
        Some(&Value::String("Charlie".into()))
    );

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 74. MATCH with ORDER BY and LIMIT on follower
// ---------------------------------------------------------------------------
#[tokio::test]
async fn match_with_order_by_and_limit_on_follower() {
    let cluster = TestCluster::new(3).await;

    for i in 0..10 {
        cluster
            .write(&format!("INSERT (:Item {{name: 'item_{:02}'}})", i))
            .await
            .unwrap();
    }
    cluster.wait_for_convergence(5000).await;

    let leader = cluster.get_leader().unwrap();
    let follower = (1..=3u64).find(|&id| id != leader).unwrap();

    let rs = cluster
        .read_on(
            follower,
            "MATCH (n:Item) RETURN n.name ORDER BY n.name LIMIT 5",
        )
        .unwrap();

    assert_eq!(
        rs.records.len(),
        5,
        "LIMIT 5 should return exactly 5 records"
    );
    for (i, rec) in rs.records.iter().enumerate() {
        let expected = format!("item_{:02}", i);
        assert_eq!(
            rec.get("n.name"),
            Some(&Value::String(expected.clone())),
            "record {i} should be {expected}"
        );
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 75. Aggregation query (COUNT) on follower matches leader
// ---------------------------------------------------------------------------
#[tokio::test]
async fn count_aggregation_on_follower_matches_leader() {
    let cluster = TestCluster::new(3).await;

    for i in 0..7 {
        cluster
            .write(&format!("INSERT (:Widget {{idx: {}}})", i))
            .await
            .unwrap();
    }
    cluster.wait_for_convergence(5000).await;

    let leader = cluster.get_leader().unwrap();
    let follower = (1..=3u64).find(|&id| id != leader).unwrap();

    let leader_rs = cluster
        .read_on(leader, "MATCH (n:Widget) RETURN COUNT(n)")
        .unwrap();
    let follower_rs = cluster
        .read_on(follower, "MATCH (n:Widget) RETURN COUNT(n)")
        .unwrap();

    assert_eq!(leader_rs.records.len(), 1);
    assert_eq!(follower_rs.records.len(), 1);

    let leader_count = &leader_rs.records[0].values[0];
    let follower_count = &follower_rs.records[0].values[0];
    assert_eq!(
        leader_count, follower_count,
        "COUNT must match: leader={leader_count:?}, follower={follower_count:?}"
    );
    assert_eq!(leader_count, &Value::Integer(7));

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 76. Multi-hop path traversal on follower
// ---------------------------------------------------------------------------
#[tokio::test]
async fn multi_hop_path_traversal_on_follower() {
    let cluster = TestCluster::new(3).await;

    // Create a chain: A -> B -> C
    cluster.write("INSERT (:Hop {name: 'A'})").await.unwrap();
    cluster.write("INSERT (:Hop {name: 'B'})").await.unwrap();
    cluster.write("INSERT (:Hop {name: 'C'})").await.unwrap();
    cluster
        .write("MATCH (a:Hop {name: 'A'}), (b:Hop {name: 'B'}) INSERT (a)-[:LINK]->(b)")
        .await
        .unwrap();
    cluster
        .write("MATCH (b:Hop {name: 'B'}), (c:Hop {name: 'C'}) INSERT (b)-[:LINK]->(c)")
        .await
        .unwrap();
    cluster.wait_for_convergence(5000).await;

    let leader = cluster.get_leader().unwrap();
    let follower = (1..=3u64).find(|&id| id != leader).unwrap();

    let rs = cluster
        .read_on(
            follower,
            "MATCH (a:Hop)-[:LINK]->(b:Hop)-[:LINK]->(c:Hop) RETURN a.name, b.name, c.name",
        )
        .unwrap();

    assert_eq!(rs.records.len(), 1, "should find exactly one A->B->C path");
    assert_eq!(
        rs.records[0].get("a.name"),
        Some(&Value::String("A".into()))
    );
    assert_eq!(
        rs.records[0].get("b.name"),
        Some(&Value::String("B".into()))
    );
    assert_eq!(
        rs.records[0].get("c.name"),
        Some(&Value::String("C".into()))
    );

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 77. Schema query (labels) on follower matches leader
// ---------------------------------------------------------------------------
#[tokio::test]
async fn label_query_on_follower_matches_leader() {
    let cluster = TestCluster::new(3).await;

    cluster.write("INSERT (:Dog {name: 'Rex'})").await.unwrap();
    cluster
        .write("INSERT (:Cat {name: 'Whiskers'})")
        .await
        .unwrap();
    cluster
        .write("INSERT (:Bird {name: 'Tweety'})")
        .await
        .unwrap();
    cluster.wait_for_convergence(5000).await;

    let leader = cluster.get_leader().unwrap();
    let follower = (1..=3u64).find(|&id| id != leader).unwrap();

    for label in &["Dog", "Cat", "Bird"] {
        let query = format!("MATCH (n:{}) RETURN n.name", label);
        let lr = cluster.read_on(leader, &query).unwrap();
        let fr = cluster.read_on(follower, &query).unwrap();
        assert_eq!(
            lr.records.len(),
            fr.records.len(),
            "label {label} count mismatch between leader and follower"
        );
        assert_eq!(lr.records.len(), 1, "should have 1 {label} node");
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 78. Index-backed scan works on follower after index replication
// ---------------------------------------------------------------------------
#[tokio::test]
async fn index_backed_scan_on_follower() {
    use synaptica_core::graph::GraphId;
    let cluster = TestCluster::new(3).await;

    cluster
        .write("CREATE INDEX idx_person_name FOR (p:Person) ON (p.name)")
        .await
        .unwrap();
    cluster
        .write("INSERT (:Person {name: 'Alice'})")
        .await
        .unwrap();
    cluster
        .write("INSERT (:Person {name: 'Bob'})")
        .await
        .unwrap();
    cluster.wait_for_convergence(5000).await;

    let leader = cluster.get_leader().unwrap();
    let follower = (1..=3u64).find(|&id| id != leader).unwrap();

    // Verify index definition replicated to follower
    let follower_node = cluster.nodes.get(&follower).unwrap();
    let graph_id = GraphId::from_name("test");
    let indexes = follower_node.storage.list_indexes(&graph_id).unwrap();
    assert!(
        indexes.iter().any(|idx| idx.name == "idx_person_name"),
        "index must be replicated to follower, found: {:?}",
        indexes.iter().map(|i| &i.name).collect::<Vec<_>>()
    );

    // Query on follower should work (potentially index-backed)
    let rs = cluster
        .read_on(
            follower,
            "MATCH (n:Person) WHERE n.name = 'Alice' RETURN n.name",
        )
        .unwrap();
    assert_eq!(rs.records.len(), 1);
    assert_eq!(
        rs.records[0].get("n.name"),
        Some(&Value::String("Alice".into()))
    );

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 79. Read during active write still returns consistent data
// ---------------------------------------------------------------------------
#[tokio::test]
async fn read_during_active_write_is_consistent() {
    let cluster = TestCluster::new(3).await;

    // Pre-seed data
    for i in 0..5 {
        cluster
            .write(&format!("INSERT (:Seed {{idx: {}}})", i))
            .await
            .unwrap();
    }
    cluster.wait_for_convergence(5000).await;

    let leader = cluster.get_leader().unwrap();
    let follower = (1..=3u64).find(|&id| id != leader).unwrap();

    // Write a batch of additional data
    for i in 5..15 {
        cluster
            .write(&format!("INSERT (:Seed {{idx: {}}})", i))
            .await
            .unwrap();
    }

    // Read on follower mid-replication — must return a valid count
    let rs = cluster
        .read_on(follower, "MATCH (n:Seed) RETURN n.idx")
        .unwrap();
    assert!(
        rs.records.len() >= 5,
        "follower must see at least the pre-seeded 5 nodes, got {}",
        rs.records.len()
    );
    assert!(
        rs.records.len() <= 15,
        "follower must not see more than 15 nodes, got {}",
        rs.records.len()
    );

    // After full convergence, all 15 must be visible
    cluster.wait_for_convergence_count(15, 10_000).await;
    let rs2 = cluster
        .read_on(follower, "MATCH (n:Seed) RETURN n.idx")
        .unwrap();
    assert_eq!(
        rs2.records.len(),
        15,
        "after convergence follower must see all 15 nodes"
    );

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 80. Multiple concurrent reads on different nodes return same results
// ---------------------------------------------------------------------------
#[tokio::test]
async fn concurrent_reads_on_all_nodes_agree() {
    let cluster = TestCluster::new(3).await;

    for i in 0..5 {
        cluster
            .write(&format!("INSERT (:Item {{idx: {}}})", i))
            .await
            .unwrap();
    }
    cluster.wait_for_convergence(5000).await;

    let node_ids: Vec<u64> = cluster.nodes.keys().copied().collect();
    let cluster = std::sync::Arc::new(cluster);

    let mut handles = vec![];
    for id in node_ids.clone() {
        let c = cluster.clone();
        handles.push(tokio::spawn(async move {
            c.read_on(id, "MATCH (n:Item) RETURN n.idx ORDER BY n.idx")
                .unwrap()
        }));
    }

    let mut results = vec![];
    for h in handles {
        results.push(h.await.unwrap());
    }

    let expected_len = results[0].records.len();
    assert_eq!(expected_len, 5, "each node should have 5 records");

    for (i, rs) in results.iter().enumerate() {
        assert_eq!(
            rs.records.len(),
            expected_len,
            "node {} returned {} records, expected {expected_len}",
            node_ids[i],
            rs.records.len()
        );
    }

    // Compare projected values (n.idx) across all nodes — internal IDs may differ
    let col_idx = results[0]
        .columns
        .iter()
        .position(|c| c == "n.idx")
        .expect("n.idx column must exist");
    for row in 0..expected_len {
        let baseline = &results[0].records[row].values[col_idx];
        for (i, rs) in results.iter().enumerate().skip(1) {
            assert_eq!(
                &rs.records[row].values[col_idx], baseline,
                "node {} row {row} n.idx differs from node {}",
                node_ids[i], node_ids[0]
            );
        }
    }

    match std::sync::Arc::try_unwrap(cluster) {
        Ok(c) => c.shutdown().await,
        Err(_) => panic!("all handles should have completed"),
    }
}
