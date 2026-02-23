use crate::engine::ExecError;
use crate::expression::evaluate;
use crate::result::{Record, ResultSet};
use synaptica_core::graph::{GraphId, Label, Node};
use synaptica_core::types::Value;
use synaptica_gql::ast::Expression;
use synaptica_storage::engine::StorageEngine;

/// Shared execution context passed to operators.
pub struct ExecutionContext<'a> {
    pub storage: &'a StorageEngine,
    pub graph_id: GraphId,
}

/// Trait for physical query operators.
pub trait Operator {
    fn execute(&self, context: &ExecutionContext<'_>) -> Result<ResultSet, ExecError>;
}

// ---------------------------------------------------------------------------
// ScanNodes
// ---------------------------------------------------------------------------

pub struct ScanNodes {
    pub labels: Vec<String>,
}

impl Operator for ScanNodes {
    fn execute(&self, ctx: &ExecutionContext<'_>) -> Result<ResultSet, ExecError> {
        let nodes = if self.labels.is_empty() {
            ctx.storage.scan_nodes(&ctx.graph_id)?
        } else {
            let label = Label::new(&self.labels[0]);
            ctx.storage.scan_nodes_by_label(&ctx.graph_id, &label)?
        };

        let mut all_keys: Vec<String> = Vec::new();
        for node in &nodes {
            for key in node.properties.keys() {
                if !all_keys.contains(key) {
                    all_keys.push(key.clone());
                }
            }
        }

        let mut columns = vec!["__node_id".to_string(), "__labels".to_string()];
        columns.extend(all_keys.clone());

        let mut rs = ResultSet::new(columns);
        for node in &nodes {
            let mut values: Vec<Value> = Vec::new();
            values.push(Value::String(node.id.0.to_string()));
            let label_list: Vec<Value> = node
                .labels
                .iter()
                .map(|l| Value::String(l.0.clone()))
                .collect();
            values.push(Value::List(label_list));
            for key in &all_keys {
                values.push(node.properties.get(key).cloned().unwrap_or(Value::Null));
            }
            rs.add_record(values);
        }
        Ok(rs)
    }
}

// ---------------------------------------------------------------------------
// FilterOp
// ---------------------------------------------------------------------------

pub struct FilterOp {
    pub input: ResultSet,
    pub predicate: Expression,
}

impl Operator for FilterOp {
    fn execute(&self, _ctx: &ExecutionContext<'_>) -> Result<ResultSet, ExecError> {
        let mut rs = ResultSet::new(self.input.columns.clone());
        for record in &self.input.records {
            let val = evaluate(&self.predicate, record)?;
            if val == Value::Bool(true) {
                rs.add_record(record.values.clone());
            }
        }
        Ok(rs)
    }
}

// ---------------------------------------------------------------------------
// ProjectOp
// ---------------------------------------------------------------------------

pub struct ProjectOp {
    pub input: ResultSet,
    pub expressions: Vec<Expression>,
    pub aliases: Vec<String>,
}

impl Operator for ProjectOp {
    fn execute(&self, _ctx: &ExecutionContext<'_>) -> Result<ResultSet, ExecError> {
        let mut rs = ResultSet::new(self.aliases.clone());
        for record in &self.input.records {
            let mut values = Vec::with_capacity(self.expressions.len());
            for expr in &self.expressions {
                values.push(evaluate(expr, record)?);
            }
            rs.add_record(values);
        }
        Ok(rs)
    }
}

// ---------------------------------------------------------------------------
// LimitOp
// ---------------------------------------------------------------------------

pub struct LimitOp {
    pub input: ResultSet,
    pub count: Option<u64>,
    pub offset: Option<u64>,
}

impl Operator for LimitOp {
    fn execute(&self, _ctx: &ExecutionContext<'_>) -> Result<ResultSet, ExecError> {
        let skip = self.offset.unwrap_or(0) as usize;
        let take = self.count.map(|c| c as usize).unwrap_or(usize::MAX);
        let mut rs = ResultSet::new(self.input.columns.clone());
        for record in self.input.records.iter().skip(skip).take(take) {
            rs.add_record(record.values.clone());
        }
        Ok(rs)
    }
}

// ---------------------------------------------------------------------------
// SortOp (stub)
// ---------------------------------------------------------------------------

pub struct SortOp {
    pub input: ResultSet,
    pub order_by: Vec<Expression>,
}

impl Operator for SortOp {
    fn execute(&self, _ctx: &ExecutionContext<'_>) -> Result<ResultSet, ExecError> {
        // Sorting is handled inline in ExecutionEngine::exec_sort for now.
        Ok(self.input.clone())
    }
}

// ---------------------------------------------------------------------------
// ExpandOp (stub)
// ---------------------------------------------------------------------------

pub struct ExpandOp {
    pub input: ResultSet,
    pub edge_label: String,
    pub direction: synaptica_gql::ast::Direction,
    pub target_labels: Vec<String>,
}

impl Operator for ExpandOp {
    fn execute(&self, _ctx: &ExecutionContext<'_>) -> Result<ResultSet, ExecError> {
        Err(ExecError::NotImplemented("ExpandOp".into()))
    }
}

// ---------------------------------------------------------------------------
// InsertNodeOp (stub)
// ---------------------------------------------------------------------------

pub struct InsertNodeOp {
    pub labels: Vec<String>,
    pub properties: Vec<(String, Expression)>,
}

impl Operator for InsertNodeOp {
    fn execute(&self, ctx: &ExecutionContext<'_>) -> Result<ResultSet, ExecError> {
        let mut node = Node::new(ctx.graph_id);
        for l in &self.labels {
            node.add_label(l.as_str());
        }
        let dummy = Record::new(vec![], vec![]);
        for (key, expr) in &self.properties {
            let val = evaluate(expr, &dummy)?;
            node.set_property(key.clone(), val);
        }
        ctx.storage.put_node(&node)?;

        let mut rs = ResultSet::new(vec!["__node_id".to_string()]);
        rs.add_record(vec![Value::String(node.id.0.to_string())]);
        Ok(rs)
    }
}

// ---------------------------------------------------------------------------
// DeleteNodeOp (stub)
// ---------------------------------------------------------------------------

pub struct DeleteNodeOp {
    pub input: ResultSet,
}

impl Operator for DeleteNodeOp {
    fn execute(&self, _ctx: &ExecutionContext<'_>) -> Result<ResultSet, ExecError> {
        Err(ExecError::NotImplemented("DeleteNodeOp".into()))
    }
}

// ---------------------------------------------------------------------------
// SetPropertyOp (stub)
// ---------------------------------------------------------------------------

pub struct SetPropertyOp {
    pub input: ResultSet,
    pub property: String,
    pub value: Expression,
}

impl Operator for SetPropertyOp {
    fn execute(&self, _ctx: &ExecutionContext<'_>) -> Result<ResultSet, ExecError> {
        Err(ExecError::NotImplemented("SetPropertyOp".into()))
    }
}