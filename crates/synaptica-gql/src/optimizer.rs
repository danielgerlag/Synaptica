//! Logical plan optimiser with a rule-based framework.
//!
//! Rules are applied in order to a [`LogicalPlan`]; each rule can rewrite the
//! plan tree or leave it unchanged.

use crate::planner::LogicalPlan;

// ---------------------------------------------------------------------------
// Rule trait
// ---------------------------------------------------------------------------

/// A single optimisation rule.
///
/// Implementations inspect (and optionally rewrite) a [`LogicalPlan`] node.
/// Returning `Some(plan)` indicates the rule fired and replaced the node;
/// `None` means the node was left unchanged.
pub trait OptimizerRule: std::fmt::Debug {
    fn name(&self) -> &str;
    fn apply(&self, plan: &LogicalPlan) -> Option<LogicalPlan>;
}

// ---------------------------------------------------------------------------
// Built-in rules
// ---------------------------------------------------------------------------

/// Push `Filter` nodes closer to `Scan` nodes when possible.
#[derive(Debug, Default)]
pub struct FilterPushdown;

impl OptimizerRule for FilterPushdown {
    fn name(&self) -> &str {
        "FilterPushdown"
    }

    fn apply(&self, plan: &LogicalPlan) -> Option<LogicalPlan> {
        // Stub: look for Filter-over-Project and swap them so the filter is
        // applied before the projection.
        if let LogicalPlan::Filter {
            input,
            predicate,
        } = plan
        {
            if let LogicalPlan::Project {
                input: proj_input,
                expressions,
            } = input.as_ref()
            {
                return Some(LogicalPlan::Project {
                    input: Box::new(LogicalPlan::Filter {
                        input: proj_input.clone(),
                        predicate: predicate.clone(),
                    }),
                    expressions: expressions.clone(),
                });
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Optimizer
// ---------------------------------------------------------------------------

/// Applies a sequence of [`OptimizerRule`]s to a logical plan.
#[derive(Debug)]
pub struct QueryOptimizer {
    rules: Vec<Box<dyn OptimizerRule>>,
}

impl QueryOptimizer {
    /// Create an optimizer with the default rule set.
    pub fn new() -> Self {
        Self {
            rules: vec![Box::new(FilterPushdown)],
        }
    }

    /// Create an optimizer with a custom list of rules.
    pub fn with_rules(rules: Vec<Box<dyn OptimizerRule>>) -> Self {
        Self { rules }
    }

    /// Optimise the given plan by applying every rule once (top-down).
    pub fn optimize(&self, plan: LogicalPlan) -> LogicalPlan {
        let mut current = plan;
        for rule in &self.rules {
            if let Some(rewritten) = rule.apply(&current) {
                current = rewritten;
            }
        }
        current
    }
}

impl Default for QueryOptimizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Expression;
    use crate::planner::LogicalPlan;

    #[test]
    fn filter_pushdown_swaps_filter_and_project() {
        let plan = LogicalPlan::Filter {
            input: Box::new(LogicalPlan::Project {
                input: Box::new(LogicalPlan::Scan {
                    labels: vec!["Person".into()],
                    graph_id: None,
                }),
                expressions: vec![Expression::Identifier("n".into())],
            }),
            predicate: Expression::Identifier("pred".into()),
        };

        let opt = QueryOptimizer::new();
        let result = opt.optimize(plan);

        // After pushdown the Project should wrap a Filter.
        assert!(matches!(result, LogicalPlan::Project { .. }));
    }
}