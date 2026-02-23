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
    /// An empty result set (identity for unions, etc.).
    Empty,
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
                // Build a Scan from the first path pattern's first node labels.
                let labels: Vec<String> = m
                    .pattern
                    .paths
                    .iter()
                    .flat_map(|p| p.elements.iter())
                    .filter_map(|e| match e {
                        crate::ast::PatternElement::Node(n) => Some(n.labels.clone()),
                        _ => None,
                    })
                    .flatten()
                    .collect();

                let mut plan = LogicalPlan::Scan {
                    labels,
                    graph_id: m.graph.clone(),
                };

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