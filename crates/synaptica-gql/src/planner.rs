//! Logical query plan types and query planner stub.
//!
//! Translates a parsed GQL AST into a tree of logical operators that can later
//! be optimised and executed.

use crate::ast::{Direction, Expression, GqlProgram};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PlanError {
    #[error("unsupported statement kind")]
    UnsupportedStatement,
    #[error("planning error: {0}")]
    Internal(String),
}

// ---------------------------------------------------------------------------
// Logical plan
// ---------------------------------------------------------------------------

/// A logical query plan node.
#[derive(Debug, Clone, PartialEq)]
pub enum LogicalPlan {
    /// Scan for nodes matching `labels` within `graph_id`.
    Scan {
        labels: Vec<String>,
        graph_id: Option<String>,
        variable: Option<String>,
    },
    /// Apply a predicate filter.
    Filter {
        input: Box<LogicalPlan>,
        predicate: Expression,
    },
    /// Project a set of expressions (column selection / computation).
    Project {
        input: Box<LogicalPlan>,
        expressions: Vec<Expression>,
    },
    /// Traverse an edge from the current node set.
    Expand {
        input: Box<LogicalPlan>,
        edge_label: String,
        direction: Direction,
        target_labels: Vec<String>,
        edge_variable: Option<String>,
        target_variable: Option<String>,
    },
    /// Join two plan branches.
    Join {
        left: Box<LogicalPlan>,
        right: Box<LogicalPlan>,
    },
    /// Aggregate with optional grouping.
    Aggregate {
        input: Box<LogicalPlan>,
        group_by: Vec<Expression>,
        aggregates: Vec<Expression>,
    },
    /// Sort the result set.
    Sort {
        input: Box<LogicalPlan>,
        order_by: Vec<Expression>,
    },
    /// Limit (and optionally skip) rows.
    Limit {
        input: Box<LogicalPlan>,
        count: Option<u64>,
        offset: Option<u64>,
    },
    /// Remove duplicate rows.
    Distinct {
        input: Box<LogicalPlan>,
    },
    /// Create a new node.
    CreateNode {
        labels: Vec<String>,
        properties: Vec<(String, Expression)>,
    },
    /// Create a new edge between source and target.
    CreateEdge {
        label: String,
        source: Box<LogicalPlan>,
        target: Box<LogicalPlan>,
        properties: Vec<(String, Expression)>,
    },
    /// Delete nodes produced by `input`.
    DeleteNode {
        input: Box<LogicalPlan>,
    },
    /// Set a property on nodes/edges produced by `input`.
    SetProperty {
        input: Box<LogicalPlan>,
        property: String,
        value: Expression,
    },
    /// Set-union of two plans.
    Union {
        left: Box<LogicalPlan>,
        right: Box<LogicalPlan>,
        all: bool,
    },
    /// Call a built-in procedure.
    CallProcedure {
        procedure: String,
        arguments: Vec<Expression>,
        yield_items: Option<Vec<String>>,
    },
    /// An empty result set (identity for unions, etc.).
    Empty,
    /// Create edges from MATCH results, referencing bound variables.
    CreateEdgeFromMatch {
        input: Box<LogicalPlan>,
        source_var: String,
        target_var: String,
        label: String,
        properties: Vec<(String, Expression)>,
    },
}

// ---------------------------------------------------------------------------
// Planner
// ---------------------------------------------------------------------------

/// Translates a [`GqlProgram`] into a [`LogicalPlan`].
pub struct QueryPlanner;

impl QueryPlanner {
    pub fn new() -> Self {
        Self
    }

    /// Plan a complete GQL program.
    ///
    /// Currently handles a limited subset of statement sequences; this will be
    /// extended as the planner matures.
    pub fn plan(&self, program: &GqlProgram) -> Result<LogicalPlan, PlanError> {
        if program.statements.is_empty() {
            return Ok(LogicalPlan::Empty);
        }

        let mut current: Option<LogicalPlan> = None;

        for stmt in &program.statements {
            current = Some(self.plan_statement(stmt, current)?);
        }

        current.ok_or(PlanError::Internal("empty plan".into()))
    }

    fn plan_statement(
        &self,
        stmt: &crate::ast::GqlStatement,
        input: Option<LogicalPlan>,
    ) -> Result<LogicalPlan, PlanError> {
        use crate::ast::GqlStatement;

        match stmt {
            GqlStatement::Match(m) => {
                // Build one plan per path pattern and combine with Join.
                let mut scans: Vec<LogicalPlan> = Vec::new();

                for path in &m.pattern.paths {
                    // Check if path has edges (traversal pattern)
                    let has_edges = path.elements.iter().any(|e| matches!(e, crate::ast::PatternElement::Edge(_)));

                    if has_edges {
                        // Extract node/edge elements in order
                        let nodes: Vec<&crate::ast::NodePattern> = path.elements.iter()
                            .filter_map(|e| match e {
                                crate::ast::PatternElement::Node(n) => Some(n),
                                _ => None,
                            })
                            .collect();
                        let edges: Vec<&crate::ast::EdgePattern> = path.elements.iter()
                            .filter_map(|e| match e {
                                crate::ast::PatternElement::Edge(e) => Some(e),
                                _ => None,
                            })
                            .collect();

                        let source_node = nodes.first().ok_or_else(|| PlanError::Internal("edge pattern missing source node".into()))?;
                        let edge_pat = edges.first().ok_or_else(|| PlanError::Internal("edge pattern missing edge".into()))?;
                        let target_node = nodes.get(1);

                        let source_scan = LogicalPlan::Scan {
                            labels: source_node.labels.clone(),
                            graph_id: m.graph.clone(),
                            variable: source_node.variable.clone(),
                        };

                        scans.push(LogicalPlan::Expand {
                            input: Box::new(source_scan),
                            edge_label: edge_pat.labels.first().cloned().unwrap_or_default(),
                            direction: edge_pat.direction.clone(),
                            target_labels: target_node.map(|n| n.labels.clone()).unwrap_or_default(),
                            edge_variable: edge_pat.variable.clone(),
                            target_variable: target_node.and_then(|n| n.variable.clone()),
                        });
                    } else {
                        let labels: Vec<String> = path.elements.iter()
                            .filter_map(|e| match e {
                                crate::ast::PatternElement::Node(n) => Some(n.labels.clone()),
                                _ => None,
                            })
                            .flatten()
                            .collect();

                        let variable = path.elements.iter()
                            .filter_map(|e| match e {
                                crate::ast::PatternElement::Node(n) => n.variable.clone(),
                                _ => None,
                            })
                            .next();

                        scans.push(LogicalPlan::Scan {
                            labels,
                            graph_id: m.graph.clone(),
                            variable,
                        });
                    }
                }

                let mut plan = scans.remove(0);
                for scan in scans {
                    plan = LogicalPlan::Join {
                        left: Box::new(plan),
                        right: Box::new(scan),
                    };
                }

                if let Some(ref wc) = m.where_clause {
                    plan = LogicalPlan::Filter {
                        input: Box::new(plan),
                        predicate: *wc.condition.clone(),
                    };
                }

                Ok(plan)
            }
            GqlStatement::Return(r) => {
                let base = input.unwrap_or(LogicalPlan::Empty);
                let exprs: Vec<Expression> =
                    r.items.iter().map(|i| i.expression.clone()).collect();

                let mut plan = LogicalPlan::Project {
                    input: Box::new(base),
                    expressions: exprs,
                };

                if r.distinct {
                    plan = LogicalPlan::Distinct {
                        input: Box::new(plan),
                    };
                }

                if let Some(ref ob) = r.order_by {
                    let order_exprs: Vec<Expression> =
                        ob.items.iter().map(|i| i.expression.clone()).collect();
                    plan = LogicalPlan::Sort {
                        input: Box::new(plan),
                        order_by: order_exprs,
                    };
                }

                if let Some(ref lo) = r.limit_offset {
                    let count = lo.limit.as_ref().and_then(|e| match e {
                        Expression::Literal(crate::ast::Literal::Integer(n)) => Some(*n as u64),
                        _ => None,
                    });
                    let offset = lo.offset.as_ref().and_then(|e| match e {
                        Expression::Literal(crate::ast::Literal::Integer(n)) => Some(*n as u64),
                        _ => None,
                    });
                    plan = LogicalPlan::Limit {
                        input: Box::new(plan),
                        count,
                        offset,
                    };
                }

                Ok(plan)
            }
            GqlStatement::Insert(ins) => {
                let has_edges = ins.patterns.iter().any(|p|
                    p.elements.iter().any(|e| matches!(e, crate::ast::PatternElement::Edge(_)))
                );

                if has_edges && input.is_some() {
                    let base = input.unwrap();
                    let elements: Vec<_> = ins.patterns.iter()
                        .flat_map(|p| p.elements.iter())
                        .collect();

                    let source_var = elements.iter().find_map(|e| match e {
                        crate::ast::PatternElement::Node(n) => n.variable.clone(),
                        _ => None,
                    }).ok_or(PlanError::Internal("edge INSERT requires source node variable".into()))?;

                    let edge = elements.iter().find_map(|e| match e {
                        crate::ast::PatternElement::Edge(ep) => Some(ep),
                        _ => None,
                    }).ok_or(PlanError::Internal("edge INSERT requires edge pattern".into()))?;

                    let target_var = elements.iter().filter_map(|e| match e {
                        crate::ast::PatternElement::Node(n) => n.variable.clone(),
                        _ => None,
                    }).nth(1).ok_or(PlanError::Internal("edge INSERT requires target node variable".into()))?;

                    let label = edge.labels.first().cloned()
                        .ok_or(PlanError::Internal("edge INSERT requires a label".into()))?;

                    Ok(LogicalPlan::CreateEdgeFromMatch {
                        input: Box::new(base),
                        source_var,
                        target_var,
                        label,
                        properties: edge.properties.clone(),
                    })
                } else {
                    let node = ins
                        .patterns
                        .iter()
                        .flat_map(|p| p.elements.iter())
                        .filter_map(|e| match e {
                            crate::ast::PatternElement::Node(n) => Some(n),
                            _ => None,
                        })
                        .next()
                        .ok_or(PlanError::Internal(
                            "INSERT requires at least one node pattern".into(),
                        ))?;
                    Ok(LogicalPlan::CreateNode {
                        labels: node.labels.clone(),
                        properties: node.properties.clone(),
                    })
                }
            }
            GqlStatement::With(w) => {
                let base = input.unwrap_or(LogicalPlan::Empty);
                let exprs: Vec<Expression> =
                    w.items.iter().map(|i| i.expression.clone()).collect();

                let mut plan = LogicalPlan::Project {
                    input: Box::new(base),
                    expressions: exprs,
                };

                if w.distinct {
                    plan = LogicalPlan::Distinct {
                        input: Box::new(plan),
                    };
                }

                if let Some(ref wc) = w.where_clause {
                    plan = LogicalPlan::Filter {
                        input: Box::new(plan),
                        predicate: *wc.condition.clone(),
                    };
                }

                if let Some(ref ob) = w.order_by {
                    let order_exprs: Vec<Expression> =
                        ob.items.iter().map(|i| i.expression.clone()).collect();
                    plan = LogicalPlan::Sort {
                        input: Box::new(plan),
                        order_by: order_exprs,
                    };
                }

                if let Some(ref lo) = w.limit_offset {
                    let count = lo.limit.as_ref().and_then(|e| match e {
                        Expression::Literal(crate::ast::Literal::Integer(n)) => Some(*n as u64),
                        _ => None,
                    });
                    let offset = lo.offset.as_ref().and_then(|e| match e {
                        Expression::Literal(crate::ast::Literal::Integer(n)) => Some(*n as u64),
                        _ => None,
                    });
                    plan = LogicalPlan::Limit {
                        input: Box::new(plan),
                        count,
                        offset,
                    };
                }

                Ok(plan)
            }
            GqlStatement::Call(c) => {
                Ok(LogicalPlan::CallProcedure {
                    procedure: c.procedure.clone(),
                    arguments: c.arguments.clone(),
                    yield_items: c.yield_items.clone(),
                })
            }
            _ => Err(PlanError::UnsupportedStatement),
        }
    }
}

impl Default for QueryPlanner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;

    #[test]
    fn empty_program_produces_empty_plan() {
        let planner = QueryPlanner::new();
        let plan = planner
            .plan(&GqlProgram {
                statements: vec![],
            })
            .unwrap();
        assert_eq!(plan, LogicalPlan::Empty);
    }
}