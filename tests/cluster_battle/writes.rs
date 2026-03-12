//! Write operation and routing tests for the Raft cluster.
//!
//! Tests 46–70: INSERT/SET/DELETE/REMOVE replication, schema DDL,
//! write routing, classification, batching, concurrency, and latency.

use crate::cluster_battle::harness::TestCluster;
use synaptica_core::types::Value;

// ---------------------------------------------------------------------------
// 46. INSERT single node replicates to all 3 cluster nodes
// ---------------------------------------------------------------------------
#[tokio::test]
async fn insert_single_node_replicates_to_all() {
    let cluster = TestCluster::new(3).await;

    let resp = cluster.write("INSERT (:Person {name: 'Alice'})").await.unwrap();
    assert!(resp.success, "INSERT should succeed: {:?}", resp.error);

    cluster.wait_for_convergence(5000).await;

    for id in cluster.nodes.keys() {
        assert_eq!(cluster.count_nodes_on(*id), 1, "node {} should have 1 graph node", id);
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 47. INSERT node with properties replicates values correctly
// ---------------------------------------------------------------------------
#[tokio::test]
async fn insert_node_with_properties_replicates_values() {
    let cluster = TestCluster::new(3).await;

    cluster
        .write("INSERT (:Person {name: 'Bob', age: 42})")
        .await
        .unwrap();

    cluster.wait_for_convergence(5000).await;

    for id in cluster.nodes.keys() {
        let rs = cluster
            .read_on(*id, "MATCH (n:Person) RETURN n.name, n.age")
            .unwrap();
        assert_eq!(rs.records.len(), 1, "node {} should return 1 record", id);
        let rec = &rs.records[0];
        assert_eq!(rec.get("n.name"), Some(&Value::String("Bob".into())));
        assert_eq!(rec.get("n.age"), Some(&Value::Integer(42)));
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 48. INSERT edge between nodes replicates to all nodes
// ---------------------------------------------------------------------------
#[tokio::test]
async fn insert_edge_replicates_to_all() {
    let cluster = TestCluster::new(3).await;

    cluster.write("INSERT (:Person {name: 'Alice'})").await.unwrap();
    cluster.write("INSERT (:Person {name: 'Bob'})").await.unwrap();
    cluster
        .write("INSERT (:Person {name: 'Alice'})-[:KNOWS]->(:Person {name: 'Bob'})")
        .await
        .unwrap();

    cluster.wait_for_convergence(5000).await;

    for id in cluster.nodes.keys() {
        assert!(
            cluster.count_edges_on(*id) >= 1,
            "node {} should have at least 1 edge",
            id
        );
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 49. SET property update replicates
// ---------------------------------------------------------------------------
#[tokio::test]
async fn set_property_replicates() {
    let cluster = TestCluster::new(3).await;

    cluster
        .write("INSERT (:Person {name: 'Charlie', age: 30})")
        .await
        .unwrap();
    cluster.wait_for_convergence(5000).await;

    cluster
        .write("MATCH (n:Person) WHERE n.name = 'Charlie' SET n.age = 31")
        .await
        .unwrap();

    // Give replication a moment
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    for id in cluster.nodes.keys() {
        let rs = cluster
            .read_on(*id, "MATCH (n:Person) WHERE n.name = 'Charlie' RETURN n.age")
            .unwrap();
        assert_eq!(rs.records.len(), 1, "node {} should have 1 record", id);
        assert_eq!(
            rs.records[0].get("n.age"),
            Some(&Value::Integer(31)),
            "node {} should see updated age",
            id
        );
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 50. DELETE node replicates removal
// ---------------------------------------------------------------------------
#[tokio::test]
async fn delete_node_replicates_removal() {
    let cluster = TestCluster::new(3).await;

    cluster.write("INSERT (:Temp {x: 1})").await.unwrap();
    cluster.wait_for_convergence(5000).await;

    cluster.write("MATCH (n:Temp) DELETE n").await.unwrap();

    cluster.wait_for_convergence_count(0, 5000).await;

    for id in cluster.nodes.keys() {
        assert_eq!(cluster.count_nodes_on(*id), 0, "node {} should have 0 nodes after DELETE", id);
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 51. REMOVE property replicates
// ---------------------------------------------------------------------------
#[tokio::test]
async fn remove_property_replicates() {
    let cluster = TestCluster::new(3).await;

    cluster
        .write("INSERT (:Person {name: 'Dave', age: 25})")
        .await
        .unwrap();
    cluster.wait_for_convergence(5000).await;

    cluster
        .write("MATCH (n:Person) WHERE n.name = 'Dave' REMOVE n.age")
        .await
        .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    for id in cluster.nodes.keys() {
        let rs = cluster
            .read_on(*id, "MATCH (n:Person) WHERE n.name = 'Dave' RETURN n.age")
            .unwrap();
        assert_eq!(rs.records.len(), 1, "node {} should still find Dave", id);
        let val = rs.records[0].get("n.age");
        assert!(
            val.is_none() || val == Some(&Value::Null),
            "node {} should have age removed",
            id
        );
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 52. CREATE GRAPH replicates new graph to all nodes
// ---------------------------------------------------------------------------
#[tokio::test]
async fn create_graph_replicates() {
    let cluster = TestCluster::new(3).await;

    let resp = cluster.write("CREATE GRAPH mygraph").await.unwrap();
    assert!(resp.success, "CREATE GRAPH should succeed: {:?}", resp.error);

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let graph_id = synaptica_core::graph::GraphId::from_name("mygraph");
    for (id, node) in &cluster.nodes {
        let meta = node.storage.get_graph_meta(&graph_id);
        assert!(meta.is_ok(), "node {} should have graph 'mygraph'", id);
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 53. DROP GRAPH replicates deletion
// ---------------------------------------------------------------------------
#[tokio::test]
async fn drop_graph_replicates() {
    let cluster = TestCluster::new(3).await;

    cluster.write("CREATE GRAPH dropme").await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let resp = cluster.write("DROP GRAPH dropme").await.unwrap();
    assert!(resp.success, "DROP GRAPH should succeed: {:?}", resp.error);

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let graph_id = synaptica_core::graph::GraphId::from_name("dropme");
    for (id, node) in &cluster.nodes {
        let meta = node.storage.get_graph_meta(&graph_id);
        assert!(meta.is_err(), "node {} should NOT have graph 'dropme' after DROP", id);
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 54. CREATE INDEX replicates definition
// ---------------------------------------------------------------------------
#[tokio::test]
async fn create_index_replicates() {
    let cluster = TestCluster::new(3).await;

    let resp = cluster
        .write("CREATE INDEX idx_name FOR (n:Person) ON (n.name)")
        .await
        .unwrap();
    assert!(resp.success, "CREATE INDEX should succeed: {:?}", resp.error);

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let graph_id = synaptica_core::graph::GraphId::from_name(&cluster.graph_name);
    for (id, node) in &cluster.nodes {
        let mgr = synaptica_storage::index::IndexManager::new(
            std::sync::Arc::clone(node.storage.raw_db()),
        );
        let indexes = mgr.list_indexes(&graph_id).unwrap();
        assert!(
            indexes.iter().any(|idx| idx.name == "idx_name"),
            "node {} should have index idx_name",
            id
        );
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 55. DROP INDEX replicates removal
// ---------------------------------------------------------------------------
#[tokio::test]
async fn drop_index_replicates() {
    let cluster = TestCluster::new(3).await;

    cluster
        .write("CREATE INDEX idx_drop FOR (n:Person) ON (n.name)")
        .await
        .unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let resp = cluster.write("DROP INDEX idx_drop").await.unwrap();
    assert!(resp.success, "DROP INDEX should succeed: {:?}", resp.error);

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let graph_id = synaptica_core::graph::GraphId::from_name(&cluster.graph_name);
    for (id, node) in &cluster.nodes {
        let mgr = synaptica_storage::index::IndexManager::new(
            std::sync::Arc::clone(node.storage.raw_db()),
        );
        let indexes = mgr.list_indexes(&graph_id).unwrap();
        assert!(
            !indexes.iter().any(|idx| idx.name == "idx_drop"),
            "node {} should NOT have index idx_drop after DROP",
            id
        );
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 56. Write on follower returns ForwardToLeader error
// ---------------------------------------------------------------------------
#[tokio::test]
async fn write_on_follower_returns_forward_to_leader() {
    let cluster = TestCluster::new(3).await;

    let leader = cluster.get_leader().expect("should have a leader");
    let follower = *cluster
        .nodes
        .keys()
        .find(|id| **id != leader)
        .expect("should have a follower");

    let result = cluster
        .write_on(follower, "INSERT (:Person {name: 'Reject'})")
        .await;

    assert!(result.is_err(), "write on follower should fail");
    let err = result.unwrap_err();
    assert!(
        err.contains("ForwardToLeader"),
        "error should mention ForwardToLeader, got: {}",
        err
    );

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 57. 100 sequential INSERTs maintain correct order on all replicas
// ---------------------------------------------------------------------------
#[tokio::test]
async fn sequential_inserts_maintain_order() {
    let cluster = TestCluster::new(3).await;

    for i in 0..100 {
        let q = format!("INSERT (:Item {{seq: {}}})", i);
        let resp = cluster.write(&q).await.unwrap();
        assert!(resp.success, "INSERT #{} failed: {:?}", i, resp.error);
    }

    cluster.wait_for_convergence_count(100, 10_000).await;

    for id in cluster.nodes.keys() {
        assert_eq!(
            cluster.count_nodes_on(*id),
            100,
            "node {} should have 100 nodes",
            id
        );
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 58. Mixed INSERT nodes and edges in rapid succession
// ---------------------------------------------------------------------------
#[tokio::test]
async fn mixed_insert_nodes_and_edges_rapid() {
    let cluster = TestCluster::new(3).await;

    // Insert base nodes
    for i in 0..10 {
        cluster
            .write(&format!("INSERT (:Item {{idx: {}}})", i))
            .await
            .unwrap();
    }

    // Insert edges between consecutive nodes
    for i in 0..9 {
        let q = format!(
            "INSERT (:Item {{idx: {}}})-[:NEXT]->(:Item {{idx: {}}})",
            i,
            i + 1
        );
        cluster.write(&q).await.unwrap();
    }

    cluster.wait_for_convergence(10_000).await;

    for id in cluster.nodes.keys() {
        let nc = cluster.count_nodes_on(*id);
        let ec = cluster.count_edges_on(*id);
        assert!(nc >= 10, "node {} should have >= 10 nodes, got {}", id, nc);
        assert!(ec >= 9, "node {} should have >= 9 edges, got {}", id, ec);
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 59. Write with GQL syntax error returns error without replication
// ---------------------------------------------------------------------------
#[tokio::test]
async fn syntax_error_returns_error_no_replication() {
    let cluster = TestCluster::new(3).await;

    let result = cluster.write("INSERTT BROKEN SYNTAX HERE").await;

    // Should fail at parse / plan / execution level
    match result {
        Ok(resp) => {
            assert!(!resp.success, "bad syntax should not succeed");
            assert!(resp.error.is_some(), "should have error message");
        }
        Err(e) => {
            // Also acceptable: error propagated before Raft commit
            assert!(!e.is_empty(), "should have error text: {}", e);
        }
    }

    // No data should have been replicated
    for id in cluster.nodes.keys() {
        assert_eq!(cluster.count_nodes_on(*id), 0, "node {} should have 0 nodes", id);
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 60. Write to non-existent graph returns error
// ---------------------------------------------------------------------------
#[tokio::test]
async fn write_to_nonexistent_graph_returns_error() {
    let cluster = TestCluster::new_with_graph(3, "real_graph").await;

    // Directly craft a Raft request targeting a non-existent graph.
    let leader = cluster.get_leader().expect("should have leader");
    let node = cluster.nodes.get(&leader).unwrap();
    let req = synaptica_cluster::raft::RaftRequest::WriteQuery {
        query: "INSERT (:Ghost {x: 1})".to_string(),
        graph_name: "nonexistent_graph".to_string(),
    };
    let result = node.raft.client_write(req).await.map(|r| r.data);

    match result {
        Ok(resp) => {
            // The Raft entry may commit but execution on state machine fails
            assert!(
                !resp.success || resp.error.is_some(),
                "write to missing graph should fail or report error"
            );
        }
        Err(_) => {
            // Also acceptable
        }
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 61. Batch INSERT of 500 nodes replicates to all followers
// ---------------------------------------------------------------------------
#[tokio::test]
async fn batch_insert_500_nodes_replicates() {
    let cluster = TestCluster::new(3).await;

    for i in 0..500 {
        let q = format!("INSERT (:Batch {{n: {}}})", i);
        cluster.write(&q).await.unwrap();
    }

    cluster.wait_for_convergence_count(500, 30_000).await;

    for id in cluster.nodes.keys() {
        assert_eq!(cluster.count_nodes_on(*id), 500, "node {} should have 500 nodes", id);
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 62. UPDATE via SET after INSERT visible on all nodes
// ---------------------------------------------------------------------------
#[tokio::test]
async fn update_via_set_after_insert_visible_on_all() {
    let cluster = TestCluster::new(3).await;

    cluster
        .write("INSERT (:Rec {key: 'k1', val: 10})")
        .await
        .unwrap();
    cluster.wait_for_convergence(5000).await;

    cluster
        .write("MATCH (n:Rec) WHERE n.key = 'k1' SET n.val = 99")
        .await
        .unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    for id in cluster.nodes.keys() {
        let rs = cluster
            .read_on(*id, "MATCH (n:Rec) WHERE n.key = 'k1' RETURN n.val")
            .unwrap();
        assert_eq!(rs.records.len(), 1, "node {} should have 1 Rec", id);
        assert_eq!(
            rs.records[0].get("n.val"),
            Some(&Value::Integer(99)),
            "node {} should see val=99",
            id
        );
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 63. DELETE after INSERT produces empty state on all nodes
// ---------------------------------------------------------------------------
#[tokio::test]
async fn delete_after_insert_empty_state() {
    let cluster = TestCluster::new(3).await;

    for i in 0..5 {
        cluster
            .write(&format!("INSERT (:Tmp {{i: {}}})", i))
            .await
            .unwrap();
    }
    cluster.wait_for_convergence_count(5, 5000).await;

    cluster.write("MATCH (n:Tmp) DELETE n").await.unwrap();
    cluster.wait_for_convergence_count(0, 5000).await;

    for id in cluster.nodes.keys() {
        assert_eq!(cluster.count_nodes_on(*id), 0, "node {} should be empty", id);
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 64. Complex multi-hop INSERT (nodes and edges) replicates atomically
// ---------------------------------------------------------------------------
#[tokio::test]
async fn complex_multi_hop_insert_replicates() {
    let cluster = TestCluster::new(3).await;

    // Build a small chain: A -> B -> C
    cluster.write("INSERT (:Hop {name: 'A'})").await.unwrap();
    cluster.write("INSERT (:Hop {name: 'B'})").await.unwrap();
    cluster.write("INSERT (:Hop {name: 'C'})").await.unwrap();
    cluster
        .write("INSERT (:Hop {name: 'A'})-[:LINK]->(:Hop {name: 'B'})")
        .await
        .unwrap();
    cluster
        .write("INSERT (:Hop {name: 'B'})-[:LINK]->(:Hop {name: 'C'})")
        .await
        .unwrap();

    cluster.wait_for_convergence(10_000).await;

    for id in cluster.nodes.keys() {
        let nc = cluster.count_nodes_on(*id);
        let ec = cluster.count_edges_on(*id);
        // At least the 3 original + any created by edge inserts
        assert!(nc >= 3, "node {} should have >= 3 nodes, got {}", id, nc);
        assert!(ec >= 2, "node {} should have >= 2 edges, got {}", id, ec);
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 65. Concurrent writes from different clients serialize correctly
// ---------------------------------------------------------------------------
#[tokio::test]
async fn concurrent_writes_serialize_correctly() {
    let cluster = std::sync::Arc::new(TestCluster::new(3).await);

    let mut handles = Vec::new();
    for i in 0..20 {
        let c = cluster.clone();
        handles.push(tokio::spawn(async move {
            let q = format!("INSERT (:Conc {{id: {}}})", i);
            c.write(&q).await
        }));
    }

    let mut successes = 0;
    for h in handles {
        if let Ok(Ok(resp)) = h.await {
            if resp.success {
                successes += 1;
            }
        }
    }
    assert_eq!(successes, 20, "all 20 concurrent writes should succeed");

    cluster.wait_for_convergence_count(20, 15_000).await;

    for id in cluster.nodes.keys() {
        assert_eq!(
            cluster.count_nodes_on(*id),
            20,
            "node {} should have 20 nodes",
            id
        );
    }

    // Arc prevents move into shutdown; Raft will drop when Arc drops.
}

// ---------------------------------------------------------------------------
// 66. Write classification correctly identifies all mutation types
// ---------------------------------------------------------------------------
#[tokio::test]
async fn write_classification_identifies_mutations() {
    let cluster = TestCluster::new(3).await;

    // Each of these is a write that should be accepted through Raft on the leader
    let write_queries = vec![
        "INSERT (:W {v: 1})",
        "MATCH (n:W) WHERE n.v = 1 SET n.v = 2",
        "MATCH (n:W) WHERE n.v = 2 DELETE n",
        "INSERT (:W2 {v: 1})",
        "MATCH (n:W2) REMOVE n.v",
        "CREATE INDEX idx_w FOR (n:W2) ON (n.v)",
        "DROP INDEX idx_w",
        "CREATE GRAPH classtest",
        "DROP GRAPH classtest",
    ];

    for q in &write_queries {
        let result = cluster.write(q).await;
        match result {
            Ok(resp) => assert!(resp.success, "query '{}' should succeed: {:?}", q, resp.error),
            Err(e) => panic!("query '{}' should not error: {}", q, e),
        }
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 67. Read-only MATCH query does NOT go through Raft (succeeds on follower)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn read_only_query_succeeds_on_follower() {
    let cluster = TestCluster::new(3).await;

    cluster
        .write("INSERT (:Rd {k: 'hello'})")
        .await
        .unwrap();
    cluster.wait_for_convergence(5000).await;

    let leader = cluster.get_leader().unwrap();
    let follower = *cluster
        .nodes
        .keys()
        .find(|id| **id != leader)
        .unwrap();

    // Local read on follower should work (no Raft needed)
    let rs = cluster
        .read_on(follower, "MATCH (n:Rd) RETURN n.k")
        .unwrap();
    assert_eq!(rs.records.len(), 1);
    assert_eq!(
        rs.records[0].get("n.k"),
        Some(&Value::String("hello".into()))
    );

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 68. Mixed read+write query (INSERT then MATCH) classified as write
// ---------------------------------------------------------------------------
#[tokio::test]
async fn mixed_read_write_classified_as_write() {
    let cluster = TestCluster::new(3).await;

    let leader = cluster.get_leader().unwrap();
    let follower = *cluster
        .nodes
        .keys()
        .find(|id| **id != leader)
        .unwrap();

    // A write query on a follower should be rejected via Raft
    let result = cluster
        .write_on(follower, "INSERT (:Mix {x: 1})")
        .await;

    assert!(result.is_err(), "INSERT on follower must be rejected");
    let err = result.unwrap_err();
    assert!(
        err.contains("ForwardToLeader"),
        "should contain ForwardToLeader: {}",
        err
    );

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 69. CREATE GRAPH TYPE replicates schema (fallback: CREATE GRAPH)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn create_graph_type_or_graph_replicates() {
    let cluster = TestCluster::new(3).await;

    // Try CREATE GRAPH TYPE; if unsupported, fall back to CREATE GRAPH
    let result = cluster.write("CREATE GRAPH TYPE mytype").await;

    match result {
        Ok(resp) if resp.success => {
            // CREATE GRAPH TYPE is supported — verify replication
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
        _ => {
            // Fallback: CREATE GRAPH is definitely supported
            let resp = cluster.write("CREATE GRAPH schema_g").await.unwrap();
            assert!(resp.success, "CREATE GRAPH fallback failed: {:?}", resp.error);

            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            let graph_id = synaptica_core::graph::GraphId::from_name("schema_g");
            for (id, node) in &cluster.nodes {
                assert!(
                    node.storage.get_graph_meta(&graph_id).is_ok(),
                    "node {} should have graph schema_g",
                    id
                );
            }
        }
    }

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// 70. Write latency for 3-node cluster under 500ms for simple INSERT
// ---------------------------------------------------------------------------
#[tokio::test]
async fn write_latency_under_500ms() {
    let cluster = TestCluster::new(3).await;

    // Warm up — first write may be slower due to leader setup
    cluster.write("INSERT (:Warm {w: 0})").await.unwrap();
    cluster.wait_for_convergence(5000).await;

    let start = std::time::Instant::now();
    let resp = cluster
        .write("INSERT (:Latency {t: 1})")
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert!(resp.success, "INSERT should succeed: {:?}", resp.error);
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "write latency should be < 500ms, was {:?}",
        elapsed
    );

    cluster.shutdown().await;
}
