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
                assert_eq!(e.direction, Direction::Left); // -> means Left in AST
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
