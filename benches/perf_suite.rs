//! Synaptica Performance Benchmark Suite
//!
//! Run:   cargo bench --bench perf_suite
//! Save:  cargo bench --bench perf_suite -- --save baseline
//! Compare: cargo bench --bench perf_suite -- --compare baseline
//!
//! Reports are saved to `target/perf/` as JSON files.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use synaptica_core::graph::{Edge, GraphId, GraphMeta, Node};
use synaptica_core::types::Value;
use synaptica_exec::engine::ExecutionEngine;
use synaptica_gql::parser;
use synaptica_gql::planner::QueryPlanner;
use synaptica_storage::engine::{StorageConfig, StorageEngine};

// ═══════════════════════════════════════════════════════════════════════════
// Benchmark infrastructure
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct BenchResult {
    name: String,
    category: String,
    iterations: u64,
    min_ns: u64,
    median_ns: u64,
    mean_ns: u64,
    p95_ns: u64,
    p99_ns: u64,
    max_ns: u64,
    ops_per_sec: f64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Report {
    timestamp: String,
    commit: String,
    results: Vec<BenchResult>,
}

fn format_duration(ns: u64) -> String {
    if ns < 1_000 {
        format!("{} ns", ns)
    } else if ns < 1_000_000 {
        format!("{:.1} µs", ns as f64 / 1_000.0)
    } else if ns < 1_000_000_000 {
        format!("{:.2} ms", ns as f64 / 1_000_000.0)
    } else {
        format!("{:.2} s", ns as f64 / 1_000_000_000.0)
    }
}

fn format_ops(ops: f64) -> String {
    if ops >= 1_000_000.0 {
        format!("{:.1}M", ops / 1_000_000.0)
    } else if ops >= 1_000.0 {
        format!("{:.1}K", ops / 1_000.0)
    } else {
        format!("{:.0}", ops)
    }
}

fn percentile(sorted: &[u64], pct: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * pct / 100.0).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Run a benchmark function `f` for at least `min_duration`, collecting timings.
fn bench<F: FnMut()>(name: &str, category: &str, mut f: F, warmup_iters: u64) -> BenchResult {
    // Warmup
    for _ in 0..warmup_iters {
        f();
    }

    // Calibrate: run for 500ms to estimate iteration count
    let cal_start = Instant::now();
    let mut cal_count = 0u64;
    while cal_start.elapsed() < Duration::from_millis(500) {
        f();
        cal_count += 1;
    }
    let cal_elapsed = cal_start.elapsed();
    let ns_per_iter = cal_elapsed.as_nanos() as f64 / cal_count as f64;

    // Target: at least 2 seconds or 100 iterations, whichever is more
    let target_iters = ((2_000_000_000.0 / ns_per_iter) as u64).max(100);

    // Collect individual timings
    let mut timings: Vec<u64> = Vec::with_capacity(target_iters as usize);
    for _ in 0..target_iters {
        let start = Instant::now();
        f();
        timings.push(start.elapsed().as_nanos() as u64);
    }

    timings.sort_unstable();
    let sum: u64 = timings.iter().sum();
    let mean = sum / timings.len() as u64;
    let median = percentile(&timings, 50.0);
    let ops = 1_000_000_000.0 / median as f64;

    BenchResult {
        name: name.to_string(),
        category: category.to_string(),
        iterations: target_iters,
        min_ns: timings[0],
        median_ns: median,
        mean_ns: mean,
        p95_ns: percentile(&timings, 95.0),
        p99_ns: percentile(&timings, 99.0),
        max_ns: *timings.last().unwrap(),
        ops_per_sec: ops,
    }
}

fn get_git_commit() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn report_dir() -> PathBuf {
    let dir = PathBuf::from("target/perf");
    fs::create_dir_all(&dir).ok();
    dir
}

// ═══════════════════════════════════════════════════════════════════════════
// Test environment setup
// ═══════════════════════════════════════════════════════════════════════════

struct TestEnv {
    storage: StorageEngine,
    graph_id: GraphId,
    _tmpdir: tempfile::TempDir,
}

impl TestEnv {
    fn new() -> Self {
        let tmpdir = tempfile::TempDir::new().expect("temp dir");
        let storage = StorageEngine::open(
            tmpdir.path().to_str().unwrap(),
            &StorageConfig::default(),
        )
        .expect("open storage");

        let graph_id = GraphId::from_name("bench");
        storage
            .put_graph_meta(&GraphMeta {
                id: graph_id,
                name: "bench".to_string(),
                graph_type: None,
            })
            .expect("create graph");

        Self {
            storage,
            graph_id,
            _tmpdir: tmpdir,
        }
    }

    fn execute_gql(&self, query: &str) {
        let program = parser::parse(query).expect("parse");
        let plan = QueryPlanner::new().plan(&program).expect("plan");
        let engine = ExecutionEngine::new(&self.storage);
        engine.execute_plan(&plan, &self.graph_id).expect("exec");
    }

    fn seed_persons(&self, count: usize) {
        for i in 0..count {
            let mut node = Node::new(self.graph_id);
            node.add_label("Person");
            node.set_property("name", Value::String(format!("Person_{}", i)));
            node.set_property("age", Value::Integer((20 + i % 60) as i64));
            node.set_property("city", Value::String(
                ["NYC", "LA", "Chicago", "Boston", "Seattle"][i % 5].to_string(),
            ));
            self.storage.put_node(&node).expect("put node");
        }
    }

    fn seed_edges_chain(&self, count: usize) {
        // Create a chain: n0 -> n1 -> n2 -> ... -> n(count-1)
        let mut ids = Vec::with_capacity(count);
        for i in 0..count {
            let mut node = Node::new(self.graph_id);
            node.add_label("Chain");
            node.set_property("idx", Value::Integer(i as i64));
            ids.push(node.id);
            self.storage.put_node(&node).expect("put node");
        }
        for i in 0..count.saturating_sub(1) {
            let edge = Edge::new(self.graph_id, ids[i], ids[i + 1], "NEXT");
            self.storage.put_edge(&edge).expect("put edge");
        }
    }

    fn seed_star(&self, center_count: usize, spokes_per_center: usize) {
        // Create star topologies: each center has `spokes` outgoing KNOWS edges
        for c in 0..center_count {
            let mut center = Node::new(self.graph_id);
            center.add_label("Star");
            center.set_property("role", Value::String("center".to_string()));
            center.set_property("idx", Value::Integer(c as i64));
            let cid = center.id;
            self.storage.put_node(&center).expect("put center");

            for s in 0..spokes_per_center {
                let mut spoke = Node::new(self.graph_id);
                spoke.add_label("Star");
                spoke.set_property("role", Value::String("spoke".to_string()));
                spoke.set_property("idx", Value::Integer((c * 1000 + s) as i64));
                let sid = spoke.id;
                self.storage.put_node(&spoke).expect("put spoke");

                let edge = Edge::new(self.graph_id, cid, sid, "KNOWS");
                self.storage.put_edge(&edge).expect("put edge");
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Benchmark definitions
// ═══════════════════════════════════════════════════════════════════════════

fn run_parse_benchmarks(results: &mut Vec<BenchResult>) {
    let queries = [
        ("simple_match", "MATCH (n:Person) RETURN n.name"),
        ("filter_and_project", "MATCH (n:Person) WHERE n.age > 30 RETURN n.name, n.age ORDER BY n.age LIMIT 10"),
        ("two_hop_traversal", "MATCH (a:Person)-[:KNOWS]->(b:Person)-[:KNOWS]->(c:Person) RETURN a.name, c.name"),
        ("aggregation", "MATCH (p:Person) RETURN p.city, COUNT(*) AS cnt, AVG(p.age) AS avg_age ORDER BY cnt DESC"),
        ("complex_pipeline", "MATCH (p:Person) WHERE p.age > 25 WITH p.city AS city, COUNT(*) AS cnt WHERE cnt > 1 MATCH (q:Person {city: city}) RETURN DISTINCT q.name ORDER BY q.name LIMIT 20"),
        ("insert_statement", "INSERT (:Person {name: 'Alice', age: 30, city: 'NYC', email: 'alice@test.com'})"),
        ("case_expression", "MATCH (p:Person) RETURN p.name, CASE WHEN p.age > 50 THEN 'senior' WHEN p.age > 30 THEN 'mid' ELSE 'junior' END AS tier"),
        ("multi_function", "MATCH (p:Person) RETURN p.name, TOSTRING(p.age), TOFLOAT(p.age), LABELS(p), KEYS(p)"),
    ];

    for (name, query) in &queries {
        let q = *query;
        results.push(bench(
            &format!("parse/{}", name),
            "parse",
            || { parser::parse(q).unwrap(); },
            50,
        ));
    }
}

fn run_plan_benchmarks(results: &mut Vec<BenchResult>) {
    let queries = [
        ("simple_match", "MATCH (n:Person) RETURN n.name"),
        ("filter_order_limit", "MATCH (n:Person) WHERE n.age > 30 RETURN n.name ORDER BY n.age DESC LIMIT 10"),
        ("two_hop", "MATCH (a:Person)-[:KNOWS]->(b)-[:KNOWS]->(c) RETURN a.name, c.name"),
        ("aggregation_pipeline", "MATCH (p:Person) WITH p.city AS city, COUNT(*) AS cnt RETURN city, cnt ORDER BY cnt DESC"),
    ];

    for (name, query) in &queries {
        let program = parser::parse(query).unwrap();
        let p = program.clone();
        results.push(bench(
            &format!("plan/{}", name),
            "plan",
            || { QueryPlanner::new().plan(&p).unwrap(); },
            50,
        ));
    }
}

fn run_insert_benchmarks(results: &mut Vec<BenchResult>) {
    // Single node insert via GQL
    {
        let env = TestEnv::new();
        let mut i = 0u64;
        results.push(bench(
            "insert/single_node_gql",
            "write",
            || {
                env.execute_gql(&format!(
                    "INSERT (:Bench {{id: {}, value: 'test'}})", i
                ));
                i += 1;
            },
            10,
        ));
    }

    // Single node insert via direct API
    {
        let env = TestEnv::new();
        let mut i = 0u64;
        results.push(bench(
            "insert/single_node_direct",
            "write",
            || {
                let mut node = Node::new(env.graph_id);
                node.add_label("Bench");
                node.set_property("id", Value::Integer(i as i64));
                env.storage.put_node(&node).unwrap();
                i += 1;
            },
            10,
        ));
    }

    // Bulk: 100 nodes via direct API
    {
        let env = TestEnv::new();
        let mut batch = 0u64;
        results.push(bench(
            "insert/100_nodes_direct",
            "write",
            || {
                for j in 0..100 {
                    let mut node = Node::new(env.graph_id);
                    node.add_label("Bulk");
                    node.set_property("id", Value::Integer((batch * 100 + j) as i64));
                    env.storage.put_node(&node).unwrap();
                }
                batch += 1;
            },
            5,
        ));
    }

    // Edge insert via direct API
    {
        let env = TestEnv::new();
        let mut n1 = Node::new(env.graph_id);
        n1.add_label("A");
        let mut n2 = Node::new(env.graph_id);
        n2.add_label("B");
        env.storage.put_node(&n1).unwrap();
        env.storage.put_node(&n2).unwrap();
        let src = n1.id;
        let tgt = n2.id;
        results.push(bench(
            "insert/single_edge_direct",
            "write",
            || {
                let edge = Edge::new(env.graph_id, src, tgt, "REL");
                env.storage.put_edge(&edge).unwrap();
            },
            10,
        ));
    }
}

fn run_scan_benchmarks(results: &mut Vec<BenchResult>) {
    for &size in &[100, 1_000, 10_000] {
        let env = TestEnv::new();
        env.seed_persons(size);

        let label = format!("{}_nodes", size);

        // Full label scan
        {
            let program = parser::parse("MATCH (n:Person) RETURN n.name").unwrap();
            let plan = QueryPlanner::new().plan(&program).unwrap();
            let engine = ExecutionEngine::new(&env.storage);
            let gid = env.graph_id;
            results.push(bench(
                &format!("scan/{}_full", label),
                "read",
                || { engine.execute_plan(&plan, &gid).unwrap(); },
                5,
            ));
        }

        // Filtered scan
        {
            let program = parser::parse(
                "MATCH (n:Person) WHERE n.age > 50 RETURN n.name"
            ).unwrap();
            let plan = QueryPlanner::new().plan(&program).unwrap();
            let engine = ExecutionEngine::new(&env.storage);
            let gid = env.graph_id;
            results.push(bench(
                &format!("scan/{}_filtered", label),
                "read",
                || { engine.execute_plan(&plan, &gid).unwrap(); },
                5,
            ));
        }

        // Scan + ORDER BY + LIMIT
        {
            let program = parser::parse(
                "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age DESC LIMIT 10"
            ).unwrap();
            let plan = QueryPlanner::new().plan(&program).unwrap();
            let engine = ExecutionEngine::new(&env.storage);
            let gid = env.graph_id;
            results.push(bench(
                &format!("scan/{}_sort_limit", label),
                "read",
                || { engine.execute_plan(&plan, &gid).unwrap(); },
                5,
            ));
        }
    }
}

fn run_traversal_benchmarks(results: &mut Vec<BenchResult>) {
    // Chain traversal (1-hop)
    for &size in &[100, 1_000] {
        let env = TestEnv::new();
        env.seed_edges_chain(size);

        let program = parser::parse(
            "MATCH (a:Chain)-[:NEXT]->(b:Chain) RETURN a.idx, b.idx"
        ).unwrap();
        let plan = QueryPlanner::new().plan(&program).unwrap();
        let engine = ExecutionEngine::new(&env.storage);
        let gid = env.graph_id;
        results.push(bench(
            &format!("traverse/1hop_chain_{}", size),
            "traverse",
            || { engine.execute_plan(&plan, &gid).unwrap(); },
            5,
        ));
    }

    // 2-hop traversal on chain
    {
        let env = TestEnv::new();
        env.seed_edges_chain(200);

        let program = parser::parse(
            "MATCH (a:Chain)-[:NEXT]->(b:Chain)-[:NEXT]->(c:Chain) RETURN a.idx, c.idx"
        ).unwrap();
        let plan = QueryPlanner::new().plan(&program).unwrap();
        let engine = ExecutionEngine::new(&env.storage);
        let gid = env.graph_id;
        results.push(bench(
            "traverse/2hop_chain_200",
            "traverse",
            || { engine.execute_plan(&plan, &gid).unwrap(); },
            5,
        ));
    }

    // Star topology (fan-out)
    for &spokes in &[10, 50] {
        let env = TestEnv::new();
        env.seed_star(20, spokes);

        let program = parser::parse(
            "MATCH (c:Star {role: 'center'})-[:KNOWS]->(s:Star) RETURN c.idx, s.idx"
        ).unwrap();
        let plan = QueryPlanner::new().plan(&program).unwrap();
        let engine = ExecutionEngine::new(&env.storage);
        let gid = env.graph_id;
        results.push(bench(
            &format!("traverse/star_20x{}", spokes),
            "traverse",
            || { engine.execute_plan(&plan, &gid).unwrap(); },
            5,
        ));
    }
}

fn run_aggregation_benchmarks(results: &mut Vec<BenchResult>) {
    for &size in &[100, 1_000, 10_000] {
        let env = TestEnv::new();
        env.seed_persons(size);

        // COUNT(*)
        {
            let program = parser::parse(
                "MATCH (p:Person) RETURN COUNT(*) AS total"
            ).unwrap();
            let plan = QueryPlanner::new().plan(&program).unwrap();
            let engine = ExecutionEngine::new(&env.storage);
            let gid = env.graph_id;
            results.push(bench(
                &format!("agg/count_{}", size),
                "aggregation",
                || { engine.execute_plan(&plan, &gid).unwrap(); },
                5,
            ));
        }

        // GROUP BY + COUNT + ORDER
        {
            let program = parser::parse(
                "MATCH (p:Person) RETURN p.city, COUNT(*) AS cnt ORDER BY cnt DESC"
            ).unwrap();
            let plan = QueryPlanner::new().plan(&program).unwrap();
            let engine = ExecutionEngine::new(&env.storage);
            let gid = env.graph_id;
            results.push(bench(
                &format!("agg/group_count_{}", size),
                "aggregation",
                || { engine.execute_plan(&plan, &gid).unwrap(); },
                5,
            ));
        }

        // Multi-aggregate: COUNT + SUM + AVG
        {
            let program = parser::parse(
                "MATCH (p:Person) RETURN p.city, COUNT(*) AS cnt, SUM(p.age) AS total_age, AVG(p.age) AS avg_age"
            ).unwrap();
            let plan = QueryPlanner::new().plan(&program).unwrap();
            let engine = ExecutionEngine::new(&env.storage);
            let gid = env.graph_id;
            results.push(bench(
                &format!("agg/multi_agg_{}", size),
                "aggregation",
                || { engine.execute_plan(&plan, &gid).unwrap(); },
                5,
            ));
        }
    }
}

fn run_pipeline_benchmarks(results: &mut Vec<BenchResult>) {
    let env = TestEnv::new();
    env.seed_persons(1_000);

    // WITH pipeline
    {
        let program = parser::parse(
            "MATCH (p:Person) WHERE p.age > 30 WITH p.city AS city, COUNT(*) AS cnt RETURN city, cnt ORDER BY cnt DESC"
        ).unwrap();
        let plan = QueryPlanner::new().plan(&program).unwrap();
        let engine = ExecutionEngine::new(&env.storage);
        let gid = env.graph_id;
        results.push(bench(
            "pipeline/with_agg_1000",
            "pipeline",
            || { engine.execute_plan(&plan, &gid).unwrap(); },
            5,
        ));
    }

    // DISTINCT
    {
        let program = parser::parse(
            "MATCH (p:Person) RETURN DISTINCT p.city ORDER BY p.city"
        ).unwrap();
        let plan = QueryPlanner::new().plan(&program).unwrap();
        let engine = ExecutionEngine::new(&env.storage);
        let gid = env.graph_id;
        results.push(bench(
            "pipeline/distinct_1000",
            "pipeline",
            || { engine.execute_plan(&plan, &gid).unwrap(); },
            5,
        ));
    }

    // Full pipeline: MATCH → WITH → MATCH → RETURN
    {
        // Seed edges for pipeline test
        env.execute_gql("INSERT (:Company {name: 'Acme'})");
        env.execute_gql("INSERT (:Company {name: 'Globex'})");
        // Connect some persons to companies
        env.execute_gql("MATCH (p:Person), (c:Company {name: 'Acme'}) WHERE p.city = 'NYC' INSERT (p)-[:WORKS_AT]->(c)");
        env.execute_gql("MATCH (p:Person), (c:Company {name: 'Globex'}) WHERE p.city = 'LA' INSERT (p)-[:WORKS_AT]->(c)");

        let program = parser::parse(
            "MATCH (p:Person) WHERE p.age > 30 WITH p MATCH (p)-[:WORKS_AT]->(c:Company) RETURN p.name, c.name"
        ).unwrap();
        let plan = QueryPlanner::new().plan(&program).unwrap();
        let engine = ExecutionEngine::new(&env.storage);
        let gid = env.graph_id;
        results.push(bench(
            "pipeline/full_pipeline_1000",
            "pipeline",
            || { engine.execute_plan(&plan, &gid).unwrap(); },
            5,
        ));
    }
}

fn run_expression_benchmarks(results: &mut Vec<BenchResult>) {
    let env = TestEnv::new();
    env.seed_persons(1_000);

    // CASE expression
    {
        let program = parser::parse(
            "MATCH (p:Person) RETURN p.name, CASE WHEN p.age > 50 THEN 'senior' WHEN p.age > 30 THEN 'mid' ELSE 'junior' END AS tier"
        ).unwrap();
        let plan = QueryPlanner::new().plan(&program).unwrap();
        let engine = ExecutionEngine::new(&env.storage);
        let gid = env.graph_id;
        results.push(bench(
            "expr/case_1000",
            "expression",
            || { engine.execute_plan(&plan, &gid).unwrap(); },
            5,
        ));
    }

    // String concatenation
    {
        let program = parser::parse(
            "MATCH (p:Person) RETURN p.name || ' from ' || p.city AS info"
        ).unwrap();
        let plan = QueryPlanner::new().plan(&program).unwrap();
        let engine = ExecutionEngine::new(&env.storage);
        let gid = env.graph_id;
        results.push(bench(
            "expr/concat_1000",
            "expression",
            || { engine.execute_plan(&plan, &gid).unwrap(); },
            5,
        ));
    }

    // Arithmetic
    {
        let program = parser::parse(
            "MATCH (p:Person) RETURN p.name, p.age * 12 AS months, (p.age + 10) * 2 AS future"
        ).unwrap();
        let plan = QueryPlanner::new().plan(&program).unwrap();
        let engine = ExecutionEngine::new(&env.storage);
        let gid = env.graph_id;
        results.push(bench(
            "expr/arithmetic_1000",
            "expression",
            || { engine.execute_plan(&plan, &gid).unwrap(); },
            5,
        ));
    }

    // Type conversion functions
    {
        let program = parser::parse(
            "MATCH (p:Person) RETURN TOSTRING(p.age) AS s, TOFLOAT(p.age) AS f"
        ).unwrap();
        let plan = QueryPlanner::new().plan(&program).unwrap();
        let engine = ExecutionEngine::new(&env.storage);
        let gid = env.graph_id;
        results.push(bench(
            "expr/type_conv_1000",
            "expression",
            || { engine.execute_plan(&plan, &gid).unwrap(); },
            5,
        ));
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Index vs full-scan benchmarks
// ═══════════════════════════════════════════════════════════════════════════

fn run_index_benchmarks(results: &mut Vec<BenchResult>) {
    use synaptica_storage::index::{IndexDefinition, IndexEntityType, IndexManager};

    for &size in &[1_000, 10_000, 50_000] {
        let label = format!("{}",  size);

        // --- Full scan (no index) ---
        {
            let env = TestEnv::new();
            env.seed_persons(size);
            let target_name = format!("Person_{}", size / 2);
            let query = format!(
                "MATCH (p:Person) WHERE p.name = '{}' RETURN p.name, p.age",
                target_name
            );
            let program = parser::parse(&query).unwrap();
            let plan = QueryPlanner::new().plan(&program).unwrap();
            let engine = ExecutionEngine::new(&env.storage);
            let gid = env.graph_id;
            results.push(bench(
                &format!("scan_no_idx/{}", label),
                "index",
                || { engine.execute_plan(&plan, &gid).unwrap(); },
                3,
            ));
        }

        // --- Indexed scan ---
        {
            let env = TestEnv::new();
            env.seed_persons(size);
            // Create index and backfill
            env.execute_gql("CREATE INDEX idx_name FOR (n:Person) ON (n.name)");
            let target_name = format!("Person_{}", size / 2);
            let query = format!(
                "MATCH (p:Person) WHERE p.name = '{}' RETURN p.name, p.age",
                target_name
            );
            let program = parser::parse(&query).unwrap();
            let plan = QueryPlanner::new().plan(&program).unwrap();
            let engine = ExecutionEngine::new(&env.storage);
            let gid = env.graph_id;
            results.push(bench(
                &format!("scan_with_idx/{}", label),
                "index",
                || { engine.execute_plan(&plan, &gid).unwrap(); },
                3,
            ));
        }
    }
}

fn print_index_comparison(results: &[BenchResult]) {
    let idx_results: Vec<&BenchResult> = results.iter()
        .filter(|r| r.category == "index")
        .collect();

    if idx_results.is_empty() {
        return;
    }

    let bar = "═".repeat(100);
    let thin = "─".repeat(100);
    println!();
    println!("  {}", bar);
    println!("  Index vs Full-Scan Comparison");
    println!("  {}", bar);
    println!();
    println!("  {:<30} {:>12} {:>12} {:>12} {:>12}", 
             "Dataset Size", "Full Scan", "Index Scan", "Speedup", "");
    println!("  {}", thin);

    // Group by dataset size
    let sizes: Vec<String> = idx_results.iter()
        .filter(|r| r.name.starts_with("scan_no_idx/"))
        .map(|r| r.name.strip_prefix("scan_no_idx/").unwrap().to_string())
        .collect();

    for size in &sizes {
        let no_idx = idx_results.iter()
            .find(|r| r.name == format!("scan_no_idx/{}", size))
            .unwrap();
        let with_idx = idx_results.iter()
            .find(|r| r.name == format!("scan_with_idx/{}", size))
            .unwrap();
        
        let speedup = no_idx.median_ns as f64 / with_idx.median_ns as f64;
        let indicator = if speedup > 10.0 { "🚀" }
            else if speedup > 5.0 { "⚡" }
            else if speedup > 2.0 { "✓✓" }
            else { "✓" };

        println!("  {:<30} {:>12} {:>12} {:>11.1}x {}",
                 format!("{} nodes", size),
                 format_duration(no_idx.median_ns),
                 format_duration(with_idx.median_ns),
                 speedup,
                 indicator);
    }

    println!("  {}", thin);
    println!("  Legend: 🚀 >10x  ⚡ >5x  ✓✓ >2x  ✓ faster");
    println!("  {}", bar);
    println!();
}

fn print_report(report: &Report) {
    let bar = "═".repeat(95);
    let thin = "─".repeat(95);
    println!();
    println!("  {}", bar);
    println!("  Synaptica Performance Report");
    println!("  Date:   {}", report.timestamp);
    println!("  Commit: {}", report.commit);
    println!("  {}", bar);
    println!();
    println!("  {:<40} {:>10} {:>10} {:>10} {:>10}", 
             "Benchmark", "Median", "p95", "p99", "ops/sec");
    println!("  {}", thin);

    let mut current_cat = String::new();
    for r in &report.results {
        if r.category != current_cat {
            if !current_cat.is_empty() {
                println!("  {}", thin);
            }
            current_cat = r.category.clone();
        }
        println!("  {:<40} {:>10} {:>10} {:>10} {:>10}",
                 r.name,
                 format_duration(r.median_ns),
                 format_duration(r.p95_ns),
                 format_duration(r.p99_ns),
                 format_ops(r.ops_per_sec));
    }
    println!("  {}", bar);
    println!();
}

fn print_comparison(baseline: &Report, current: &Report) {
    let bar = "═".repeat(100);
    let thin = "─".repeat(100);
    println!();
    println!("  {}", bar);
    println!("  Performance Comparison");
    println!("  Baseline: {} ({})", baseline.commit, baseline.timestamp);
    println!("  Current:  {} ({})", current.commit, current.timestamp);
    println!("  {}", bar);
    println!();
    println!("  {:<40} {:>10} {:>10} {:>10} {:>6}", 
             "Benchmark", "Before", "After", "Change", "");
    println!("  {}", thin);

    let baseline_map: BTreeMap<&str, &BenchResult> = baseline.results.iter()
        .map(|r| (r.name.as_str(), r))
        .collect();

    let mut regressions = 0;
    let mut improvements = 0;

    for r in &current.results {
        if let Some(base) = baseline_map.get(r.name.as_str()) {
            let pct_change = ((r.median_ns as f64 - base.median_ns as f64) / base.median_ns as f64) * 100.0;
            let indicator = if pct_change < -5.0 {
                improvements += 1;
                "  ✓✓"
            } else if pct_change < -1.0 {
                improvements += 1;
                "  ✓"
            } else if pct_change > 10.0 {
                regressions += 1;
                "  ✗✗"
            } else if pct_change > 3.0 {
                regressions += 1;
                "  ⚠"
            } else {
                "  ─"
            };

            println!("  {:<40} {:>10} {:>10} {:>+9.1}% {}",
                     r.name,
                     format_duration(base.median_ns),
                     format_duration(r.median_ns),
                     pct_change,
                     indicator);
        } else {
            println!("  {:<40} {:>10} {:>10} {:>10} {:>6}",
                     r.name, "N/A", format_duration(r.median_ns), "new", "");
        }
    }

    println!("  {}", thin);
    println!();
    println!("  Legend: ✓✓ >5% faster  ✓ >1% faster  ─ within noise  ⚠ >3% slower  ✗✗ >10% slower");
    println!("  Summary: {} improvements, {} regressions", improvements, regressions);
    println!("  {}", bar);
    println!();
}

// ═══════════════════════════════════════════════════════════════════════════
// Main
// ═══════════════════════════════════════════════════════════════════════════

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let save_name = args.windows(2)
        .find(|w| w[0] == "--save")
        .map(|w| w[1].clone());
    let compare_name = args.windows(2)
        .find(|w| w[0] == "--compare")
        .map(|w| w[1].clone());

    println!("\n  Running Synaptica performance benchmarks...\n");

    let mut results = Vec::new();

    // Run all benchmark categories
    print!("  [1/7] Parsing benchmarks...");
    std::io::stdout().flush().unwrap();
    run_parse_benchmarks(&mut results);
    println!(" done ({} benchmarks)", results.len());

    let n = results.len();
    print!("  [2/7] Planning benchmarks...");
    std::io::stdout().flush().unwrap();
    run_plan_benchmarks(&mut results);
    println!(" done ({} benchmarks)", results.len() - n);

    let n = results.len();
    print!("  [3/7] Insert benchmarks...");
    std::io::stdout().flush().unwrap();
    run_insert_benchmarks(&mut results);
    println!(" done ({} benchmarks)", results.len() - n);

    let n = results.len();
    print!("  [4/7] Scan benchmarks...");
    std::io::stdout().flush().unwrap();
    run_scan_benchmarks(&mut results);
    println!(" done ({} benchmarks)", results.len() - n);

    let n = results.len();
    print!("  [5/7] Traversal benchmarks...");
    std::io::stdout().flush().unwrap();
    run_traversal_benchmarks(&mut results);
    println!(" done ({} benchmarks)", results.len() - n);

    let n = results.len();
    print!("  [6/7] Aggregation benchmarks...");
    std::io::stdout().flush().unwrap();
    run_aggregation_benchmarks(&mut results);
    println!(" done ({} benchmarks)", results.len() - n);

    let n = results.len();
    print!("  [7/8] Pipeline & expression benchmarks...");
    std::io::stdout().flush().unwrap();
    run_pipeline_benchmarks(&mut results);
    run_expression_benchmarks(&mut results);
    println!(" done ({} benchmarks)", results.len() - n);

    let n = results.len();
    print!("  [8/8] Index benchmarks...");
    std::io::stdout().flush().unwrap();
    run_index_benchmarks(&mut results);
    println!(" done ({} benchmarks)", results.len() - n);

    let report = Report {
        timestamp: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        commit: get_git_commit(),
        results,
    };

    // Print the report
    print_report(&report);
    print_index_comparison(&report.results);

    // Save if requested
    if let Some(name) = &save_name {
        let path = report_dir().join(format!("{}.json", name));
        let json = serde_json::to_string_pretty(&report).unwrap();
        fs::write(&path, &json).unwrap();
        println!("  Saved to: {}\n", path.display());
    }

    // Always save as "latest"
    {
        let path = report_dir().join("latest.json");
        let json = serde_json::to_string_pretty(&report).unwrap();
        fs::write(&path, &json).unwrap();
    }

    // Compare if requested
    if let Some(name) = &compare_name {
        let path = report_dir().join(format!("{}.json", name));
        if path.exists() {
            let json = fs::read_to_string(&path).unwrap();
            let baseline: Report = serde_json::from_str(&json).unwrap();
            print_comparison(&baseline, &report);
        } else {
            eprintln!("  Baseline '{}' not found at: {}", name, path.display());
            eprintln!("  Available baselines:");
            if let Ok(entries) = fs::read_dir(report_dir()) {
                for entry in entries.flatten() {
                    if entry.path().extension().map_or(false, |e| e == "json") {
                        let stem = entry.path().file_stem().unwrap().to_string_lossy().to_string();
                        if stem != "latest" {
                            eprintln!("    - {}", stem);
                        }
                    }
                }
            }
        }
    }
}
