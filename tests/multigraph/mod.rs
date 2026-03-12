//! Multi-graph integration tests for Synaptica.
//!
//! Verifies CREATE GRAPH, DROP GRAPH, LIST GRAPHS, data isolation,
//! and graph switching via the execution engine.

use synaptica_core::graph::{GraphId, GraphMeta};
use synaptica_core::types::Value;
use synaptica_exec::engine::ExecutionEngine;
use synaptica_exec::result::ResultSet;
use synaptica_gql::parser::parse;
use synaptica_gql::planner::QueryPlanner;
use synaptica_storage::engine::{StorageConfig, StorageEngine};

// ===========================================================================
// Helpers
// ===========================================================================

struct MultiGraphEnv {
    storage: StorageEngine,
    _dir: tempfile::TempDir,
}

impl MultiGraphEnv {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let storage =
            StorageEngine::open(dir.path(), &StorageConfig::default()).expect("open storage");
        MultiGraphEnv { storage, _dir: dir }
    }

    fn exec(&self, graph_name: &str, query: &str) -> ResultSet {
        let graph_id = GraphId::from_name(graph_name);
        let program = parse(query).expect("parse");
        let planner = QueryPlanner::new();
        let plan = planner.plan(&program).expect("plan");
        let engine = ExecutionEngine::new(&self.storage);
        engine.execute_plan(&plan, &graph_id).expect("execute")
    }

    fn exec_result(&self, graph_name: &str, query: &str) -> Result<ResultSet, String> {
        let graph_id = GraphId::from_name(graph_name);
        let program = parse(query).map_err(|e| format!("{e}"))?;
        let planner = QueryPlanner::new();
        let plan = planner.plan(&program).map_err(|e| format!("{e}"))?;
        let engine = ExecutionEngine::new(&self.storage);
        engine.execute_plan(&plan, &graph_id).map_err(|e| format!("{e}"))
    }

    fn create_graph(&self, name: &str) {
        let graph_id = GraphId::from_name(name);
        let meta = GraphMeta {
            id: graph_id,
            name: name.to_string(),
            graph_type: None,
        };
        self.storage.put_graph_meta(&meta).unwrap();
    }

    fn list_graphs(&self) -> Vec<GraphMeta> {
        self.storage.list_graphs().unwrap()
    }
}

fn col_values(rs: &ResultSet, col: &str) -> Vec<Value> {
    rs.records
        .iter()
        .map(|r| r.get(col).cloned().unwrap_or(Value::Null))
        .collect()
}

fn col_strings(rs: &ResultSet, col: &str) -> Vec<String> {
    col_values(rs, col)
        .into_iter()
        .map(|v| match v {
            Value::String(s) => s,
            other => format!("{:?}", other),
        })
        .collect()
}

// ===========================================================================
// CREATE GRAPH tests
// ===========================================================================

#[test]
fn create_graph_registers_metadata() {
    let env = MultiGraphEnv::new();
    env.create_graph("analytics");
    let graphs = env.list_graphs();
    assert!(graphs.iter().any(|g| g.name == "analytics"));
}

#[test]
fn create_multiple_graphs() {
    let env = MultiGraphEnv::new();
    env.create_graph("alpha");
    env.create_graph("beta");
    env.create_graph("gamma");
    let graphs = env.list_graphs();
    let names: Vec<&str> = graphs.iter().map(|g| g.name.as_str()).collect();
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta"));
    assert!(names.contains(&"gamma"));
}

#[test]
fn create_graph_via_query() {
    let env = MultiGraphEnv::new();
    env.create_graph("default");
    env.exec("default", "CREATE GRAPH test_graph");
    let graphs = env.list_graphs();
    assert!(graphs.iter().any(|g| g.name == "test_graph"));
}

#[test]
fn create_graph_if_not_exists_is_idempotent() {
    let env = MultiGraphEnv::new();
    env.create_graph("default");
    env.exec("default", "CREATE GRAPH IF NOT EXISTS myg");
    env.exec("default", "CREATE GRAPH IF NOT EXISTS myg");
    let count = env.list_graphs().iter().filter(|g| g.name == "myg").count();
    assert_eq!(count, 1);
}

// ===========================================================================
// DROP GRAPH tests
// ===========================================================================

#[test]
fn drop_graph_removes_metadata() {
    let env = MultiGraphEnv::new();
    env.create_graph("default");
    env.create_graph("ephemeral");
    assert!(env.list_graphs().iter().any(|g| g.name == "ephemeral"));
    env.exec("default", "DROP GRAPH ephemeral");
    assert!(!env.list_graphs().iter().any(|g| g.name == "ephemeral"));
}

#[test]
fn drop_graph_if_exists_no_error_when_missing() {
    let env = MultiGraphEnv::new();
    env.create_graph("default");
    // Should not panic/error
    env.exec("default", "DROP GRAPH IF EXISTS nonexistent");
}

#[test]
fn drop_graph_removes_all_data() {
    let env = MultiGraphEnv::new();
    env.create_graph("doomed");
    env.exec("doomed", "INSERT (:Person {name: 'Alice'})");
    env.exec("doomed", "INSERT (:Person {name: 'Bob'})");
    assert_eq!(env.exec("doomed", "MATCH (n) RETURN n").records.len(), 2);

    env.create_graph("default");
    env.exec("default", "DROP GRAPH doomed");

    // After drop, recreating and querying should be empty
    env.create_graph("doomed");
    assert_eq!(env.exec("doomed", "MATCH (n) RETURN n").records.len(), 0);
}

// ===========================================================================
// LIST GRAPHS tests
// ===========================================================================

#[test]
fn list_graphs_empty_initially() {
    let env = MultiGraphEnv::new();
    assert!(env.list_graphs().is_empty());
}

#[test]
fn list_graphs_returns_all() {
    let env = MultiGraphEnv::new();
    env.create_graph("one");
    env.create_graph("two");
    env.create_graph("three");
    assert_eq!(env.list_graphs().len(), 3);
}

#[test]
fn list_graphs_via_query() {
    let env = MultiGraphEnv::new();
    env.create_graph("default");
    env.create_graph("project_a");
    env.create_graph("project_b");
    let rs = env.exec("default", "LIST GRAPHS");
    assert!(rs.records.len() >= 3);
}

// ===========================================================================
// Data isolation tests
// ===========================================================================

#[test]
fn data_isolated_between_graphs() {
    let env = MultiGraphEnv::new();
    env.create_graph("graph_a");
    env.create_graph("graph_b");

    env.exec("graph_a", "INSERT (:Person {name: 'Alice'})");
    env.exec("graph_b", "INSERT (:Product {name: 'Widget'})");

    let a_nodes = env.exec("graph_a", "MATCH (n) RETURN n.name");
    let b_nodes = env.exec("graph_b", "MATCH (n) RETURN n.name");

    assert_eq!(a_nodes.records.len(), 1);
    assert_eq!(b_nodes.records.len(), 1);

    let a_names = col_strings(&a_nodes, "n.name");
    let b_names = col_strings(&b_nodes, "n.name");
    assert_eq!(a_names, vec!["Alice"]);
    assert_eq!(b_names, vec!["Widget"]);
}

#[test]
fn labels_isolated_between_graphs() {
    let env = MultiGraphEnv::new();
    env.create_graph("g1");
    env.create_graph("g2");

    env.exec("g1", "INSERT (:Dog {name: 'Rex'})");
    env.exec("g2", "INSERT (:Cat {name: 'Whiskers'})");

    let g1_dogs = env.exec("g1", "MATCH (n:Dog) RETURN n.name");
    let g1_cats = env.exec("g1", "MATCH (n:Cat) RETURN n.name");
    let g2_dogs = env.exec("g2", "MATCH (n:Dog) RETURN n.name");
    let g2_cats = env.exec("g2", "MATCH (n:Cat) RETURN n.name");

    assert_eq!(g1_dogs.records.len(), 1);
    assert_eq!(g1_cats.records.len(), 0);
    assert_eq!(g2_dogs.records.len(), 0);
    assert_eq!(g2_cats.records.len(), 1);
}

#[test]
fn edges_isolated_between_graphs() {
    let env = MultiGraphEnv::new();
    env.create_graph("social");
    env.create_graph("logistics");

    env.exec("social", "INSERT (:Person {name: 'Alice'})");
    env.exec("social", "INSERT (:Person {name: 'Bob'})");
    env.exec(
        "social",
        "MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) INSERT (a)-[:KNOWS]->(b)",
    );

    env.exec("logistics", "INSERT (:Warehouse {name: 'W1'})");
    env.exec("logistics", "INSERT (:Warehouse {name: 'W2'})");
    env.exec(
        "logistics",
        "MATCH (a:Warehouse {name: 'W1'}), (b:Warehouse {name: 'W2'}) INSERT (a)-[:SHIPS_TO]->(b)",
    );

    // Each graph should have exactly one edge of its type
    let social_edges = env.exec("social", "MATCH ()-[r:KNOWS]->() RETURN r");
    let social_ships = env.exec("social", "MATCH ()-[r:SHIPS_TO]->() RETURN r");
    assert_eq!(social_edges.records.len(), 1);
    assert_eq!(social_ships.records.len(), 0);

    let logi_edges = env.exec("logistics", "MATCH ()-[r:KNOWS]->() RETURN r");
    let logi_ships = env.exec("logistics", "MATCH ()-[r:SHIPS_TO]->() RETURN r");
    assert_eq!(logi_edges.records.len(), 0);
    assert_eq!(logi_ships.records.len(), 1);
}

#[test]
fn mutations_on_one_graph_dont_affect_another() {
    let env = MultiGraphEnv::new();
    env.create_graph("stable");
    env.create_graph("volatile");

    env.exec("stable", "INSERT (:Item {val: 1})");
    env.exec("stable", "INSERT (:Item {val: 2})");
    env.exec("volatile", "INSERT (:Item {val: 100})");

    // Delete everything from volatile
    env.exec("volatile", "MATCH (n) DELETE n");

    // Stable should be untouched
    assert_eq!(
        env.exec("stable", "MATCH (n) RETURN n").records.len(),
        2
    );
    assert_eq!(
        env.exec("volatile", "MATCH (n) RETURN n").records.len(),
        0
    );
}

#[test]
fn property_updates_isolated() {
    let env = MultiGraphEnv::new();
    env.create_graph("g1");
    env.create_graph("g2");

    env.exec("g1", "INSERT (:Config {key: 'mode', val: 'fast'})");
    env.exec("g2", "INSERT (:Config {key: 'mode', val: 'safe'})");

    env.exec("g1", "MATCH (n:Config {key: 'mode'}) SET n.val = 'turbo'");

    let g1_val = col_strings(&env.exec("g1", "MATCH (n:Config) RETURN n.val"), "n.val");
    let g2_val = col_strings(&env.exec("g2", "MATCH (n:Config) RETURN n.val"), "n.val");
    assert_eq!(g1_val, vec!["turbo"]);
    assert_eq!(g2_val, vec!["safe"]);
}

#[test]
fn indexes_isolated_between_graphs() {
    let env = MultiGraphEnv::new();
    env.create_graph("idx_a");
    env.create_graph("idx_b");

    env.exec("idx_a", "INSERT (:User {email: 'a@test.com'})");
    env.exec("idx_b", "INSERT (:User {email: 'b@test.com'})");

    // Queries on each graph return only their data
    let a = col_strings(
        &env.exec("idx_a", "MATCH (n:User) RETURN n.email"),
        "n.email",
    );
    let b = col_strings(
        &env.exec("idx_b", "MATCH (n:User) RETURN n.email"),
        "n.email",
    );
    assert_eq!(a, vec!["a@test.com"]);
    assert_eq!(b, vec!["b@test.com"]);
}

// ===========================================================================
// Graph switching / concurrent access tests
// ===========================================================================

#[test]
fn interleaved_operations_across_graphs() {
    let env = MultiGraphEnv::new();
    env.create_graph("a");
    env.create_graph("b");

    for i in 0..10 {
        let graph = if i % 2 == 0 { "a" } else { "b" };
        env.exec(graph, &format!("INSERT (:Item {{seq: {i}}})"));
    }

    let a_count = env.exec("a", "MATCH (n:Item) RETURN n").records.len();
    let b_count = env.exec("b", "MATCH (n:Item) RETURN n").records.len();
    assert_eq!(a_count, 5);
    assert_eq!(b_count, 5);
}

#[test]
fn large_dataset_isolation() {
    let env = MultiGraphEnv::new();
    env.create_graph("big");
    env.create_graph("small");

    for i in 0..100 {
        env.exec("big", &format!("INSERT (:Record {{id: {i}}})"));
    }
    env.exec("small", "INSERT (:Record {id: 999})");

    assert_eq!(
        env.exec("big", "MATCH (n:Record) RETURN n").records.len(),
        100
    );
    assert_eq!(
        env.exec("small", "MATCH (n:Record) RETURN n").records.len(),
        1
    );
}

#[test]
fn graph_names_are_case_sensitive() {
    let env = MultiGraphEnv::new();
    env.create_graph("MyGraph");
    env.create_graph("mygraph");

    env.exec("MyGraph", "INSERT (:X {val: 'upper'})");
    env.exec("mygraph", "INSERT (:X {val: 'lower'})");

    let upper = col_strings(&env.exec("MyGraph", "MATCH (n:X) RETURN n.val"), "n.val");
    let lower = col_strings(&env.exec("mygraph", "MATCH (n:X) RETURN n.val"), "n.val");
    assert_eq!(upper, vec!["upper"]);
    assert_eq!(lower, vec!["lower"]);
}

// ===========================================================================
// Edge cases
// ===========================================================================

#[test]
fn default_graph_works() {
    let env = MultiGraphEnv::new();
    env.create_graph("default");
    env.exec("default", "INSERT (:Test {status: 'ok'})");
    let rs = env.exec("default", "MATCH (n:Test) RETURN n.status");
    assert_eq!(col_strings(&rs, "n.status"), vec!["ok"]);
}

#[test]
fn drop_and_recreate_graph() {
    let env = MultiGraphEnv::new();
    env.create_graph("temp");
    env.exec("temp", "INSERT (:Data {val: 'old'})");
    assert_eq!(env.exec("temp", "MATCH (n) RETURN n").records.len(), 1);

    env.storage.delete_graph(&GraphId::from_name("temp")).unwrap();
    env.create_graph("temp");

    assert_eq!(env.exec("temp", "MATCH (n) RETURN n").records.len(), 0);
    env.exec("temp", "INSERT (:Data {val: 'new'})");
    let vals = col_strings(&env.exec("temp", "MATCH (n:Data) RETURN n.val"), "n.val");
    assert_eq!(vals, vec!["new"]);
}

#[test]
fn many_graphs_coexist() {
    let env = MultiGraphEnv::new();
    let count = 20;
    for i in 0..count {
        let name = format!("graph_{i}");
        env.create_graph(&name);
        env.exec(&name, &format!("INSERT (:Marker {{graph: '{name}'}})",));
    }
    assert_eq!(env.list_graphs().len(), count);

    for i in 0..count {
        let name = format!("graph_{i}");
        let rs = env.exec(&name, "MATCH (n:Marker) RETURN n.graph");
        assert_eq!(rs.records.len(), 1);
        let val = col_strings(&rs, "n.graph");
        assert_eq!(val, vec![name]);
    }
}

#[test]
fn traversal_isolated_between_graphs() {
    let env = MultiGraphEnv::new();
    env.create_graph("chain");
    env.create_graph("other");

    // Build a chain: A -> B -> C in "chain" graph
    env.exec("chain", "INSERT (:N {name: 'A'})");
    env.exec("chain", "INSERT (:N {name: 'B'})");
    env.exec("chain", "INSERT (:N {name: 'C'})");
    env.exec(
        "chain",
        "MATCH (a:N {name: 'A'}), (b:N {name: 'B'}) INSERT (a)-[:NEXT]->(b)",
    );
    env.exec(
        "chain",
        "MATCH (b:N {name: 'B'}), (c:N {name: 'C'}) INSERT (b)-[:NEXT]->(c)",
    );

    // "other" graph has no data
    env.exec("other", "INSERT (:N {name: 'X'})");

    // Traversal in chain should find the path
    let rs = env.exec(
        "chain",
        "MATCH (a:N {name: 'A'})-[:NEXT]->(b)-[:NEXT]->(c) RETURN c.name",
    );
    assert_eq!(rs.records.len(), 1);
    assert_eq!(col_strings(&rs, "c.name"), vec!["C"]);

    // Same traversal in "other" should find nothing
    let rs2 = env.exec(
        "other",
        "MATCH (a:N)-[:NEXT]->(b) RETURN b.name",
    );
    assert_eq!(rs2.records.len(), 0);
}

#[test]
fn aggregation_isolated_between_graphs() {
    let env = MultiGraphEnv::new();
    env.create_graph("sales");
    env.create_graph("inventory");

    env.exec("sales", "INSERT (:Sale {amount: 100})");
    env.exec("sales", "INSERT (:Sale {amount: 200})");
    env.exec("sales", "INSERT (:Sale {amount: 300})");
    env.exec("inventory", "INSERT (:Sale {amount: 9999})");

    let rs = env.exec("sales", "MATCH (n:Sale) RETURN sum(n.amount) AS total");
    let total = &rs.records[0].get("total");
    match total {
        Some(Value::Integer(600)) => {}
        Some(Value::Float(f)) if (*f - 600.0).abs() < 0.01 => {}
        other => panic!("expected 600, got {:?}", other),
    }
}
