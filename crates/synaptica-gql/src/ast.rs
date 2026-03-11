//! GQL Abstract Syntax Tree types for ISO/IEC 39075:2024.
//!
//! This module defines the complete AST representation for GQL (Graph Query Language),
//! covering statements, graph patterns, expressions, clauses, literals, and schema types.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Top-level program
// ---------------------------------------------------------------------------

/// A complete GQL program consisting of one or more statements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GqlProgram {
    pub statements: Vec<GqlStatement>,
}

// ---------------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------------

/// A single GQL statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GqlStatement {
    /// `MATCH` – linear or composite graph pattern matching.
    Match(MatchStatement),
    /// `RETURN` – project results.
    Return(ReturnStatement),
    /// `INSERT` – insert nodes/edges.
    Insert(InsertStatement),
    /// `SET` – update properties or labels.
    Set(SetStatement),
    /// `DELETE` – delete nodes/edges.
    Delete(DeleteStatement),
    /// `REMOVE` – remove properties or labels.
    Remove(RemoveStatement),
    /// `CREATE GRAPH` – create a named graph.
    CreateGraph(CreateGraphStatement),
    /// `DROP GRAPH` – drop a named graph.
    DropGraph(DropGraphStatement),
    /// `CREATE GRAPH TYPE` – define a graph type.
    CreateGraphType(CreateGraphTypeStatement),
    /// `CALL` – invoke a stored procedure.
    Call(CallStatement),
    /// `WITH` – pipe intermediate results for further processing.
    With(WithStatement),
    /// Composite query with set operations (UNION, INTERSECT, EXCEPT).
    CompositeQuery(CompositeQueryStatement),
    /// `CREATE INDEX` – create a property index.
    CreateIndex(CreateIndexStatement),
    /// `DROP INDEX` – drop a property index.
    DropIndex(DropIndexStatement),
}

/// A `MATCH` statement with an optional graph reference, graph pattern, and clauses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchStatement {
    /// Optional `ON <graph>` clause.
    pub graph: Option<String>,
    /// Whether this is `OPTIONAL MATCH`.
    pub optional: bool,
    /// The graph pattern to match.
    pub pattern: GraphPattern,
    /// Optional `WHERE` filter.
    pub where_clause: Option<WhereClause>,
    /// Optional `ORDER BY`.
    pub order_by: Option<OrderByClause>,
    /// Optional `LIMIT` / `OFFSET`.
    pub limit_offset: Option<LimitOffsetClause>,
}

/// A `RETURN` statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReturnStatement {
    /// If `true`, `RETURN DISTINCT`.
    pub distinct: bool,
    /// Items to return; empty means `RETURN *`.
    pub items: Vec<ReturnItem>,
    /// Optional `GROUP BY`.
    pub group_by: Option<GroupByClause>,
    /// Optional `HAVING`.
    pub having: Option<HavingClause>,
    /// Optional `ORDER BY`.
    pub order_by: Option<OrderByClause>,
    /// Optional `LIMIT` / `OFFSET`.
    pub limit_offset: Option<LimitOffsetClause>,
}

/// A `WITH` statement – projects intermediate results for further processing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WithStatement {
    /// If `true`, `WITH DISTINCT`.
    pub distinct: bool,
    /// Items to project; empty means `WITH *`.
    pub items: Vec<ReturnItem>,
    /// Optional `WHERE` filter applied after projection.
    pub where_clause: Option<WhereClause>,
    /// Optional `ORDER BY`.
    pub order_by: Option<OrderByClause>,
    /// Optional `LIMIT` / `OFFSET`.
    pub limit_offset: Option<LimitOffsetClause>,
}

/// An `INSERT` statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertStatement {
    /// Patterns describing the nodes/edges to insert.
    pub patterns: Vec<PathPattern>,
}

/// A `SET` statement – set properties or labels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetStatement {
    pub items: Vec<SetItem>,
}

/// A single item in a `SET` clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SetItem {
    /// `SET v.prop = expr`
    Property {
        target: Expression,
        property: String,
        value: Expression,
    },
    /// `SET v :Label`
    Label {
        target: Expression,
        label: String,
    },
    /// `SET v = expr` (replace all properties)
    AllProperties {
        target: Expression,
        value: Expression,
    },
}

/// A `DELETE` statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteStatement {
    /// If `true`, `DETACH DELETE`.
    pub detach: bool,
    pub targets: Vec<Expression>,
}

/// A `REMOVE` statement – remove properties or labels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoveStatement {
    pub items: Vec<RemoveItem>,
}

/// A single item in a `REMOVE` clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RemoveItem {
    /// `REMOVE v.prop`
    Property { target: Expression, property: String },
    /// `REMOVE v :Label`
    Label { target: Expression, label: String },
}

/// `CREATE GRAPH` statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateGraphStatement {
    pub name: String,
    /// Optional `IF NOT EXISTS`.
    pub if_not_exists: bool,
    /// Optional graph type reference or inline type.
    pub graph_type: Option<GraphType>,
}

/// `DROP GRAPH` statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropGraphStatement {
    pub name: String,
    /// Optional `IF EXISTS`.
    pub if_exists: bool,
}

/// `CREATE GRAPH TYPE` statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateGraphTypeStatement {
    pub name: String,
    pub if_not_exists: bool,
    pub graph_type: GraphType,
}

/// `CALL` statement – invoke a procedure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallStatement {
    pub procedure: String,
    pub arguments: Vec<Expression>,
    /// Optional `YIELD` items.
    pub yield_items: Option<Vec<String>>,
}

/// A composite query formed by set operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompositeQueryStatement {
    pub body: SetOperation,
    /// Optional trailing `ORDER BY`.
    pub order_by: Option<OrderByClause>,
    /// Optional trailing `LIMIT` / `OFFSET`.
    pub limit_offset: Option<LimitOffsetClause>,
}

/// `CREATE [UNIQUE] INDEX name FOR (v:Label) ON (v.prop1, v.prop2)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateIndexStatement {
    pub name: String,
    pub unique: bool,
    pub entity_type: String,
    pub label: Option<String>,
    pub property_names: Vec<String>,
    pub if_not_exists: bool,
}

/// `DROP INDEX name [IF EXISTS]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropIndexStatement {
    pub name: String,
    pub if_exists: bool,
}

// ---------------------------------------------------------------------------
// Set operations
// ---------------------------------------------------------------------------

/// A set operation combining two query operands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetOperation {
    pub op: SetOp,
    /// `true` when the `ALL` modifier is present.
    pub all: bool,
    pub left: Box<SetOperand>,
    pub right: Box<SetOperand>,
}

/// Left or right operand of a set operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SetOperand {
    /// A plain list of statements (a linear query).
    Query(Vec<GqlStatement>),
    /// A nested set operation.
    SetOp(SetOperation),
}

/// The kind of set operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SetOp {
    Union,
    Intersect,
    Except,
}

// ---------------------------------------------------------------------------
// Graph patterns
// ---------------------------------------------------------------------------

/// A graph pattern is a comma-separated list of path patterns with optional
/// `KEEP` / `WHERE` sub-clauses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphPattern {
    pub paths: Vec<PathPattern>,
    /// Optional mode applied to all paths.
    pub mode: Option<PathMode>,
}

/// A single path pattern composed of alternating nodes and edges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathPattern {
    /// Optional path variable binding.
    pub variable: Option<String>,
    /// Optional path mode (WALK, TRAIL, …).
    pub mode: Option<PathMode>,
    /// Optional shortest-path qualifier.
    pub shortest: Option<ShortestPathMode>,
    /// Alternating sequence of node and edge elements.
    pub elements: Vec<PatternElement>,
    /// Optional quantifier for variable-length paths.
    pub quantifier: Option<Quantifier>,
    /// Optional inline WHERE on the path.
    pub where_clause: Option<Box<WhereClause>>,
}

/// An element inside a path pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PatternElement {
    Node(NodePattern),
    Edge(EdgePattern),
}

/// A node pattern `(v:Label {prop: val})`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodePattern {
    /// Optional variable binding.
    pub variable: Option<String>,
    /// Label expressions (disjunction of conjunctions).
    pub labels: Vec<String>,
    /// Inline property predicates.
    pub properties: Vec<(String, Expression)>,
}

/// An edge pattern `-[e:REL_TYPE {prop: val}]->`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgePattern {
    /// Optional variable binding.
    pub variable: Option<String>,
    /// Direction of the edge.
    pub direction: Direction,
    /// Type/label expressions.
    pub labels: Vec<String>,
    /// Inline property predicates.
    pub properties: Vec<(String, Expression)>,
    /// Optional quantifier for variable-length edges.
    pub quantifier: Option<Quantifier>,
}

/// Path traversal mode (ISO §9.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PathMode {
    Walk,
    Trail,
    Simple,
    Acyclic,
}

/// Edge direction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Direction {
    /// `->` or `-[…]->` — outgoing edge
    Outgoing,
    /// `<-` or `<-[…]-` — incoming edge
    Incoming,
    /// `-` or `-[…]-`
    Undirected,
}

/// Quantifier for variable-length paths `{min, max}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quantifier {
    /// Minimum repetitions (defaults to 1 if absent).
    pub min: Option<u64>,
    /// Maximum repetitions (`None` = unbounded).
    pub max: Option<u64>,
}

/// Shortest-path qualifiers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ShortestPathMode {
    /// `SHORTEST`
    Shortest,
    /// `ALL SHORTEST`
    AllShortest,
    /// `ANY SHORTEST PATH`
    AnyShortestPath,
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

/// GQL expression covering all value-producing constructs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expression {
    /// A literal value.
    Literal(Literal),
    /// An identifier / variable reference.
    Identifier(String),
    /// Property access: `expr.property`.
    PropertyAccess {
        object: Box<Expression>,
        property: String,
    },
    /// Function call: `name(args…)`.
    FunctionCall {
        name: String,
        args: Vec<Expression>,
    },
    /// Binary operation: `left op right`.
    BinaryOp {
        left: Box<Expression>,
        op: BinaryOp,
        right: Box<Expression>,
    },
    /// Unary operation: `op expr`.
    UnaryOp {
        op: UnaryOp,
        operand: Box<Expression>,
    },
    /// `CASE` expression (simple and searched forms).
    Case {
        /// Optional operand for simple `CASE expr`.
        operand: Option<Box<Expression>>,
        /// `WHEN … THEN …` arms.
        when_clauses: Vec<(Expression, Expression)>,
        /// Optional `ELSE` branch.
        else_clause: Option<Box<Expression>>,
    },
    /// `EXISTS { subquery }`.
    Exists {
        subquery: Box<GqlStatement>,
    },
    /// Aggregate function call.
    Aggregate {
        function: AggregateFunction,
        /// `true` for `COUNT(DISTINCT x)`.
        distinct: bool,
        arg: Option<Box<Expression>>,
    },
    /// List constructor `[expr, …]`.
    List(Vec<Expression>),
    /// Map constructor `{key: expr, …}`.
    Map(Vec<(String, Expression)>),
    /// A named or positional parameter reference (`$name`).
    Parameter(String),
    /// Scalar subquery `(SELECT …)`.
    Subquery(Box<GqlStatement>),
    /// `expr IS NULL`.
    IsNull(Box<Expression>),
    /// `expr IS NOT NULL`.
    IsNotNull(Box<Expression>),
    /// `expr IN (list)`.
    In {
        operand: Box<Expression>,
        list: Box<Expression>,
    },
    /// `expr LIKE pattern`.
    Like {
        operand: Box<Expression>,
        pattern: Box<Expression>,
    },
    /// Type cast `CAST(expr AS type)`.
    TypeCast {
        expr: Box<Expression>,
        target_type: String,
    },
    /// A path expression (first-class path value).
    Path(Box<PathPattern>),
}

/// Binary operators.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    Xor,
    /// String/list concatenation `||`.
    Concat,
    Eq,
    Neq,
    Lt,
    Gt,
    Le,
    Ge,
}

/// Unary operators.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UnaryOp {
    Not,
    Neg,
    Pos,
}

/// Aggregate functions recognised by GQL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AggregateFunction {
    Count,
    Sum,
    Avg,
    Min,
    Max,
    Collect,
    StdDev,
    Percentile,
}

// ---------------------------------------------------------------------------
// Literals
// ---------------------------------------------------------------------------

/// Scalar and composite literal values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Literal {
    Integer(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Null,
    /// ISO 8601 date string.
    Date(String),
    /// ISO 8601 time string.
    Time(String),
    /// ISO 8601 timestamp string.
    Timestamp(String),
    /// ISO 8601 duration string.
    Duration(String),
    /// List literal `[val, …]`.
    List(Vec<Literal>),
    /// Map literal `{key: val, …}`.
    Map(Vec<(String, Literal)>),
}

// ---------------------------------------------------------------------------
// Clauses
// ---------------------------------------------------------------------------

/// `WHERE <expr>` clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhereClause {
    pub condition: Box<Expression>,
}

/// `ORDER BY` clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderByClause {
    pub items: Vec<OrderByItem>,
}

/// A single item in an `ORDER BY` clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderByItem {
    pub expression: Expression,
    pub direction: SortDirection,
}

/// Sort direction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SortDirection {
    Asc,
    Desc,
}

/// `LIMIT` / `OFFSET` clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitOffsetClause {
    pub limit: Option<Expression>,
    pub offset: Option<Expression>,
}

/// `GROUP BY` clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupByClause {
    pub expressions: Vec<Expression>,
}

/// `HAVING` clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HavingClause {
    pub condition: Expression,
}

/// `WITH` clause – pipe intermediate results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WithClause {
    pub distinct: bool,
    pub items: Vec<ReturnItem>,
    pub where_clause: Option<WhereClause>,
    pub order_by: Option<OrderByClause>,
    pub limit_offset: Option<LimitOffsetClause>,
}

/// `LET` clause – bind a variable to an expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LetClause {
    pub variable: String,
    pub value: Expression,
}

/// `FOR` clause – iterate over a list binding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForClause {
    pub variable: String,
    pub list: Expression,
    pub body: Vec<GqlStatement>,
}

/// `FILTER` clause – inline predicate filter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterClause {
    pub condition: Expression,
}

/// A single return / projection item: `<expr> [AS <alias>]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReturnItem {
    pub expression: Expression,
    pub alias: Option<String>,
}

// ---------------------------------------------------------------------------
// Schema / graph type definitions
// ---------------------------------------------------------------------------

/// A graph type definition containing node and edge type declarations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphType {
    pub node_types: Vec<NodeType>,
    pub edge_types: Vec<EdgeType>,
}

/// A node type declaration within a graph type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeType {
    pub labels: Vec<String>,
    pub properties: Vec<PropertyDecl>,
}

/// An edge type declaration within a graph type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeType {
    pub labels: Vec<String>,
    /// Source node type labels.
    pub source: Vec<String>,
    /// Destination node type labels.
    pub destination: Vec<String>,
    pub direction: Direction,
    pub properties: Vec<PropertyDecl>,
}

/// A property declaration inside a node or edge type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropertyDecl {
    pub name: String,
    /// Property type name (e.g. `"STRING"`, `"INT64"`).
    pub property_type: String,
    /// Whether the property is required (`NOT NULL`).
    pub required: bool,
}