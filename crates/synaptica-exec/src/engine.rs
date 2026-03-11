use crate::expression::evaluate;
use crate::operators::ExecutionContext;
use crate::result::{Record, ResultSet};
use synaptica_core::graph::{Edge, GraphId, Label, Node, NodeId};
use synaptica_core::types::Value;
use synaptica_gql::ast::{Direction, Expression, SortDirection};
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
            LogicalPlan::Project { input, expressions, aliases } => {
                let rs = self.execute_node(input, ctx)?;
                self.exec_project(rs, expressions, aliases)
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
            LogicalPlan::CallProcedure { procedure, arguments: _, yield_items } => {
                self.exec_call_procedure(procedure, yield_items, ctx)
            }
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
            LogicalPlan::DeleteNode { input } => {
                let rs = self.execute_node(input, ctx)?;
                self.exec_delete_nodes(rs, ctx)
            }
            LogicalPlan::SetProperty { input, target, property, value } => {
                let rs = self.execute_node(input, ctx)?;
                self.exec_set_property(rs, target.as_deref(), property, value, ctx)
            }
            LogicalPlan::Distinct { input } => {
                let rs = self.execute_node(input, ctx)?;
                self.exec_distinct(rs)
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
                // Determine the neighbor node: for outgoing edges it's the target,
                // for incoming edges it's the source, for undirected we pick the "other" end.
                let target_id = match direction {
                    Direction::Incoming => edge.source,
                    Direction::Outgoing => edge.target,
                    Direction::Undirected => {
                        if edge.source == node_id {
                            edge.target
                        } else {
                            edge.source
                        }
                    }
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

        // Find __node_id column index so we can update it for chained traversals
        let node_id_col_idx = rs.columns.iter().position(|c| c == "__node_id");

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
                    id: id_str.clone(),
                    labels: row.target.labels.iter().map(|l| l.0.clone()).collect(),
                    properties: row.target.properties.clone(),
                });
                for k in &target_keys {
                    values.push(row.target.properties.get(k).cloned().unwrap_or(Value::Null));
                }
                // Update __node_id to the target for chained multi-hop traversals
                if let Some(idx) = node_id_col_idx {
                    values[idx] = Value::String(id_str);
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
        aliases: &[Option<String>],
    ) -> Result<ResultSet, ExecError> {
        // Check if any expression contains an aggregate
        let has_agg = expressions.iter().any(|e| Self::contains_aggregate(e));

        if has_agg {
            return self.exec_project_with_aggregation(input, expressions, aliases);
        }

        let mut columns: Vec<String> = expressions
            .iter()
            .enumerate()
            .map(|(i, expr)| {
                if let Some(alias) = aliases.get(i).and_then(|a| a.as_ref()) {
                    alias.clone()
                } else {
                    expr_column_name(expr, i)
                }
            })
            .collect();

        // Carry through internal columns (__node_id, __labels) when present
        // in the input but not explicitly projected — needed for WITH pipelines
        let internal_cols: Vec<String> = input.columns.iter()
            .filter(|c| c.starts_with("__") && !columns.contains(c))
            .cloned()
            .collect();
        columns.extend(internal_cols.iter().cloned());

        let mut rs = ResultSet::new(columns);

        // If input is empty (standalone RETURN without MATCH), produce one row
        if input.records.is_empty() && input.columns.is_empty() {
            let mut values = Vec::with_capacity(expressions.len());
            let empty_record = Record { columns: vec![], values: vec![] };
            for expr in expressions {
                values.push(evaluate(expr, &empty_record)?);
            }
            rs.add_record(values);
            return Ok(rs);
        }

        for record in &input.records {
            let mut values = Vec::with_capacity(expressions.len() + internal_cols.len());
            for expr in expressions {
                values.push(evaluate(expr, record)?);
            }
            // Append internal column values
            for ic in &internal_cols {
                values.push(record.get(ic).cloned().unwrap_or(Value::Null));
            }
            rs.add_record(values);
        }
        Ok(rs)
    }

    fn contains_aggregate(expr: &Expression) -> bool {
        match expr {
            Expression::Aggregate { .. } => true,
            Expression::BinaryOp { left, right, .. } => {
                Self::contains_aggregate(left) || Self::contains_aggregate(right)
            }
            Expression::UnaryOp { operand, .. } => Self::contains_aggregate(operand),
            Expression::FunctionCall { args, .. } => args.iter().any(|a| Self::contains_aggregate(a)),
            _ => false,
        }
    }

    fn exec_project_with_aggregation(
        &self,
        input: ResultSet,
        expressions: &[Expression],
        aliases: &[Option<String>],
    ) -> Result<ResultSet, ExecError> {
        use std::collections::BTreeMap;
        use synaptica_gql::ast::AggregateFunction;

        let columns: Vec<String> = expressions
            .iter()
            .enumerate()
            .map(|(i, expr)| {
                if let Some(alias) = aliases.get(i).and_then(|a| a.as_ref()) {
                    alias.clone()
                } else {
                    expr_column_name(expr, i)
                }
            })
            .collect();

        // Separate group-by keys (non-aggregate) and aggregate expressions
        let group_indices: Vec<usize> = expressions.iter().enumerate()
            .filter(|(_, e)| !Self::contains_aggregate(e))
            .map(|(i, _)| i)
            .collect();

        // Group records by the group-by keys
        let mut groups: BTreeMap<String, Vec<&Record>> = BTreeMap::new();
        for record in &input.records {
            let mut key = String::new();
            for &gi in &group_indices {
                let val = evaluate(&expressions[gi], record)?;
                key.push_str(&format!("{:?}|", val));
            }
            groups.entry(key).or_default().push(record);
        }

        let mut rs = ResultSet::new(columns);

        // When no GROUP BY and empty input, produce one row with aggregate defaults
        if groups.is_empty() && group_indices.is_empty() {
            let mut values = Vec::with_capacity(expressions.len());
            for expr in expressions {
                if let Expression::Aggregate { function, .. } = expr {
                    let default_val = match function {
                        AggregateFunction::Count => Value::Integer(0),
                        AggregateFunction::Sum => Value::Integer(0),
                        AggregateFunction::Collect => Value::List(vec![]),
                        _ => Value::Null,
                    };
                    values.push(default_val);
                } else {
                    values.push(Value::Null);
                }
            }
            rs.add_record(values);
            return Ok(rs);
        }

        for (_key, records) in &groups {
            let mut values = Vec::with_capacity(expressions.len());
            let first_record = records[0];

            for expr in expressions {
                if let Expression::Aggregate { function, arg, distinct } = expr {
                    let agg_val = match function {
                        AggregateFunction::Count => {
                            if arg.is_none() {
                                Value::Integer(records.len() as i64)
                            } else if *distinct {
                                let mut seen = std::collections::HashSet::new();
                                for r in records {
                                    if let Ok(v) = evaluate(arg.as_ref().unwrap(), r) {
                                        if v != Value::Null {
                                            seen.insert(format!("{:?}", v));
                                        }
                                    }
                                }
                                Value::Integer(seen.len() as i64)
                            } else {
                                let count = records.iter()
                                    .filter(|r| {
                                        evaluate(arg.as_ref().unwrap(), r)
                                            .map(|v| v != Value::Null)
                                            .unwrap_or(false)
                                    })
                                    .count();
                                Value::Integer(count as i64)
                            }
                        }
                        AggregateFunction::Sum => {
                            let mut total = 0i64;
                            let mut has_float = false;
                            let mut ftotal = 0.0f64;
                            for r in records {
                                if let Some(inner) = arg {
                                    match evaluate(inner, r)? {
                                        Value::Integer(n) => { total += n; ftotal += n as f64; }
                                        Value::Float(f) => { has_float = true; ftotal += f; }
                                        _ => {}
                                    }
                                }
                            }
                            if has_float { Value::Float(ftotal) } else { Value::Integer(total) }
                        }
                        AggregateFunction::Avg => {
                            let mut sum = 0.0f64;
                            let mut count = 0usize;
                            for r in records {
                                if let Some(inner) = arg {
                                    match evaluate(inner, r)? {
                                        Value::Integer(n) => { sum += n as f64; count += 1; }
                                        Value::Float(f) => { sum += f; count += 1; }
                                        _ => {}
                                    }
                                }
                            }
                            if count > 0 { Value::Float(sum / count as f64) } else { Value::Null }
                        }
                        AggregateFunction::Min => {
                            let mut min_val = Value::Null;
                            for r in records {
                                if let Some(inner) = arg {
                                    let v = evaluate(inner, r)?;
                                    if min_val == Value::Null {
                                        min_val = v;
                                    } else {
                                        match (&v, &min_val) {
                                            (Value::Integer(a), Value::Integer(b)) if a < b => min_val = v,
                                            (Value::Float(a), Value::Float(b)) if a < b => min_val = v,
                                            (Value::String(a), Value::String(b)) if a < b => min_val = v,
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            min_val
                        }
                        AggregateFunction::Max => {
                            let mut max_val = Value::Null;
                            for r in records {
                                if let Some(inner) = arg {
                                    let v = evaluate(inner, r)?;
                                    if max_val == Value::Null {
                                        max_val = v;
                                    } else {
                                        match (&v, &max_val) {
                                            (Value::Integer(a), Value::Integer(b)) if a > b => max_val = v,
                                            (Value::Float(a), Value::Float(b)) if a > b => max_val = v,
                                            (Value::String(a), Value::String(b)) if a > b => max_val = v,
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            max_val
                        }
                        AggregateFunction::Collect => {
                            let mut items = Vec::new();
                            for r in records {
                                if let Some(inner) = arg {
                                    items.push(evaluate(inner, r)?);
                                }
                            }
                            Value::List(items)
                        }
                        _ => Value::Null,
                    };
                    values.push(agg_val);
                } else {
                    values.push(evaluate(expr, first_record)?);
                }
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
        order_by: &[(Expression, SortDirection)],
    ) -> Result<ResultSet, ExecError> {
        let mut err: Option<ExecError> = None;
        input.records.sort_by(|a, b| {
            if err.is_some() {
                return Ordering::Equal;
            }
            for (expr, dir) in order_by {
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
                    return match dir {
                        SortDirection::Desc => ord.reverse(),
                        SortDirection::Asc => ord,
                    };
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
        // Find shared variable-prefixed columns (e.g. "a.name")
        // Skip bare columns (__node_id, __labels, name, age) as these are
        // coincidental overlaps between independent pattern scans.
        let shared: Vec<(usize, usize)> = left.columns.iter().enumerate()
            .filter_map(|(li, lc)| {
                // Only join on variable-prefixed columns (contain a dot)
                // and skip internal columns
                if !lc.contains('.') || lc.starts_with("__") {
                    return None;
                }
                right.columns.iter().position(|rc| rc == lc).map(|ri| (li, ri))
            })
            .collect();

        if shared.is_empty() {
            return self.exec_cross_join(left, right);
        }

        // Build output columns: all left + non-shared right
        let mut columns = left.columns.clone();
        let right_keep: Vec<usize> = (0..right.columns.len())
            .filter(|ri| !shared.iter().any(|(_, sri)| sri == ri))
            .collect();
        for &ri in &right_keep {
            columns.push(right.columns[ri].clone());
        }

        let mut rs = ResultSet::new(columns);

        // Build hash map from the right side
        let mut hash_map: std::collections::HashMap<Vec<u8>, Vec<usize>> =
            std::collections::HashMap::new();
        for (idx, record) in right.records.iter().enumerate() {
            let mut key = Vec::new();
            for &(_, ri) in &shared {
                key.extend_from_slice(format!("{:?}", record.values[ri]).as_bytes());
                key.push(0xFF);
            }
            hash_map.entry(key).or_default().push(idx);
        }

        // Probe with the left side
        for l_record in &left.records {
            let mut key = Vec::new();
            for &(li, _) in &shared {
                key.extend_from_slice(format!("{:?}", l_record.values[li]).as_bytes());
                key.push(0xFF);
            }
            if let Some(matches) = hash_map.get(&key) {
                for &r_idx in matches {
                    let r_record = &right.records[r_idx];
                    let mut values = l_record.values.clone();
                    for &ri in &right_keep {
                        values.push(r_record.values[ri].clone());
                    }
                    rs.add_record(values);
                }
            }
        }
        Ok(rs)
    }

    fn exec_cross_join(
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

    // -- DeleteNode ----------------------------------------------------------

    fn exec_delete_nodes(
        &self,
        rs: ResultSet,
        ctx: &ExecutionContext<'_>,
    ) -> Result<ResultSet, ExecError> {
        use std::collections::HashSet;
        let mut deleted = 0u64;
        let mut seen = HashSet::new();

        for record in &rs.records {
            for val in &record.values {
                if let Value::Node { ref id, .. } = val {
                    if seen.insert(id.clone()) {
                        let nid = NodeId(Uuid::parse_str(id)
                            .map_err(|e| ExecError::Internal(format!("invalid node id: {}", e)))?);
                        ctx.storage.delete_node(&ctx.graph_id, &nid)
                            .map_err(|e| ExecError::Internal(e.to_string()))?;
                        deleted += 1;
                    }
                }
            }
        }

        let mut result = ResultSet::new(vec!["deleted".to_string()]);
        result.add_record(vec![Value::Integer(deleted as i64)]);
        Ok(result)
    }

    // -- SetProperty ---------------------------------------------------------

    fn exec_set_property(
        &self,
        rs: ResultSet,
        target: Option<&str>,
        property: &str,
        value_expr: &Expression,
        ctx: &ExecutionContext<'_>,
    ) -> Result<ResultSet, ExecError> {
        let mut modified = 0u64;

        for record in &rs.records {
            let val = evaluate(value_expr, record)?;

            // If a target variable is specified, only modify that node
            if let Some(tgt) = target {
                if let Some(node_val) = record.get(tgt) {
                    if let Value::Node { ref id, .. } = node_val {
                        let nid = NodeId(Uuid::parse_str(id)
                            .map_err(|e| ExecError::Internal(format!("invalid node id: {}", e)))?);
                        let mut node = ctx.storage.get_node(&ctx.graph_id, &nid)
                            .map_err(|e| ExecError::Internal(e.to_string()))?;
                        node.properties.insert(property.to_string(), val.clone());
                        ctx.storage.put_node(&node)
                            .map_err(|e| ExecError::Internal(e.to_string()))?;
                        modified += 1;
                    }
                }
            } else {
                // Fallback: modify all nodes in the record
                for rv in &record.values {
                    if let Value::Node { ref id, .. } = rv {
                        let nid = NodeId(Uuid::parse_str(id)
                            .map_err(|e| ExecError::Internal(format!("invalid node id: {}", e)))?);
                        let mut node = ctx.storage.get_node(&ctx.graph_id, &nid)
                            .map_err(|e| ExecError::Internal(e.to_string()))?;
                        node.properties.insert(property.to_string(), val.clone());
                        ctx.storage.put_node(&node)
                            .map_err(|e| ExecError::Internal(e.to_string()))?;
                        modified += 1;
                    }
                }
            }
        }

        let mut result = ResultSet::new(vec!["modified".to_string()]);
        result.add_record(vec![Value::Integer(modified as i64)]);
        Ok(result)
    }

    // -- Distinct ------------------------------------------------------------

    fn exec_distinct(&self, rs: ResultSet) -> Result<ResultSet, ExecError> {
        use std::collections::HashSet;
        let mut result = ResultSet::new(rs.columns.clone());
        let mut seen = HashSet::new();

        // Only consider non-internal columns for uniqueness
        let user_col_indices: Vec<usize> = rs.columns.iter().enumerate()
            .filter(|(_, c)| !c.starts_with("__"))
            .map(|(i, _)| i)
            .collect();

        for record in &rs.records {
            let key: Vec<_> = user_col_indices.iter()
                .map(|&i| format!("{:?}", record.values.get(i).unwrap_or(&Value::Null)))
                .collect();
            let key_str = key.join("|");
            if seen.insert(key_str) {
                result.add_record(record.values.clone());
            }
        }

        Ok(result)
    }

    // -- CallProcedure ------------------------------------------------------

    fn exec_call_procedure(
        &self,
        procedure: &str,
        yield_items: &Option<Vec<String>>,
        ctx: &ExecutionContext<'_>,
    ) -> Result<ResultSet, ExecError> {
        let result = match procedure {
            "db.labels" => {
                let nodes = ctx.storage.scan_nodes(&ctx.graph_id)?;
                let mut labels = std::collections::BTreeSet::new();
                for node in &nodes {
                    for label in &node.labels {
                        labels.insert(label.0.clone());
                    }
                }
                let mut rs = ResultSet::new(vec!["label".to_string()]);
                for label in labels {
                    rs.add_record(vec![Value::String(label)]);
                }
                rs
            }
            "db.relationshipTypes" | "db.edgeTypes" => {
                let nodes = ctx.storage.scan_nodes(&ctx.graph_id)?;
                let mut edge_types = std::collections::BTreeSet::new();
                for node in &nodes {
                    let node_edges = ctx.storage.get_outgoing_edges(&ctx.graph_id, &node.id, None)?;
                    for edge in &node_edges {
                        edge_types.insert(edge.label.0.clone());
                    }
                }
                let mut rs = ResultSet::new(vec!["relationshipType".to_string()]);
                for t in edge_types {
                    rs.add_record(vec![Value::String(t)]);
                }
                rs
            }
            "db.propertyKeys" => {
                let nodes = ctx.storage.scan_nodes(&ctx.graph_id)?;
                let mut keys = std::collections::BTreeSet::new();
                for node in &nodes {
                    for key in node.properties.keys() {
                        keys.insert(key.clone());
                    }
                    let node_edges = ctx.storage.get_outgoing_edges(&ctx.graph_id, &node.id, None)?;
                    for edge in &node_edges {
                        for key in edge.properties.keys() {
                            keys.insert(key.clone());
                        }
                    }
                }
                let mut rs = ResultSet::new(vec!["propertyKey".to_string()]);
                for key in keys {
                    rs.add_record(vec![Value::String(key)]);
                }
                rs
            }
            "db.schema" => {
                let nodes = ctx.storage.scan_nodes(&ctx.graph_id)?;
                let mut labels = std::collections::BTreeSet::new();
                let mut edge_types = std::collections::BTreeSet::new();
                let mut prop_keys = std::collections::BTreeSet::new();
                for node in &nodes {
                    for label in &node.labels {
                        labels.insert(label.0.clone());
                    }
                    for key in node.properties.keys() {
                        prop_keys.insert(key.clone());
                    }
                    let node_edges = ctx.storage.get_outgoing_edges(&ctx.graph_id, &node.id, None)?;
                    for edge in &node_edges {
                        edge_types.insert(edge.label.0.clone());
                        for key in edge.properties.keys() {
                            prop_keys.insert(key.clone());
                        }
                    }
                }
                let mut rs = ResultSet::new(vec![
                    "labels".to_string(),
                    "relationshipTypes".to_string(),
                    "propertyKeys".to_string(),
                ]);
                let label_list = Value::List(labels.into_iter().map(Value::String).collect());
                let edge_list = Value::List(edge_types.into_iter().map(Value::String).collect());
                let prop_list = Value::List(prop_keys.into_iter().map(Value::String).collect());
                rs.add_record(vec![label_list, edge_list, prop_list]);
                rs
            }
            "db.nodeCount" => {
                let nodes = ctx.storage.scan_nodes(&ctx.graph_id)?;
                let mut rs = ResultSet::new(vec!["count".to_string()]);
                rs.add_record(vec![Value::Integer(nodes.len() as i64)]);
                rs
            }
            "db.edgeCount" => {
                let nodes = ctx.storage.scan_nodes(&ctx.graph_id)?;
                let mut count: i64 = 0;
                for node in &nodes {
                    let edges = ctx.storage.get_outgoing_edges(&ctx.graph_id, &node.id, None)?;
                    count += edges.len() as i64;
                }
                let mut rs = ResultSet::new(vec!["count".to_string()]);
                rs.add_record(vec![Value::Integer(count)]);
                rs
            }
            _ => {
                return Err(ExecError::NotImplemented(format!("procedure: {}", procedure)));
            }
        };

        // If yield_items is specified, filter columns to only include those
        if let Some(items) = yield_items {
            let mut filtered_rs = ResultSet::new(items.clone());
            let col_indices: Vec<Option<usize>> = items.iter()
                .map(|name| result.column_index(name))
                .collect();
            for record in &result.records {
                let values: Vec<Value> = col_indices.iter()
                    .map(|idx| match idx {
                        Some(i) => record.values[*i].clone(),
                        None => Value::Null,
                    })
                    .collect();
                filtered_rs.add_record(values);
            }
            Ok(filtered_rs)
        } else {
            Ok(result)
        }
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