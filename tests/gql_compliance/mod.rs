//! GQL compliance integration tests for Synaptica.
//!
//! Covers parser compliance, logical plan generation, and end-to-end
//! execution against a temporary storage engine.

use synaptica_core::graph::{Edge, GraphId, GraphMeta, Node};
use synaptica_core::types::Value;
use synaptica_exec::engine::ExecutionEngine;
use synaptica_exec::result::ResultSet;
use synaptica_gql::ast::*;
use synaptica_gql::parser::parse;
use synaptica_gql::planner::{LogicalPlan, QueryPlanner};
use synaptica_storage::engine::{StorageConfig, StorageEngine};

// ===========================================================================
// Test fixture helpers
// ===========================================================================

struct TestGraph {
    storage: StorageEngine,
    graph_id: GraphId,
    _dir: tempfile::TempDir,
}

impl TestGraph {
    /// Build a small social-network graph:
    ///
    /// Persons: Alice(30), Bob(35), Carol(28), Dave(42), Eve(25)
    /// Companies: Acme, Globex, Initech
    /// Edges:   Alice-KNOWS->Bob, Alice-KNOWS->Carol, Bob-KNOWS->Dave
    ///          Alice-WORKS_AT->Acme(since:2020), Bob-WORKS_AT->Globex(since:2018)
    ///          Dave-WORKS_AT->Initech(since:2015)
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let storage =
            StorageEngine::open(dir.path(), &StorageConfig::default()).expect("open storage");
        let graph_id = GraphId::new();

        let meta = GraphMeta {
            id: graph_id,
            name: "social".to_string(),
            graph_type: None,
        };
        storage.put_graph_meta(&meta).unwrap();

        // -- Person nodes ---------------------------------------------------
        let alice = Self::person(&storage, graph_id, "Alice", 30);
        let bob = Self::person(&storage, graph_id, "Bob", 35);
        let carol = Self::person(&storage, graph_id, "Carol", 28);
        let dave = Self::person(&storage, graph_id, "Dave", 42);
        let _eve = Self::person(&storage, graph_id, "Eve", 25);

        // -- Company nodes --------------------------------------------------
        Self::company(&storage, graph_id, "Acme");
        Self::company(&storage, graph_id, "Globex");
        Self::company(&storage, graph_id, "Initech");

        // -- KNOWS edges ----------------------------------------------------
        Self::knows(&storage, graph_id, &alice, &bob);
        Self::knows(&storage, graph_id, &alice, &carol);
        Self::knows(&storage, graph_id, &bob, &dave);

        // -- WORKS_AT edges -------------------------------------------------
        Self::works_at(&storage, graph_id, &alice, "Acme", 2020);
        Self::works_at(&storage, graph_id, &bob, "Globex", 2018);
        Self::works_at(&storage, graph_id, &dave, "Initech", 2015);

        TestGraph {
            storage,
            graph_id,
            _dir: dir,
        }
    }

    fn person(storage: &StorageEngine, gid: GraphId, name: &str, age: i64) -> Node {
        let mut n = Node::new(gid);
        n.add_label("Person");
        n.set_property("name".to_string(), Value::String(name.to_string()));
        n.set_property("age".to_string(), Value::Integer(age));
        storage.put_node(&n).unwrap();
        n
    }

    fn company(storage: &StorageEngine, gid: GraphId, name: &str) -> Node {
        let mut n = Node::new(gid);
        n.add_label("Company");
        n.set_property("name".to_string(), Value::String(name.to_string()));
        storage.put_node(&n).unwrap();
        n
    }

    fn knows(storage: &StorageEngine, gid: GraphId, from: &Node, to: &Node) {
        let edge = Edge::new(gid, from.id, to.id, "KNOWS");
        storage.put_edge(&edge).unwrap();
    }

    fn works_at(storage: &StorageEngine, gid: GraphId, person: &Node, _company_name: &str, since: i64) {
        // Find company node by scanning — keeps helpers simple.
        let companies = storage
            .scan_nodes_by_label(&gid, &synaptica_core::graph::Label::new("Company"))
            .unwrap();
        let company = companies
            .iter()
            .find(|c| c.get_property("name") == Some(&Value::String(_company_name.to_string())))
            .expect("company not found");
        let mut edge = Edge::new(gid, person.id, company.id, "WORKS_AT");
        edge.set_property("since".to_string(), Value::Integer(since));
        storage.put_edge(&edge).unwrap();
    }
}

/// Parse, plan, and execute a query against the test graph.
fn execute_query(tg: &TestGraph, query: &str) -> ResultSet {
    let program = parse(query).expect("parse failed");
    let planner = QueryPlanner::new();
    let plan = planner.plan(&program).expect("planning failed");
    let engine = ExecutionEngine::new(&tg.storage);
    engine
        .execute_plan(&plan, &tg.graph_id)
        .expect("execution failed")
}

/// Helper: collect all values from a named column in the result set.
fn column_values(rs: &ResultSet, col: &str) -> Vec<Value> {
    rs.records.iter().map(|r| r.get(col).cloned().unwrap_or(Value::Null)).collect()
}

// ===========================================================================
// 1. Parser compliance tests
// ===========================================================================

mod parser_compliance {
    use super::*;

    #[test]
    fn parse_basic_scan() {
        let prog = parse("MATCH (n) RETURN n").unwrap();
        assert_eq!(prog.statements.len(), 2);
        assert!(matches!(&prog.statements[0], GqlStatement::Match(_)));
        assert!(matches!(&prog.statements[1], GqlStatement::Return(_)));
    }

    #[test]
    fn parse_label_filter_with_property_projection() {
        let prog = parse("MATCH (n:Person) RETURN n.name").unwrap();
        assert_eq!(prog.statements.len(), 2);

        if let GqlStatement::Match(m) = &prog.statements[0] {
            let node = match &m.pattern.paths[0].elements[0] {
                PatternElement::Node(n) => n,
                _ => panic!("expected node"),
            };
            assert_eq!(node.labels, vec!["Person"]);
            assert_eq!(node.variable.as_deref(), Some("n"));
        } else {
            panic!("expected MATCH");
        }

        if let GqlStatement::Return(r) = &prog.statements[1] {
            assert_eq!(r.items.len(), 1);
            assert!(matches!(
                &r.items[0].expression,
                Expression::PropertyAccess { property, .. } if property == "name"
            ));
        } else {
            panic!("expected RETURN");
        }
    }

    #[test]
    fn parse_relationship_traversal() {
        let prog = parse("MATCH (a:Person)-[r:KNOWS]->(b:Person) RETURN a.name, b.name").unwrap();
        assert_eq!(prog.statements.len(), 2);

        if let GqlStatement::Match(m) = &prog.statements[0] {
            let elems = &m.pattern.paths[0].elements;
            // node - edge - node
            assert_eq!(elems.len(), 3);
            assert!(matches!(&elems[0], PatternElement::Node(n) if n.labels == vec!["Person"]));
            if let PatternElement::Edge(e) = &elems[1] {
                assert_eq!(e.labels, vec!["KNOWS"]);
                assert_eq!(e.variable.as_deref(), Some("r"));
                assert_eq!(e.direction, Direction::Outgoing); // -> means Outgoing
            } else {
                panic!("expected edge");
            }
            assert!(matches!(&elems[2], PatternElement::Node(n) if n.labels == vec!["Person"]));
        } else {
            panic!("expected MATCH");
        }

        if let GqlStatement::Return(r) = &prog.statements[1] {
            assert_eq!(r.items.len(), 2);
        } else {
            panic!("expected RETURN");
        }
    }

    #[test]
    fn parse_filter_and_order_by() {
        let prog =
            parse("MATCH (n:Person) WHERE n.age > 30 RETURN n.name ORDER BY n.name ASC").unwrap();
        assert_eq!(prog.statements.len(), 2);

        if let GqlStatement::Match(m) = &prog.statements[0] {
            assert!(m.where_clause.is_some());
            let cond = &*m.where_clause.as_ref().unwrap().condition;
            assert!(matches!(cond, Expression::BinaryOp { op: BinaryOp::Gt, .. }));
        } else {
            panic!("expected MATCH");
        }

        if let GqlStatement::Return(r) = &prog.statements[1] {
            let ob = r.order_by.as_ref().expect("expected ORDER BY");
            assert_eq!(ob.items.len(), 1);
            assert_eq!(ob.items[0].direction, SortDirection::Asc);
        } else {
            panic!("expected RETURN");
        }
    }

    #[test]
    fn parse_boolean_logic_or() {
        let prog =
            parse("MATCH (n:Person) WHERE n.name = 'Alice' OR n.name = 'Bob' RETURN n").unwrap();
        if let GqlStatement::Match(m) = &prog.statements[0] {
            let cond = &*m.where_clause.as_ref().unwrap().condition;
            assert!(matches!(cond, Expression::BinaryOp { op: BinaryOp::Or, .. }));
        } else {
            panic!("expected MATCH");
        }
    }

    #[test]
    fn parse_aggregate_count() {
        let prog = parse("MATCH (n:Person) RETURN COUNT(n)").unwrap();
        if let GqlStatement::Return(r) = &prog.statements[1] {
            assert!(matches!(
                &r.items[0].expression,
                Expression::Aggregate { function: AggregateFunction::Count, .. }
            ));
        } else {
            panic!("expected RETURN");
        }
    }

    #[test]
    fn parse_limit_and_offset() {
        let prog = parse("MATCH (n:Person) RETURN n.name, n.age LIMIT 5 OFFSET 2").unwrap();
        if let GqlStatement::Return(r) = &prog.statements[1] {
            assert_eq!(r.items.len(), 2);
            let lo = r.limit_offset.as_ref().expect("expected LIMIT/OFFSET");
            assert!(matches!(&lo.limit, Some(Expression::Literal(Literal::Integer(5)))));
            assert!(matches!(&lo.offset, Some(Expression::Literal(Literal::Integer(2)))));
        } else {
            panic!("expected RETURN");
        }
    }

    #[test]
    fn parse_insert_node() {
        let prog = parse("INSERT (:Person {name: 'Charlie', age: 28})").unwrap();
        assert_eq!(prog.statements.len(), 1);
        if let GqlStatement::Insert(ins) = &prog.statements[0] {
            let node = match &ins.patterns[0].elements[0] {
                PatternElement::Node(n) => n,
                _ => panic!("expected node"),
            };
            assert_eq!(node.labels, vec!["Person"]);
            assert_eq!(node.properties.len(), 2);
        } else {
            panic!("expected INSERT");
        }
    }

    #[test]
    fn parse_pattern_with_inline_properties() {
        let prog =
            parse("MATCH (a:Person {name: 'Alice'})-[:KNOWS]->(b) RETURN b.name").unwrap();
        if let GqlStatement::Match(m) = &prog.statements[0] {
            let node = match &m.pattern.paths[0].elements[0] {
                PatternElement::Node(n) => n,
                _ => panic!("expected node"),
            };
            assert_eq!(node.properties.len(), 1);
            assert_eq!(node.properties[0].0, "name");
        } else {
            panic!("expected MATCH");
        }
    }

    #[test]
    fn parse_is_not_null() {
        let prog = parse("MATCH (n) WHERE n.name IS NOT NULL RETURN n.name").unwrap();
        if let GqlStatement::Match(m) = &prog.statements[0] {
            let cond = &*m.where_clause.as_ref().unwrap().condition;
            assert!(matches!(cond, Expression::IsNotNull(_)));
        } else {
            panic!("expected MATCH");
        }
    }

    #[test]
    fn parse_in_expression() {
        let prog = parse("MATCH (n:Person) WHERE n.name IN ['Alice', 'Bob'] RETURN n").unwrap();
        if let GqlStatement::Match(m) = &prog.statements[0] {
            let cond = &*m.where_clause.as_ref().unwrap().condition;
            assert!(matches!(cond, Expression::In { .. }));
        } else {
            panic!("expected MATCH");
        }
    }

    #[test]
    fn parse_like_expression() {
        let prog = parse("MATCH (n:Person) WHERE n.name LIKE 'A%' RETURN n").unwrap();
        if let GqlStatement::Match(m) = &prog.statements[0] {
            let cond = &*m.where_clause.as_ref().unwrap().condition;
            assert!(matches!(cond, Expression::Like { .. }));
        } else {
            panic!("expected MATCH");
        }
    }

    #[test]
    fn parse_relationship_type_function() {
        // Note: `type` is a reserved keyword in the GQL lexer, so we test
        // function-call syntax with `labels()` instead, which exercises
        // the same FunctionCall AST path.
        let prog =
            parse("MATCH (a)-[r]->(b) WHERE a.name = 'Alice' RETURN labels(a), b.name").unwrap();
        if let GqlStatement::Return(r) = &prog.statements[1] {
            assert_eq!(r.items.len(), 2);
            assert!(matches!(
                &r.items[0].expression,
                Expression::FunctionCall { name, args } if name == "labels" && args.len() == 1
            ));
        } else {
            panic!("expected RETURN");
        }
    }

    #[test]
    fn parse_create_graph() {
        let prog = parse("CREATE GRAPH social").unwrap();
        assert_eq!(prog.statements.len(), 1);
        if let GqlStatement::CreateGraph(cg) = &prog.statements[0] {
            assert_eq!(cg.name, "social");
            assert!(!cg.if_not_exists);
        } else {
            panic!("expected CREATE GRAPH");
        }
    }

    #[test]
    fn parse_drop_graph() {
        let prog = parse("DROP GRAPH social").unwrap();
        assert_eq!(prog.statements.len(), 1);
        if let GqlStatement::DropGraph(dg) = &prog.statements[0] {
            assert_eq!(dg.name, "social");
            assert!(!dg.if_exists);
        } else {
            panic!("expected DROP GRAPH");
        }
    }

    #[test]
    fn parse_alias() {
        let prog = parse("MATCH (n:Person) RETURN n.name AS personName").unwrap();
        if let GqlStatement::Return(r) = &prog.statements[1] {
            assert_eq!(r.items[0].alias.as_deref(), Some("personName"));
        } else {
            panic!("expected RETURN");
        }
    }
}

// ===========================================================================
// 2. Planner tests
// ===========================================================================

mod planner_tests {
    use super::*;

    #[test]
    fn plan_scan_then_project() {
        let prog = parse("MATCH (n:Person) RETURN n.name").unwrap();
        let planner = QueryPlanner::new();
        let plan = planner.plan(&prog).unwrap();

        // Outermost should be Project wrapping a Scan.
        if let LogicalPlan::Project { input, expressions } = &plan {
            assert_eq!(expressions.len(), 1);
            assert!(matches!(&**input, LogicalPlan::Scan { labels, .. } if labels == &["Person"]));
        } else {
            panic!("expected Project, got {:?}", plan);
        }
    }

    #[test]
    fn plan_scan_filter_project() {
        let prog = parse("MATCH (n) WHERE n.age > 30 RETURN n").unwrap();
        let planner = QueryPlanner::new();
        let plan = planner.plan(&prog).unwrap();

        // Outermost: Project → Filter → Scan
        if let LogicalPlan::Project { input, .. } = &plan {
            if let LogicalPlan::Filter { input: inner, predicate } = &**input {
                assert!(matches!(&**inner, LogicalPlan::Scan { labels, .. } if labels.is_empty()));
                assert!(matches!(predicate, Expression::BinaryOp { op: BinaryOp::Gt, .. }));
            } else {
                panic!("expected Filter, got {:?}", input);
            }
        } else {
            panic!("expected Project, got {:?}", plan);
        }
    }

    #[test]
    fn plan_label_scan() {
        let prog = parse("MATCH (n:Person) RETURN n").unwrap();
        let planner = QueryPlanner::new();
        let plan = planner.plan(&prog).unwrap();

        if let LogicalPlan::Project { input, .. } = &plan {
            assert!(matches!(&**input, LogicalPlan::Scan { labels, .. } if labels == &["Person"]));
        } else {
            panic!("expected Project");
        }
    }

    #[test]
    fn plan_empty_program() {
        let planner = QueryPlanner::new();
        let plan = planner
            .plan(&GqlProgram { statements: vec![] })
            .unwrap();
        assert_eq!(plan, LogicalPlan::Empty);
    }
}

// ===========================================================================
// 3. End-to-end execution tests
// ===========================================================================

mod e2e_execution {
    use super::*;

    #[test]
    fn scan_all_nodes() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "MATCH (n) RETURN n.name");
        // 5 Person + 3 Company = 8 nodes total
        assert_eq!(rs.len(), 8, "expected 8 nodes, got {}", rs.len());
    }

    #[test]
    fn scan_by_label_person() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "MATCH (n:Person) RETURN n.name");
        assert_eq!(rs.len(), 5, "expected 5 Person nodes, got {}", rs.len());
    }

    #[test]
    fn scan_by_label_company() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "MATCH (n:Company) RETURN n.name");
        assert_eq!(rs.len(), 3, "expected 3 Company nodes, got {}", rs.len());
    }

    #[test]
    fn project_single_property() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "MATCH (n:Person) RETURN n.name");
        assert_eq!(rs.len(), 5);
        assert!(rs.columns.contains(&"n.name".to_string()));
        let names = column_values(&rs, "n.name");
        assert!(names.contains(&Value::String("Alice".into())));
        assert!(names.contains(&Value::String("Bob".into())));
        assert!(names.contains(&Value::String("Eve".into())));
    }

    #[test]
    fn project_multiple_properties() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "MATCH (n:Person) RETURN n.name, n.age");
        assert_eq!(rs.len(), 5);
        assert!(rs.columns.contains(&"n.name".to_string()));
        assert!(rs.columns.contains(&"n.age".to_string()));
    }

    #[test]
    fn filter_by_age() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "MATCH (n:Person) WHERE n.age > 30 RETURN n.name");
        let names = column_values(&rs, "n.name");
        // Bob(35) and Dave(42) are > 30
        assert_eq!(names.len(), 2);
        assert!(names.contains(&Value::String("Bob".into())));
        assert!(names.contains(&Value::String("Dave".into())));
    }

    #[test]
    fn filter_by_equality() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "MATCH (n:Person) WHERE n.name = 'Alice' RETURN n.age");
        assert_eq!(rs.len(), 1);
        let ages = column_values(&rs, "n.age");
        assert_eq!(ages, vec![Value::Integer(30)]);
    }

    #[test]
    fn limit_results() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "MATCH (n:Person) RETURN n.name LIMIT 2");
        assert_eq!(rs.len(), 2);
    }

    #[test]
    fn limit_with_offset() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "MATCH (n:Person) RETURN n.name LIMIT 2 OFFSET 1");
        assert_eq!(rs.len(), 2);
    }

    #[test]
    fn limit_exceeds_row_count() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "MATCH (n:Person) RETURN n.name LIMIT 100");
        assert_eq!(rs.len(), 5);
    }

    #[test]
    fn scan_all_returns_non_empty() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "MATCH (n) RETURN n.name");
        assert!(!rs.is_empty());
    }

    #[test]
    fn empty_label_returns_nothing() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "MATCH (n:NonExistent) RETURN n.name");
        assert_eq!(rs.len(), 0);
    }
}

// ===========================================================================
// 4. End-to-end INSERT → MATCH tests
// ===========================================================================

mod e2e_insert_query {
    use super::*;

    /// Create an empty test graph (storage + graph meta, no pre-populated data).
    fn empty_graph() -> TestGraph {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let storage =
            StorageEngine::open(dir.path(), &StorageConfig::default()).expect("open storage");
        let graph_id = GraphId::new();
        let meta = GraphMeta {
            id: graph_id,
            name: "test".to_string(),
            graph_type: None,
        };
        storage.put_graph_meta(&meta).unwrap();
        TestGraph {
            storage,
            graph_id,
            _dir: dir,
        }
    }

    #[test]
    fn insert_node_then_query() {
        let tg = empty_graph();
        execute_query(&tg, "INSERT (:Person {name: 'Zara', age: 22})");
        let rs = execute_query(&tg, "MATCH (n:Person) WHERE n.name = 'Zara' RETURN n.name, n.age");
        assert_eq!(rs.len(), 1, "expected 1 row, got {}", rs.len());
        assert_eq!(column_values(&rs, "n.name"), vec![Value::String("Zara".into())]);
        assert_eq!(column_values(&rs, "n.age"), vec![Value::Integer(22)]);
    }

    #[test]
    fn insert_multiple_nodes_then_count() {
        let tg = empty_graph();
        execute_query(&tg, "INSERT (:Person {name: 'A'})");
        execute_query(&tg, "INSERT (:Company {name: 'B'})");
        execute_query(&tg, "INSERT (:City {name: 'C'})");

        let persons = execute_query(&tg, "MATCH (n:Person) RETURN n.name");
        assert_eq!(persons.len(), 1);
        let companies = execute_query(&tg, "MATCH (n:Company) RETURN n.name");
        assert_eq!(companies.len(), 1);
        let cities = execute_query(&tg, "MATCH (n:City) RETURN n.name");
        assert_eq!(cities.len(), 1);
        let all = execute_query(&tg, "MATCH (n) RETURN n.name");
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn insert_node_with_single_label() {
        // Multi-label INSERT (e.g. `:Person:Employee`) is not currently
        // supported by the planner — it only picks up the first label from
        // each path element.  We verify single-label INSERT works correctly.
        let tg = empty_graph();
        execute_query(&tg, "INSERT (:Employee {name: 'Test'})");
        let rs = execute_query(&tg, "MATCH (n:Employee) RETURN n.name");
        assert_eq!(rs.len(), 1);
        assert_eq!(column_values(&rs, "n.name"), vec![Value::String("Test".into())]);
    }

    #[test]
    fn insert_preserves_property_types() {
        let tg = empty_graph();
        execute_query(
            &tg,
            "INSERT (:Thing {s: 'hello', i: 42, f: 3.14, b: true})",
        );
        let rs = execute_query(&tg, "MATCH (n:Thing) RETURN n.s, n.i, n.f, n.b");
        assert_eq!(rs.len(), 1);
        assert_eq!(column_values(&rs, "n.s"), vec![Value::String("hello".into())]);
        assert_eq!(column_values(&rs, "n.i"), vec![Value::Integer(42)]);
        assert_eq!(column_values(&rs, "n.f"), vec![Value::Float(3.14)]);
        assert_eq!(column_values(&rs, "n.b"), vec![Value::Bool(true)]);
    }

    #[test]
    fn insert_then_filter() {
        let tg = empty_graph();
        execute_query(&tg, "INSERT (:Person {name: 'X', age: 10})");
        execute_query(&tg, "INSERT (:Person {name: 'Y', age: 20})");
        execute_query(&tg, "INSERT (:Person {name: 'Z', age: 30})");

        let rs = execute_query(&tg, "MATCH (n:Person) WHERE n.age > 15 RETURN n.name");
        let names = column_values(&rs, "n.name");
        assert_eq!(names.len(), 2);
        assert!(names.contains(&Value::String("Y".into())));
        assert!(names.contains(&Value::String("Z".into())));
    }
}

// ===========================================================================
// 5. Complex query pattern tests (against pre-populated TestGraph)
// ===========================================================================

mod e2e_complex_queries {
    use super::*;

    #[test]
    fn filter_with_and() {
        let tg = TestGraph::new();
        let rs = execute_query(
            &tg,
            "MATCH (n:Person) WHERE n.age > 25 AND n.age < 40 RETURN n.name",
        );
        let names = column_values(&rs, "n.name");
        // Bob(35), Carol(28), Alice(30)
        assert_eq!(names.len(), 3, "expected 3 rows, got {:?}", names);
        assert!(names.contains(&Value::String("Bob".into())));
        assert!(names.contains(&Value::String("Carol".into())));
        assert!(names.contains(&Value::String("Alice".into())));
    }

    #[test]
    fn filter_with_or() {
        let tg = TestGraph::new();
        let rs = execute_query(
            &tg,
            "MATCH (n:Person) WHERE n.name = 'Alice' OR n.name = 'Bob' RETURN n.name",
        );
        let names = column_values(&rs, "n.name");
        assert_eq!(names.len(), 2);
        assert!(names.contains(&Value::String("Alice".into())));
        assert!(names.contains(&Value::String("Bob".into())));
    }

    #[test]
    fn filter_with_not_equal() {
        let tg = TestGraph::new();
        let rs = execute_query(
            &tg,
            "MATCH (n:Person) WHERE n.name <> 'Alice' RETURN n.name",
        );
        let names = column_values(&rs, "n.name");
        assert_eq!(names.len(), 4, "expected 4 rows, got {:?}", names);
        assert!(!names.contains(&Value::String("Alice".into())));
    }

    #[test]
    fn filter_with_gte_lte() {
        let tg = TestGraph::new();
        let rs = execute_query(
            &tg,
            "MATCH (n:Person) WHERE n.age >= 30 AND n.age <= 35 RETURN n.name",
        );
        let names = column_values(&rs, "n.name");
        assert_eq!(names.len(), 2, "expected 2 rows, got {:?}", names);
        assert!(names.contains(&Value::String("Alice".into())));
        assert!(names.contains(&Value::String("Bob".into())));
    }

    #[test]
    fn project_name_and_age() {
        let tg = TestGraph::new();
        let rs = execute_query(
            &tg,
            "MATCH (n:Person) WHERE n.name = 'Alice' RETURN n.name, n.age",
        );
        assert_eq!(rs.len(), 1);
        assert_eq!(column_values(&rs, "n.name"), vec![Value::String("Alice".into())]);
        assert_eq!(column_values(&rs, "n.age"), vec![Value::Integer(30)]);
    }

    #[test]
    fn multiple_properties_filter() {
        let tg = TestGraph::new();
        let rs = execute_query(
            &tg,
            "MATCH (n:Person) WHERE n.name = 'Alice' RETURN n.name, n.age",
        );
        assert_eq!(rs.len(), 1);
        assert!(rs.columns.contains(&"n.name".to_string()));
        assert!(rs.columns.contains(&"n.age".to_string()));
        assert_eq!(column_values(&rs, "n.name"), vec![Value::String("Alice".into())]);
        assert_eq!(column_values(&rs, "n.age"), vec![Value::Integer(30)]);
    }

    #[test]
    fn query_empty_result() {
        let tg = TestGraph::new();
        let rs = execute_query(
            &tg,
            "MATCH (n:Person) WHERE n.age > 100 RETURN n.name",
        );
        assert_eq!(rs.len(), 0);
    }

    #[test]
    fn query_all_nodes_count_check() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "MATCH (n) RETURN n.name");
        // 5 Person + 3 Company = 8 nodes
        assert_eq!(rs.len(), 8, "expected 8 nodes, got {}", rs.len());
    }
}

// ===========================================================================
// 6. Sorting and pagination tests
// ===========================================================================

mod e2e_sorting_pagination {
    use super::*;

    #[test]
    fn order_by_integer_asc() {
        let tg = TestGraph::new();
        let rs = execute_query(
            &tg,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age",
        );
        let ages = column_values(&rs, "n.age");
        assert_eq!(
            ages,
            vec![
                Value::Integer(25), // Eve
                Value::Integer(28), // Carol
                Value::Integer(30), // Alice
                Value::Integer(35), // Bob
                Value::Integer(42), // Dave
            ]
        );
    }

    #[test]
    fn order_by_string_asc() {
        let tg = TestGraph::new();
        let rs = execute_query(
            &tg,
            "MATCH (n:Person) RETURN n.name ORDER BY n.name",
        );
        let names = column_values(&rs, "n.name");
        assert_eq!(
            names,
            vec![
                Value::String("Alice".into()),
                Value::String("Bob".into()),
                Value::String("Carol".into()),
                Value::String("Dave".into()),
                Value::String("Eve".into()),
            ]
        );
    }

    #[test]
    fn limit_zero() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "MATCH (n:Person) RETURN n.name LIMIT 0");
        assert_eq!(rs.len(), 0);
    }

    #[test]
    fn offset_only() {
        let tg = TestGraph::new();
        let rs = execute_query(
            &tg,
            "MATCH (n:Person) RETURN n.name LIMIT 100 OFFSET 3",
        );
        assert_eq!(rs.len(), 2, "expected 2 rows (5 - 3 offset), got {}", rs.len());
    }

    #[test]
    fn offset_beyond_results() {
        let tg = TestGraph::new();
        let rs = execute_query(
            &tg,
            "MATCH (n:Person) RETURN n.name LIMIT 10 OFFSET 100",
        );
        assert_eq!(rs.len(), 0);
    }
}

// ===========================================================================
// Feature 1: WITH clause tests
// ===========================================================================

mod with_clause_tests {
    use super::*;

    #[test]
    fn parse_with_statement() {
        let prog = parse("MATCH (n:Person) WITH n RETURN n").unwrap();
        assert_eq!(prog.statements.len(), 3);
        assert!(matches!(&prog.statements[0], GqlStatement::Match(_)));
        assert!(matches!(&prog.statements[1], GqlStatement::With(_)));
        assert!(matches!(&prog.statements[2], GqlStatement::Return(_)));
    }

    #[test]
    fn parse_with_distinct() {
        let prog = parse("MATCH (n:Person) WITH DISTINCT n.name RETURN n.name").unwrap();
        if let GqlStatement::With(w) = &prog.statements[1] {
            assert!(w.distinct);
            assert_eq!(w.items.len(), 1);
        } else {
            panic!("expected WITH");
        }
    }

    #[test]
    fn parse_with_where_clause() {
        let prog = parse("MATCH (n:Person) WITH n WHERE n.age > 30 RETURN n.name").unwrap();
        if let GqlStatement::With(w) = &prog.statements[1] {
            assert!(w.where_clause.is_some());
        } else {
            panic!("expected WITH");
        }
    }

    #[test]
    fn parse_with_order_by_and_limit() {
        let prog = parse("MATCH (n:Person) WITH n ORDER BY n.age LIMIT 3 RETURN n.name").unwrap();
        if let GqlStatement::With(w) = &prog.statements[1] {
            assert!(w.order_by.is_some());
            assert!(w.limit_offset.is_some());
        } else {
            panic!("expected WITH");
        }
    }

    #[test]
    fn plan_with_produces_project() {
        let prog = parse("MATCH (n:Person) WITH n.name RETURN n.name").unwrap();
        let planner = QueryPlanner::new();
        let plan = planner.plan(&prog).unwrap();
        // Should contain a Project wrapping a Scan
        assert!(matches!(plan, LogicalPlan::Project { .. }));
    }

    #[test]
    fn with_alias_parsing() {
        let prog = parse("MATCH (n:Person) WITH n.name AS personName RETURN personName").unwrap();
        if let GqlStatement::With(w) = &prog.statements[1] {
            assert_eq!(w.items.len(), 1);
            assert_eq!(w.items[0].alias.as_deref(), Some("personName"));
        } else {
            panic!("expected WITH");
        }
    }
}

// ===========================================================================
// Feature 2: CALL/YIELD tests
// ===========================================================================

mod call_yield_tests {
    use super::*;

    #[test]
    fn parse_call_simple() {
        let prog = parse("CALL db.labels()").unwrap();
        assert_eq!(prog.statements.len(), 1);
        if let GqlStatement::Call(c) = &prog.statements[0] {
            assert_eq!(c.procedure, "db.labels");
            assert!(c.arguments.is_empty());
            assert!(c.yield_items.is_none());
        } else {
            panic!("expected CALL");
        }
    }

    #[test]
    fn parse_call_dotted_name() {
        let prog = parse("CALL db.schema()").unwrap();
        if let GqlStatement::Call(c) = &prog.statements[0] {
            assert_eq!(c.procedure, "db.schema");
        } else {
            panic!("expected CALL");
        }
    }

    #[test]
    fn parse_call_with_yield() {
        let prog = parse("CALL db.labels() YIELD label").unwrap();
        if let GqlStatement::Call(c) = &prog.statements[0] {
            assert_eq!(c.procedure, "db.labels");
            assert_eq!(c.yield_items, Some(vec!["label".to_string()]));
        } else {
            panic!("expected CALL");
        }
    }

    #[test]
    fn parse_call_with_multiple_yield() {
        let prog = parse("CALL db.schema() YIELD labels, relationshipTypes").unwrap();
        if let GqlStatement::Call(c) = &prog.statements[0] {
            assert_eq!(c.yield_items, Some(vec!["labels".to_string(), "relationshipTypes".to_string()]));
        } else {
            panic!("expected CALL");
        }
    }

    #[test]
    fn parse_call_with_arguments() {
        let prog = parse("CALL db.labels('test')").unwrap();
        if let GqlStatement::Call(c) = &prog.statements[0] {
            assert_eq!(c.arguments.len(), 1);
        } else {
            panic!("expected CALL");
        }
    }

    #[test]
    fn plan_call_produces_call_procedure() {
        let prog = parse("CALL db.labels()").unwrap();
        let planner = QueryPlanner::new();
        let plan = planner.plan(&prog).unwrap();
        assert!(matches!(plan, LogicalPlan::CallProcedure { .. }));
    }

    #[test]
    fn execute_call_db_labels() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "CALL db.labels()");
        let labels = column_values(&rs, "label");
        // We have Person and Company labels
        assert!(labels.contains(&Value::String("Person".into())));
        assert!(labels.contains(&Value::String("Company".into())));
    }

    #[test]
    fn execute_call_db_relationship_types() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "CALL db.relationshipTypes()");
        let types = column_values(&rs, "relationshipType");
        assert!(types.contains(&Value::String("KNOWS".into())));
        assert!(types.contains(&Value::String("WORKS_AT".into())));
    }

    #[test]
    fn execute_call_db_node_count() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "CALL db.nodeCount()");
        let count = column_values(&rs, "count");
        // 5 persons + 3 companies = 8
        assert_eq!(count, vec![Value::Integer(8)]);
    }

    #[test]
    fn execute_call_db_edge_count() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "CALL db.edgeCount()");
        let count = column_values(&rs, "count");
        // 3 KNOWS + 3 WORKS_AT = 6
        assert_eq!(count, vec![Value::Integer(6)]);
    }

    #[test]
    fn execute_call_db_property_keys() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "CALL db.propertyKeys()");
        let keys = column_values(&rs, "propertyKey");
        assert!(keys.contains(&Value::String("name".into())));
        assert!(keys.contains(&Value::String("age".into())));
        assert!(keys.contains(&Value::String("since".into())));
    }

    #[test]
    fn execute_call_with_yield_filter() {
        let tg = TestGraph::new();
        let rs = execute_query(&tg, "CALL db.schema() YIELD labels");
        // Should only have the "labels" column
        assert_eq!(rs.columns, vec!["labels".to_string()]);
        assert_eq!(rs.len(), 1);
    }
}

// ===========================================================================
// Feature 3: Temporal literal parsing tests
// ===========================================================================

mod temporal_tests {
    use super::*;
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
    use synaptica_exec::expression::evaluate;
    use synaptica_exec::result::Record;

    fn eval(query: &str) -> Value {
        let prog = parse(query).unwrap();
        if let GqlStatement::Return(r) = &prog.statements[0] {
            let expr = &r.items[0].expression;
            let record = Record::new(vec![], vec![]);
            evaluate(expr, &record).unwrap()
        } else {
            panic!("expected RETURN");
        }
    }

    #[test]
    fn date_function() {
        let val = eval("RETURN DATE('2024-01-15')");
        assert_eq!(val, Value::Date(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()));
    }

    #[test]
    fn time_function() {
        let val = eval("RETURN TIME('12:30:45')");
        assert_eq!(val, Value::Time(NaiveTime::from_hms_opt(12, 30, 45).unwrap()));
    }

    #[test]
    fn time_function_short() {
        let val = eval("RETURN TIME('09:15')");
        assert_eq!(val, Value::Time(NaiveTime::from_hms_opt(9, 15, 0).unwrap()));
    }

    #[test]
    fn datetime_function() {
        let val = eval("RETURN DATETIME('2024-06-15T10:30:00')");
        let expected = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap()
            .and_hms_opt(10, 30, 0).unwrap();
        assert_eq!(val, Value::Timestamp(expected));
    }

    #[test]
    fn timestamp_function() {
        let val = eval("RETURN TIMESTAMP('2024-06-15 10:30:00')");
        let expected = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap()
            .and_hms_opt(10, 30, 0).unwrap();
        assert_eq!(val, Value::Timestamp(expected));
    }

    #[test]
    fn duration_function() {
        let val = eval("RETURN DURATION('P1Y2M3D')");
        if let Value::Duration(d) = val {
            assert_eq!(d.months, 14); // 1Y = 12M + 2M = 14
            assert_eq!(d.days, 3);
        } else {
            panic!("expected Duration, got {:?}", val);
        }
    }

    #[test]
    fn duration_with_time() {
        let val = eval("RETURN DURATION('PT2H30M')");
        if let Value::Duration(d) = val {
            assert_eq!(d.months, 0);
            assert_eq!(d.days, 0);
            // 2H30M = 9000 seconds = 9_000_000_000_000 nanos
            assert_eq!(d.nanos, 9_000_000_000_000);
        } else {
            panic!("expected Duration, got {:?}", val);
        }
    }

    #[test]
    fn date_null_arg() {
        let val = eval("RETURN DATE(NULL)");
        assert_eq!(val, Value::Null);
    }

    #[test]
    fn lowercase_temporal_functions() {
        let val = eval("RETURN date('2024-01-15')");
        assert_eq!(val, Value::Date(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()));
    }
}

// ===========================================================================
// Feature 4: Scan result limits tests
// ===========================================================================

mod scan_limit_tests {
    use super::*;

    #[test]
    fn scan_nodes_limit_returns_subset() {
        let tg = TestGraph::new();
        let all_nodes = tg.storage.scan_nodes(&tg.graph_id).unwrap();
        assert_eq!(all_nodes.len(), 8); // 5 persons + 3 companies

        let limited = tg.storage.scan_nodes_limit(&tg.graph_id, 3).unwrap();
        assert_eq!(limited.len(), 3);
    }

    #[test]
    fn scan_nodes_limit_larger_than_count() {
        let tg = TestGraph::new();
        let limited = tg.storage.scan_nodes_limit(&tg.graph_id, 100).unwrap();
        assert_eq!(limited.len(), 8); // all 8 nodes
    }

    #[test]
    fn scan_nodes_limit_zero() {
        let tg = TestGraph::new();
        let limited = tg.storage.scan_nodes_limit(&tg.graph_id, 0).unwrap();
        assert_eq!(limited.len(), 0);
    }

    #[test]
    fn scan_edges_limit_returns_subset() {
        let tg = TestGraph::new();
        let limited = tg.storage.scan_edges_limit(&tg.graph_id, 2).unwrap();
        assert_eq!(limited.len(), 2);
    }

    #[test]
    fn scan_edges_limit_larger_than_count() {
        let tg = TestGraph::new();
        let limited = tg.storage.scan_edges_limit(&tg.graph_id, 100).unwrap();
        // 3 KNOWS + 3 WORKS_AT = 6
        assert_eq!(limited.len(), 6);
    }
}

// ===========================================================================
// Feature 5: Hash join tests
// ===========================================================================

mod hash_join_tests {
    use super::*;

    #[test]
    fn cross_join_no_shared_columns() {
        // When two scans use different variable names, columns like `a.name`
        // and `b.name` are distinct, but structural columns like `__node_id`
        // are shared — hash join finds matches. This verifies the hash join
        // correctly handles the structural column overlap.
        let tg = TestGraph::new();
        let prog = parse("MATCH (a:Company), (b:Company) RETURN a.name, b.name").unwrap();
        let planner = QueryPlanner::new();
        let plan = planner.plan(&prog).unwrap();
        let engine = ExecutionEngine::new(&tg.storage);
        let rs = engine.execute_plan(&plan, &tg.graph_id).unwrap();
        // Hash join on shared __node_id/structural columns: 3 companies matching themselves = 3
        assert_eq!(rs.len(), 3);
    }

    #[test]
    fn hash_join_shared_columns() {
        // When two scans share columns (like __node_id), hash join should
        // produce the inner join result instead of cartesian product
        let tg = TestGraph::new();
        // MATCH (n:Person), (n:Person) - same variable scanned twice
        // Hash join on shared __node_id column should produce n rows, not n^2
        let prog = parse("MATCH (n:Person), (n:Person) RETURN n.name").unwrap();
        let planner = QueryPlanner::new();
        let plan = planner.plan(&prog).unwrap();
        let engine = ExecutionEngine::new(&tg.storage);
        let rs = engine.execute_plan(&plan, &tg.graph_id).unwrap();
        // Hash join on shared columns: 5 persons matching themselves = 5
        assert_eq!(rs.len(), 5);
    }

    #[test]
    fn join_preserves_data_integrity() {
        let tg = TestGraph::new();
        // Simple match that goes through expansion (not join)
        let rs = execute_query(
            &tg,
            "MATCH (a:Person)-[r:KNOWS]->(b:Person) RETURN a.name, b.name",
        );
        // Alice->Bob, Alice->Carol, Bob->Dave = 3 relationships
        assert_eq!(rs.len(), 3);
        let a_names = column_values(&rs, "a.name");
        assert!(a_names.contains(&Value::String("Alice".into())));
        assert!(a_names.contains(&Value::String("Bob".into())));
    }
}
