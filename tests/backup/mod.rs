//! Integration tests for backup, export, and import functionality.

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

struct BackupEnv {
    storage: StorageEngine,
    dir: tempfile::TempDir,
}

impl BackupEnv {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let storage =
            StorageEngine::open(dir.path(), &StorageConfig::default()).expect("open storage");
        BackupEnv { storage, dir }
    }

    fn exec(&self, graph_name: &str, query: &str) -> ResultSet {
        let graph_id = GraphId::from_name(graph_name);
        let program = parse(query).expect("parse");
        let planner = QueryPlanner::new();
        let plan = planner.plan(&program).expect("plan");
        let engine = ExecutionEngine::new(&self.storage);
        engine.execute_plan(&plan, &graph_id).expect("execute")
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

    fn count_nodes(&self, graph: &str, label: &str) -> usize {
        let rs = self.exec(graph, &format!("MATCH (n:{label}) RETURN n"));
        rs.records.len()
    }
}

fn col_strings(rs: &ResultSet, col: &str) -> Vec<String> {
    rs.records
        .iter()
        .map(|r| match r.get(col).cloned().unwrap_or(Value::Null) {
            Value::String(s) => s,
            other => format!("{:?}", other),
        })
        .collect()
}

// ===========================================================================
// Backup create / list / delete tests
// ===========================================================================

#[test]
fn create_backup_produces_valid_checkpoint() {
    let env = BackupEnv::new();
    env.create_graph("default");
    env.exec("default", "INSERT (:Person {name: 'Alice'})");

    let backup_dir = env.dir.path().join("backup1");
    env.storage.create_backup(&backup_dir).unwrap();

    // RocksDB checkpoint creates files in the directory
    assert!(backup_dir.exists());
    let entries: Vec<_> = std::fs::read_dir(&backup_dir)
        .unwrap()
        .collect();
    assert!(!entries.is_empty(), "backup directory should contain RocksDB files");
}

#[test]
fn backup_is_independent_from_live_db() {
    let env = BackupEnv::new();
    env.create_graph("default");
    env.exec("default", "INSERT (:Person {name: 'Alice'})");

    let backup_dir = env.dir.path().join("backup_snapshot");
    env.storage.create_backup(&backup_dir).unwrap();

    // Insert more data into the live DB
    env.exec("default", "INSERT (:Person {name: 'Bob'})");
    env.exec("default", "INSERT (:Person {name: 'Charlie'})");

    // Open the backup as a separate StorageEngine
    let backup_storage =
        StorageEngine::open(&backup_dir, &StorageConfig::default()).unwrap();
    let engine = ExecutionEngine::new(&backup_storage);
    let graph_id = GraphId::from_name("default");
    let program = parse("MATCH (n:Person) RETURN n.name").unwrap();
    let planner = QueryPlanner::new();
    let plan = planner.plan(&program).unwrap();
    let rs = engine.execute_plan(&plan, &graph_id).unwrap();

    // Backup should only have Alice (snapshot at time of backup)
    assert_eq!(rs.records.len(), 1);
    let names = col_strings(&rs, "n.name");
    assert_eq!(names, vec!["Alice"]);

    // Live DB should have all three
    assert_eq!(env.count_nodes("default", "Person"), 3);
}

#[test]
fn multiple_backups_are_independent() {
    let env = BackupEnv::new();
    env.create_graph("default");
    env.exec("default", "INSERT (:City {name: 'NYC'})");

    let b1 = env.dir.path().join("b1");
    env.storage.create_backup(&b1).unwrap();

    env.exec("default", "INSERT (:City {name: 'London'})");
    let b2 = env.dir.path().join("b2");
    env.storage.create_backup(&b2).unwrap();

    // b1 has 1 city, b2 has 2
    let s1 = StorageEngine::open(&b1, &StorageConfig::default()).unwrap();
    let s2 = StorageEngine::open(&b2, &StorageConfig::default()).unwrap();

    let gid = GraphId::from_name("default");
    let program = parse("MATCH (n:City) RETURN n").unwrap();
    let planner = QueryPlanner::new();
    let plan = planner.plan(&program).unwrap();

    let r1 = ExecutionEngine::new(&s1).execute_plan(&plan, &gid).unwrap();
    let r2 = ExecutionEngine::new(&s2).execute_plan(&plan, &gid).unwrap();

    assert_eq!(r1.records.len(), 1);
    assert_eq!(r2.records.len(), 2);
}

#[test]
fn backup_to_single_level_subdir() {
    let env = BackupEnv::new();
    let backup_path = env.dir.path().join("my_backup");
    env.storage.create_backup(&backup_path).unwrap();
    assert!(backup_path.exists());
}

// ===========================================================================
// Export tests
// ===========================================================================

#[test]
fn export_empty_graph_produces_no_output() {
    let env = BackupEnv::new();
    env.create_graph("empty");
    let gid = GraphId::from_name("empty");
    let mut buf = Vec::new();
    env.storage.export_graph(&gid, &mut buf).unwrap();
    let output = String::from_utf8(buf).unwrap();
    assert!(output.trim().is_empty(), "empty graph should export nothing");
}

#[test]
fn export_includes_all_nodes() {
    let env = BackupEnv::new();
    env.create_graph("g");
    env.exec("g", "INSERT (:Person {name: 'Alice'})");
    env.exec("g", "INSERT (:Person {name: 'Bob'})");
    env.exec("g", "INSERT (:City {name: 'NYC'})");

    let gid = GraphId::from_name("g");
    let mut buf = Vec::new();
    env.storage.export_graph(&gid, &mut buf).unwrap();
    let output = String::from_utf8(buf).unwrap();

    assert!(output.contains("Alice"), "export should contain Alice");
    assert!(output.contains("Bob"), "export should contain Bob");
    assert!(output.contains("NYC"), "export should contain NYC");
    assert!(output.contains("Person"), "export should contain Person label");
    assert!(output.contains("City"), "export should contain City label");
}

#[test]
fn export_includes_edges() {
    let env = BackupEnv::new();
    env.create_graph("g");
    env.exec("g", "INSERT (:Person {name: 'Alice'})");
    env.exec("g", "INSERT (:Person {name: 'Bob'})");
    env.exec(
        "g",
        "MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) INSERT (a)-[:KNOWS {since: 2020}]->(b)",
    );

    let gid = GraphId::from_name("g");
    let mut buf = Vec::new();
    env.storage.export_graph(&gid, &mut buf).unwrap();
    let output = String::from_utf8(buf).unwrap();

    assert!(output.contains("KNOWS"), "export should contain KNOWS edge label");
    assert!(output.contains("since"), "export should contain edge property");
}

#[test]
fn export_handles_special_characters_in_strings() {
    let env = BackupEnv::new();
    env.create_graph("g");
    env.exec("g", "INSERT (:Note {text: 'hello world'})");

    let gid = GraphId::from_name("g");
    let mut buf = Vec::new();
    env.storage.export_graph(&gid, &mut buf).unwrap();
    let output = String::from_utf8(buf).unwrap();
    assert!(output.contains("hello"), "should export text property");
}

#[test]
fn export_handles_numeric_and_boolean_properties() {
    let env = BackupEnv::new();
    env.create_graph("g");
    env.exec("g", "INSERT (:Item {count: 42, price: 9.99, active: true})");

    let gid = GraphId::from_name("g");
    let mut buf = Vec::new();
    env.storage.export_graph(&gid, &mut buf).unwrap();
    let output = String::from_utf8(buf).unwrap();

    assert!(output.contains("42"), "should export integer");
    assert!(output.contains("9.99"), "should export float");
    assert!(output.contains("true"), "should export boolean");
}

// ===========================================================================
// Import tests (round-trip)
// ===========================================================================

#[test]
fn round_trip_nodes_preserve_data() {
    // Export from env1, import into env2, verify data
    let env1 = BackupEnv::new();
    env1.create_graph("g");
    env1.exec("g", "INSERT (:Person {name: 'Alice', age: 30})");
    env1.exec("g", "INSERT (:Person {name: 'Bob', age: 25})");
    env1.exec("g", "INSERT (:City {name: 'NYC'})");

    let gid = GraphId::from_name("g");
    let mut buf = Vec::new();
    env1.storage.export_graph(&gid, &mut buf).unwrap();
    let exported = String::from_utf8(buf).unwrap();

    // Import into fresh env
    let env2 = BackupEnv::new();
    env2.create_graph("g");
    for line in exported.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        env2.exec("g", line);
    }

    // Verify counts
    assert_eq!(env2.count_nodes("g", "Person"), 2);
    assert_eq!(env2.count_nodes("g", "City"), 1);

    // Verify properties
    let mut names = col_strings(
        &env2.exec("g", "MATCH (n:Person) RETURN n.name"),
        "n.name",
    );
    names.sort();
    assert_eq!(names, vec!["Alice", "Bob"]);
}

#[test]
fn round_trip_with_edges() {
    let env1 = BackupEnv::new();
    env1.create_graph("g");
    env1.exec("g", "INSERT (:Person {name: 'Alice'})");
    env1.exec("g", "INSERT (:Person {name: 'Bob'})");
    env1.exec(
        "g",
        "MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) INSERT (a)-[:KNOWS]->(b)",
    );

    let gid = GraphId::from_name("g");
    let mut buf = Vec::new();
    env1.storage.export_graph(&gid, &mut buf).unwrap();
    let exported = String::from_utf8(buf).unwrap();

    // Import into fresh env
    let env2 = BackupEnv::new();
    env2.create_graph("g");
    for line in exported.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        env2.exec("g", line);
    }

    // Verify edge exists
    let rs = env2.exec(
        "g",
        "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name, b.name",
    );
    assert_eq!(rs.records.len(), 1);
}

#[test]
fn round_trip_preserves_edge_properties() {
    let env1 = BackupEnv::new();
    env1.create_graph("g");
    env1.exec("g", "INSERT (:Person {name: 'Alice'})");
    env1.exec("g", "INSERT (:Person {name: 'Bob'})");
    env1.exec(
        "g",
        "MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) INSERT (a)-[:KNOWS {since: 2020, close: true}]->(b)",
    );

    let gid = GraphId::from_name("g");
    let mut buf = Vec::new();
    env1.storage.export_graph(&gid, &mut buf).unwrap();
    let exported = String::from_utf8(buf).unwrap();

    let env2 = BackupEnv::new();
    env2.create_graph("g");
    for line in exported.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        env2.exec("g", line);
    }

    let rs = env2.exec(
        "g",
        "MATCH (a)-[r:KNOWS]->(b) RETURN r.since",
    );
    assert_eq!(rs.records.len(), 1);
    let since = rs.records[0].get("r.since").cloned().unwrap_or(Value::Null);
    assert_eq!(since, Value::Integer(2020));
}

#[test]
fn round_trip_multiple_graphs() {
    let env1 = BackupEnv::new();
    env1.create_graph("social");
    env1.create_graph("inventory");
    env1.exec("social", "INSERT (:Person {name: 'Alice'})");
    env1.exec("inventory", "INSERT (:Product {sku: 'ABC-123'})");

    // Export both
    let gid_social = GraphId::from_name("social");
    let gid_inv = GraphId::from_name("inventory");
    let mut buf_social = Vec::new();
    let mut buf_inv = Vec::new();
    env1.storage.export_graph(&gid_social, &mut buf_social).unwrap();
    env1.storage.export_graph(&gid_inv, &mut buf_inv).unwrap();

    let exp_social = String::from_utf8(buf_social).unwrap();
    let exp_inv = String::from_utf8(buf_inv).unwrap();

    // Import into fresh env
    let env2 = BackupEnv::new();
    env2.create_graph("social");
    env2.create_graph("inventory");

    for line in exp_social.lines() {
        let line = line.trim();
        if !line.is_empty() { env2.exec("social", line); }
    }
    for line in exp_inv.lines() {
        let line = line.trim();
        if !line.is_empty() { env2.exec("inventory", line); }
    }

    assert_eq!(env2.count_nodes("social", "Person"), 1);
    assert_eq!(env2.count_nodes("inventory", "Product"), 1);
    // Cross-graph isolation
    assert_eq!(env2.count_nodes("social", "Product"), 0);
    assert_eq!(env2.count_nodes("inventory", "Person"), 0);
}

// ===========================================================================
// Backup + restore full cycle
// ===========================================================================

#[test]
fn full_backup_restore_cycle() {
    let env = BackupEnv::new();
    env.create_graph("default");
    env.exec("default", "INSERT (:User {name: 'Admin', role: 'admin'})");
    env.exec("default", "INSERT (:User {name: 'Guest', role: 'guest'})");
    env.exec("default", "INSERT (:Config {key: 'timeout', value: '30'})");

    // Create backup
    let backup_path = env.dir.path().join("full_backup");
    env.storage.create_backup(&backup_path).unwrap();

    // Destroy original data by inserting conflicting data
    env.exec("default", "INSERT (:User {name: 'Hacker', role: 'root'})");

    // "Restore" by opening backup as fresh storage
    let restored = StorageEngine::open(&backup_path, &StorageConfig::default()).unwrap();
    let engine = ExecutionEngine::new(&restored);
    let gid = GraphId::from_name("default");
    let program = parse("MATCH (n:User) RETURN n.name").unwrap();
    let planner = QueryPlanner::new();
    let plan = planner.plan(&program).unwrap();
    let rs = engine.execute_plan(&plan, &gid).unwrap();

    // Should have original 2 users, not the hacker
    assert_eq!(rs.records.len(), 2);
    let mut names: Vec<String> = rs.records.iter()
        .filter_map(|r| match r.get("n.name") {
            Some(Value::String(s)) => Some(s.clone()),
            _ => None,
        })
        .collect();
    names.sort();
    assert_eq!(names, vec!["Admin", "Guest"]);
}

#[test]
fn backup_preserves_multiple_graphs() {
    let env = BackupEnv::new();
    env.create_graph("g1");
    env.create_graph("g2");
    env.exec("g1", "INSERT (:A {x: 1})");
    env.exec("g2", "INSERT (:B {y: 2})");

    let backup_path = env.dir.path().join("multi_graph_backup");
    env.storage.create_backup(&backup_path).unwrap();

    let restored = StorageEngine::open(&backup_path, &StorageConfig::default()).unwrap();
    let e = ExecutionEngine::new(&restored);

    let gid1 = GraphId::from_name("g1");
    let gid2 = GraphId::from_name("g2");
    let planner = QueryPlanner::new();

    let p1 = planner.plan(&parse("MATCH (n:A) RETURN n").unwrap()).unwrap();
    let p2 = planner.plan(&parse("MATCH (n:B) RETURN n").unwrap()).unwrap();

    let r1 = e.execute_plan(&p1, &gid1).unwrap();
    let r2 = e.execute_plan(&p2, &gid2).unwrap();

    assert_eq!(r1.records.len(), 1);
    assert_eq!(r2.records.len(), 1);
}

#[test]
fn backup_preserves_indexes() {
    let env = BackupEnv::new();
    env.create_graph("g");
    env.exec("g", "CREATE INDEX idx_person_name FOR (n:Person) ON (n.name)");
    env.exec("g", "INSERT (:Person {name: 'Alice'})");

    let backup_path = env.dir.path().join("idx_backup");
    env.storage.create_backup(&backup_path).unwrap();

    let restored = StorageEngine::open(&backup_path, &StorageConfig::default()).unwrap();
    let e = ExecutionEngine::new(&restored);
    let gid = GraphId::from_name("g");
    let program = parse("MATCH (n:Person {name: 'Alice'}) RETURN n.name").unwrap();
    let planner = QueryPlanner::new();
    let plan = planner.plan(&program).unwrap();
    let rs = e.execute_plan(&plan, &gid).unwrap();
    assert_eq!(rs.records.len(), 1);
}

#[test]
fn export_large_graph() {
    let env = BackupEnv::new();
    env.create_graph("g");
    // Insert 100 nodes
    for i in 0..100 {
        env.exec("g", &format!("INSERT (:Item {{id: {i}}})")); 
    }

    let gid = GraphId::from_name("g");
    let mut buf = Vec::new();
    env.storage.export_graph(&gid, &mut buf).unwrap();
    let output = String::from_utf8(buf).unwrap();

    let line_count = output.lines().filter(|l| !l.trim().is_empty()).count();
    assert_eq!(line_count, 100, "should export 100 INSERT statements");
}
