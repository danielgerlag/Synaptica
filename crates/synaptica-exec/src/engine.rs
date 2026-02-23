use crate::expression::evaluate;
use crate::operators::ExecutionContext;
use crate::result::{Record, ResultSet};
use synaptica_core::graph::{GraphId, Label, Node};
use synaptica_core::types::Value;
use synaptica_gql::ast::Expression;
use synaptica_gql::planner::LogicalPlan;
use synaptica_storage::engine::StorageEngine;
use std::cmp::Ordering;
use std::fmt;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ExecError {
    NotImplemented(String),
    StorageError(String),
    TypeError(String),
    ExpressionError(String),
    Internal(String),
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecError::NotImplemented(msg) => write!(f, "not implemented: {}", msg),
            ExecError::StorageError(msg) => write!(f, "storage error: {}", msg),
            ExecError::TypeError(msg) => write!(f, "type error: {}", msg),
            ExecError::ExpressionError(msg) => write!(f, "expression error: {}", msg),
            ExecError::Internal(msg) => write!(f, "internal error: {}", msg),
        }
    }
}

impl std::error::Error for ExecError {}

impl From<synaptica_storage::engine::StorageError> for ExecError {
    fn from(e: synaptica_storage::engine::StorageError) -> Self {
        ExecError::StorageError(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Execution engine
// ---------------------------------------------------------------------------

pub struct ExecutionEngine<'a> {
    storage: &'a StorageEngine,
}

impl<'a> ExecutionEngine<'a> {
    pub fn new(storage: &'a StorageEngine) -> Self {
        Self { storage }
    }

    pub fn execute_plan(
        &self,
        plan: &LogicalPlan,
        graph_id: &GraphId,
    ) -> Result<ResultSet, ExecError> {
        let ctx = ExecutionContext {
            storage: self.storage,
            graph_id: *graph_id,
        };
        self.execute_node(plan, &ctx)
    }

    fn execute_node(
        &self,
        plan: &LogicalPlan,
        ctx: &ExecutionContext<'_>,
    ) -> Result<ResultSet, ExecError> {
        match plan {
            LogicalPlan::Scan { labels, .. } => self.exec_scan(labels, ctx),
            LogicalPlan::Filter { input, predicate } => {
                let rs = self.execute_node(input, ctx)?;
                self.exec_filter(rs, predicate)
            }
            LogicalPlan::Project { input, expressions } => {
                let rs = self.execute_node(input, ctx)?;
                self.exec_project(rs, expressions)
            }
            LogicalPlan::Limit {
                input,
                count,
                offset,
            } => {
                let rs = self.execute_node(input, ctx)?;
                self.exec_limit(rs, *count, *offset)
            }
            LogicalPlan::Sort { input, order_by } => {
                let rs = self.execute_node(input, ctx)?;
                self.exec_sort(rs, order_by)
            }
            LogicalPlan::CreateNode { labels, properties } => {
                self.exec_create_node(labels, properties, ctx)
            }
            LogicalPlan::Empty => Ok(ResultSet::new(vec![])),
            _ => Err(ExecError::NotImplemented(format!(
                "{:?}",
                std::mem::discriminant(plan)
            ))),
        }
    }

    // -- Scan ---------------------------------------------------------------

    fn exec_scan(
        &self,
        labels: &[String],
        ctx: &ExecutionContext<'_>,
    ) -> Result<ResultSet, ExecError> {
        let nodes = if labels.is_empty() {
            ctx.storage.scan_nodes(&ctx.graph_id)?
        } else {
            let label = Label::new(&labels[0]);
            ctx.storage.scan_nodes_by_label(&ctx.graph_id, &label)?
        };

        // Build a result set with columns: __node_id, __labels, + all property keys
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
                values.push(
                    node.properties
                        .get(key)
                        .cloned()
                        .unwrap_or(Value::Null),
                );
            }
            rs.add_record(values);
        }
        Ok(rs)
    }

    // -- Filter -------------------------------------------------------------

    fn exec_filter(
        &self,
        input: ResultSet,
        predicate: &Expression,
    ) -> Result<ResultSet, ExecError> {
        let mut rs = ResultSet::new(input.columns.clone());
        for record in &input.records {
            let val = evaluate(predicate, record)?;
            if val == Value::Bool(true) {
                rs.add_record(record.values.clone());
            }
        }
        Ok(rs)
    }

    // -- Project ------------------------------------------------------------

    fn exec_project(
        &self,
        input: ResultSet,
        expressions: &[Expression],
    ) -> Result<ResultSet, ExecError> {
        let columns: Vec<String> = expressions
            .iter()
            .enumerate()
            .map(|(i, expr)| expr_column_name(expr, i))
            .collect();

        let mut rs = ResultSet::new(columns);
        for record in &input.records {
            let mut values = Vec::with_capacity(expressions.len());
            for expr in expressions {
                values.push(evaluate(expr, record)?);
            }
            rs.add_record(values);
        }
        Ok(rs)
    }

    // -- Limit --------------------------------------------------------------

    fn exec_limit(
        &self,
        input: ResultSet,
        count: Option<u64>,
        offset: Option<u64>,
    ) -> Result<ResultSet, ExecError> {
        let skip = offset.unwrap_or(0) as usize;
        let take = count.map(|c| c as usize).unwrap_or(usize::MAX);
        let mut rs = ResultSet::new(input.columns.clone());
        for record in input.records.into_iter().skip(skip).take(take) {
            rs.add_record(record.values);
        }
        Ok(rs)
    }

    // -- Sort ---------------------------------------------------------------

    fn exec_sort(
        &self,
        mut input: ResultSet,
        order_by: &[Expression],
    ) -> Result<ResultSet, ExecError> {
        // Evaluate sort keys for each record, then sort
        let mut err: Option<ExecError> = None;
        input.records.sort_by(|a, b| {
            if err.is_some() {
                return Ordering::Equal;
            }
            for expr in order_by {
                let va = match evaluate(expr, a) {
                    Ok(v) => v,
                    Err(e) => {
                        err = Some(e);
                        return Ordering::Equal;
                    }
                };
                let vb = match evaluate(expr, b) {
                    Ok(v) => v,
                    Err(e) => {
                        err = Some(e);
                        return Ordering::Equal;
                    }
                };
                let ord = cmp_values(&va, &vb);
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            Ordering::Equal
        });
        if let Some(e) = err {
            return Err(e);
        }
        Ok(input)
    }

    // -- CreateNode ---------------------------------------------------------

    fn exec_create_node(
        &self,
        labels: &[String],
        properties: &[(String, Expression)],
        ctx: &ExecutionContext<'_>,
    ) -> Result<ResultSet, ExecError> {
        let mut node = Node::new(ctx.graph_id);
        for l in labels {
            node.add_label(l.as_str());
        }
        let dummy = Record::new(vec![], vec![]);
        for (key, expr) in properties {
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
// Helpers
// ---------------------------------------------------------------------------

fn expr_column_name(expr: &Expression, idx: usize) -> String {
    match expr {
        Expression::Identifier(name) => name.clone(),
        Expression::PropertyAccess { object, property } => {
            format!("{}.{}", expr_column_name(object, idx), property)
        }
        _ => format!("col_{}", idx),
    }
}

fn cmp_values(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Less,
        (_, Value::Null) => Ordering::Greater,
        (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Integer(x), Value::Float(y)) => {
            (*x as f64).partial_cmp(y).unwrap_or(Ordering::Equal)
        }
        (Value::Float(x), Value::Integer(y)) => {
            x.partial_cmp(&(*y as f64)).unwrap_or(Ordering::Equal)
        }
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => Ordering::Equal,
    }
}