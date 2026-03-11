use crate::expression::evaluate;
use crate::operators::ExecutionContext;
use crate::result::{Record, ResultSet};
use synaptica_core::graph::{Edge, GraphId, Label, Node, NodeId};
use synaptica_core::types::Value;
use synaptica_gql::ast::{Direction, Expression};
use synaptica_gql::planner::LogicalPlan;
use synaptica_storage::engine::StorageEngine;
use std::cmp::Ordering;
use std::fmt;
use uuid::Uuid;

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
            LogicalPlan::Scan { labels, variable, .. } => self.exec_scan(labels, variable, ctx),
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
            LogicalPlan::Join { left, right } => {
                let left_rs = self.execute_node(left, ctx)?;
                let right_rs = self.execute_node(right, ctx)?;
                self.exec_join(left_rs, right_rs)
            }
            LogicalPlan::CreateEdgeFromMatch { input, source_var, target_var, label, properties } => {
                let rs = self.execute_node(input, ctx)?;
                self.exec_create_edge_from_match(rs, source_var, target_var, label, properties, ctx)
            }
            LogicalPlan::Empty => Ok(ResultSet::new(vec![])),
            LogicalPlan::Expand {
                input,
                edge_label,
                direction,
                target_labels,
                edge_variable,
                target_variable,
            } => {
                let rs = self.execute_node(input, ctx)?;
                self.exec_expand(rs, edge_label, direction, target_labels, edge_variable, target_variable, ctx)
            }
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
        variable: &Option<String>,
        ctx: &ExecutionContext<'_>,
    ) -> Result<ResultSet, ExecError> {
        let nodes = if labels.is_empty() {
            ctx.storage.scan_nodes(&ctx.graph_id)?
        } else {
            let label = Label::new(&labels[0]);
            let mut result = ctx.storage.scan_nodes_by_label(&ctx.graph_id, &label)?;
            // Filter by ALL required labels, not just the first
            if labels.len() > 1 {
                result.retain(|node| labels.iter().all(|l| node.has_label(l)));
            }
            result
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

        if let Some(var) = variable {
            columns.push(var.clone());
            for key in &all_keys {
                columns.push(format!("{}.{}", var, key));
            }
        }

        let mut rs = ResultSet::new(columns);
        for node in &nodes {
            let mut values: Vec<Value> = Vec::new();
            let id_str = node.id.0.to_string();
            values.push(Value::String(id_str.clone()));
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

            if let Some(_var) = variable {
                let node_val = Value::Node {
                    id: id_str,
                    labels: node.labels.iter().map(|l| l.0.clone()).collect(),
                    properties: node.properties.clone(),
                };
                values.push(node_val);
                for key in &all_keys {
                    values.push(
                        node.properties
                            .get(key)
                            .cloned()
                            .unwrap_or(Value::Null),
                    );
                }
            }

            rs.add_record(values);
        }
        Ok(rs)
    }

    // -- Expand -------------------------------------------------------------

    fn exec_expand(
        &self,
        input: ResultSet,
        edge_label: &str,
        direction: &Direction,
        target_labels: &[String],
        edge_variable: &Option<String>,
        target_variable: &Option<String>,
        ctx: &ExecutionContext<'_>,
    ) -> Result<ResultSet, ExecError> {
        struct ExpandRow {
            source_values: Vec<Value>,
            edge: Edge,
            target: Node,
        }
        let mut expanded: Vec<ExpandRow> = Vec::new();

        let label_filter = if edge_label.is_empty() {
            None
        } else {
            Some(Label::new(edge_label))
        };

        for record in &input.records {
            let node_id_str = record
                .get("__node_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ExecError::Internal("missing __node_id in expand input".into()))?;

            let node_uuid = Uuid::parse_str(node_id_str)
                .map_err(|e| ExecError::Internal(format!("invalid node id: {}", e)))?;
            let node_id = NodeId(node_uuid);

            let edges = match direction {
                Direction::Outgoing => ctx.storage.get_outgoing_edges(&ctx.graph_id, &node_id, label_filter.as_ref())?,
                Direction::Incoming => ctx.storage.get_incoming_edges(&ctx.graph_id, &node_id, label_filter.as_ref())?,
                Direction::Undirected => {
                    let mut edges = ctx.storage.get_outgoing_edges(&ctx.graph_id, &node_id, label_filter.as_ref())?;
                    let outgoing_ids: std::collections::HashSet<_> = edges.iter().map(|e| e.id).collect();
                    // Only add incoming edges not already seen (avoids self-loop duplicates)
                    for e in ctx.storage.get_incoming_edges(&ctx.graph_id, &node_id, label_filter.as_ref())? {
                        if !outgoing_ids.contains(&e.id) {
                            edges.push(e);
                        }
                    }
                    edges
                }
            };

            for edge in edges {
                // Determine target node based on direction
                let target_id = match direction {
                    Direction::Incoming => edge.source,
                    _ => edge.target,
                };

                let target = match ctx.storage.get_node(&ctx.graph_id, &target_id) {
                    Ok(n) => n,
                    Err(_) => continue,
                };

                // Filter by target labels if specified
                if !target_labels.is_empty() {
                    let has_label = target_labels.iter().any(|l| target.has_label(l));
                    if !has_label {
                        continue;
                    }
                }

                expanded.push(ExpandRow {
                    source_values: record.values.clone(),
                    edge,
                    target,
                });
            }
        }

        // Collect property keys across all expanded rows
        let mut target_keys: Vec<String> = Vec::new();
        let mut edge_keys: Vec<String> = Vec::new();
        for row in &expanded {
            for k in row.target.properties.keys() {
                if !target_keys.contains(k) {
                    target_keys.push(k.clone());
                }
            }
            for k in row.edge.properties.keys() {
                if !edge_keys.contains(k) {
                    edge_keys.push(k.clone());
                }
            }
        }

        // Build output columns
        let mut columns = input.columns.clone();
        if let Some(ev) = edge_variable {
            columns.push(ev.clone());
            for k in &edge_keys {
                columns.push(format!("{}.{}", ev, k));
            }
        }
        if let Some(tv) = target_variable {
            columns.push(tv.clone());
            for k in &target_keys {
                columns.push(format!("{}.{}", tv, k));
            }
        }

        let mut rs = ResultSet::new(columns);

        for row in &expanded {
            let mut values = row.source_values.clone();

            if let Some(_ev) = edge_variable {
                values.push(Value::Edge {
                    id: row.edge.id.0.to_string(),
                    label: row.edge.label.0.clone(),
                    source_id: row.edge.source.0.to_string(),
                    target_id: row.edge.target.0.to_string(),
                    properties: row.edge.properties.clone(),
                });
                for k in &edge_keys {
                    values.push(row.edge.properties.get(k).cloned().unwrap_or(Value::Null));
                }
            }

            if let Some(_tv) = target_variable {
                let id_str = row.target.id.0.to_string();
                values.push(Value::Node {
                    id: id_str,
                    labels: row.target.labels.iter().map(|l| l.0.clone()).collect(),
                    properties: row.target.properties.clone(),
                });
                for k in &target_keys {
                    values.push(row.target.properties.get(k).cloned().unwrap_or(Value::Null));
                }
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

    // -- Join ---------------------------------------------------------------

    fn exec_join(
        &self,
        left: ResultSet,
        right: ResultSet,
    ) -> Result<ResultSet, ExecError> {
        let mut columns = left.columns.clone();
        columns.extend(right.columns.clone());
        let mut rs = ResultSet::new(columns);
        for l in &left.records {
            for r in &right.records {
                let mut values = l.values.clone();
                values.extend(r.values.clone());
                rs.add_record(values);
            }
        }
        Ok(rs)
    }

    // -- CreateEdgeFromMatch ------------------------------------------------

    fn exec_create_edge_from_match(
        &self,
        input: ResultSet,
        source_var: &str,
        target_var: &str,
        label: &str,
        properties: &[(String, Expression)],
        ctx: &ExecutionContext<'_>,
    ) -> Result<ResultSet, ExecError> {
        let mut rs = ResultSet::new(vec!["__edge_id".to_string()]);
        let dummy = Record::new(vec![], vec![]);

        for record in &input.records {
            let source_id_str = record
                .get(source_var)
                .and_then(|v| v.as_node_id().map(|s| s.to_string()))
                .or_else(|| {
                    let col = format!("{}.__node_id", source_var);
                    record.get(&col).and_then(|v| v.as_str().map(|s| s.to_string()))
                })
                .or_else(|| {
                    record.get("__node_id").and_then(|v| v.as_str().map(|s| s.to_string()))
                })
                .ok_or_else(|| ExecError::Internal(format!("cannot resolve source node from variable '{}'", source_var)))?;

            let target_id_str = record
                .get(target_var)
                .and_then(|v| v.as_node_id().map(|s| s.to_string()))
                .or_else(|| {
                    let col = format!("{}.__node_id", target_var);
                    record.get(&col).and_then(|v| v.as_str().map(|s| s.to_string()))
                })
                .ok_or_else(|| ExecError::Internal(format!("cannot resolve target node from variable '{}'", target_var)))?;

            let source_uuid = Uuid::parse_str(&source_id_str)
                .map_err(|e| ExecError::Internal(format!("invalid source UUID: {}", e)))?;
            let target_uuid = Uuid::parse_str(&target_id_str)
                .map_err(|e| ExecError::Internal(format!("invalid target UUID: {}", e)))?;

            let mut edge = Edge::new(
                ctx.graph_id,
                NodeId(source_uuid),
                NodeId(target_uuid),
                label,
            );
            for (key, expr) in properties {
                let val = evaluate(expr, &dummy)?;
                edge.set_property(key.clone(), val);
            }
            ctx.storage.put_edge(&edge)?;
            rs.add_record(vec![Value::String(edge.id.0.to_string())]);
        }
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