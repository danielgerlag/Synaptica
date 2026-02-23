//! Semantic analysis for GQL programs.
//!
//! Walks the AST to verify variable scoping: variables defined in MATCH patterns
//! must be referenced correctly in WHERE / RETURN clauses.

use std::collections::HashSet;

use crate::ast::{
    Expression, GqlProgram, GqlStatement, MatchStatement, PatternElement, ReturnItem,
    ReturnStatement,
};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors detected during semantic analysis.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SemanticError {
    #[error("undefined variable `{0}`")]
    UndefinedVariable(String),
    #[error("duplicate variable `{0}`")]
    DuplicateVariable(String),
    #[error("type mismatch: expected `{expected}`, found `{actual}`")]
    TypeMismatch { expected: String, actual: String },
    #[error("invalid label reference `{0}`")]
    InvalidLabelReference(String),
    #[error("invalid property reference `{0}`")]
    InvalidPropertyReference(String),
}

// ---------------------------------------------------------------------------
// Analyzer
// ---------------------------------------------------------------------------

/// Basic semantic analyzer that tracks variable scope across a GQL program.
pub struct SemanticAnalyzer {
    /// Variables currently in scope.
    scope: HashSet<String>,
}

impl SemanticAnalyzer {
    pub fn new() -> Self {
        Self {
            scope: HashSet::new(),
        }
    }

    /// Analyse a complete GQL program.
    pub fn analyze(&mut self, program: &GqlProgram) -> Result<(), SemanticError> {
        for stmt in &program.statements {
            self.analyze_statement(stmt)?;
        }
        Ok(())
    }

    fn analyze_statement(&mut self, stmt: &GqlStatement) -> Result<(), SemanticError> {
        match stmt {
            GqlStatement::Match(m) => self.analyze_match(m),
            GqlStatement::Return(r) => self.analyze_return(r),
            // Other statement types are not yet covered by the semantic pass.
            _ => Ok(()),
        }
    }

    /// Collect variables introduced by MATCH and validate WHERE references.
    fn analyze_match(&mut self, m: &MatchStatement) -> Result<(), SemanticError> {
        // Collect variables defined by pattern elements.
        for path in &m.pattern.paths {
            for elem in &path.elements {
                match elem {
                    PatternElement::Node(n) => {
                        if let Some(ref var) = n.variable {
                            if !self.scope.insert(var.clone()) {
                                return Err(SemanticError::DuplicateVariable(var.clone()));
                            }
                        }
                    }
                    PatternElement::Edge(e) => {
                        if let Some(ref var) = e.variable {
                            if !self.scope.insert(var.clone()) {
                                return Err(SemanticError::DuplicateVariable(var.clone()));
                            }
                        }
                    }
                }
            }
        }

        // Validate WHERE clause references.
        if let Some(ref wc) = m.where_clause {
            self.check_expression(&wc.condition)?;
        }

        Ok(())
    }

    /// Validate that RETURN items only reference variables that are in scope.
    fn analyze_return(&self, r: &ReturnStatement) -> Result<(), SemanticError> {
        for item in &r.items {
            self.check_return_item(item)?;
        }
        Ok(())
    }

    fn check_return_item(&self, item: &ReturnItem) -> Result<(), SemanticError> {
        self.check_expression(&item.expression)
    }

    /// Recursively verify that every identifier in `expr` is in scope.
    fn check_expression(&self, expr: &Expression) -> Result<(), SemanticError> {
        match expr {
            Expression::Identifier(name) => {
                if !self.scope.contains(name) {
                    return Err(SemanticError::UndefinedVariable(name.clone()));
                }
            }
            Expression::PropertyAccess { object, .. } => {
                self.check_expression(object)?;
            }
            Expression::BinaryOp { left, right, .. } => {
                self.check_expression(left)?;
                self.check_expression(right)?;
            }
            Expression::UnaryOp { operand, .. } => {
                self.check_expression(operand)?;
            }
            Expression::FunctionCall { args, .. } => {
                for arg in args {
                    self.check_expression(arg)?;
                }
            }
            Expression::Aggregate { arg, .. } => {
                if let Some(ref a) = arg {
                    self.check_expression(a)?;
                }
            }
            Expression::IsNull(inner) | Expression::IsNotNull(inner) => {
                self.check_expression(inner)?;
            }
            Expression::In { operand, list } => {
                self.check_expression(operand)?;
                self.check_expression(list)?;
            }
            Expression::Like { operand, pattern } => {
                self.check_expression(operand)?;
                self.check_expression(pattern)?;
            }
            Expression::List(items) => {
                for item in items {
                    self.check_expression(item)?;
                }
            }
            Expression::Map(entries) => {
                for (_, val) in entries {
                    self.check_expression(val)?;
                }
            }
            // Literals, parameters, etc. need no scope check.
            _ => {}
        }
        Ok(())
    }
}

impl Default for SemanticAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;
    use crate::parser::parse;

    #[test]
    fn undefined_variable_in_return() {
        let program = GqlProgram {
            statements: vec![GqlStatement::Return(ReturnStatement {
                distinct: false,
                items: vec![ReturnItem {
                    expression: Expression::Identifier("x".into()),
                    alias: None,
                }],
                group_by: None,
                having: None,
                order_by: None,
                limit_offset: None,
            })],
        };

        let mut analyzer = SemanticAnalyzer::new();
        let err = analyzer.analyze(&program).unwrap_err();
        assert_eq!(err, SemanticError::UndefinedVariable("x".into()));
    }

    /// Helper: parse GQL source then run semantic analysis.
    fn analyze_gql(src: &str) -> Result<(), SemanticError> {
        let program = parse(src).expect("parse should succeed");
        let mut analyzer = SemanticAnalyzer::new();
        analyzer.analyze(&program)
    }

    #[test]
    fn test_valid_simple_query() {
        assert!(analyze_gql("MATCH (n:Person) RETURN n.name").is_ok());
    }

    #[test]
    fn test_valid_edge_pattern() {
        assert!(analyze_gql("MATCH (a)-[r:KNOWS]->(b) RETURN a.name, b.name").is_ok());
    }

    #[test]
    fn test_undefined_var_in_where() {
        let result = analyze_gql("MATCH (n:Person) WHERE x.age > 30 RETURN n");
        assert_eq!(
            result.unwrap_err(),
            SemanticError::UndefinedVariable("x".into())
        );
    }

    #[test]
    fn test_undefined_var_in_return_parsed() {
        let result = analyze_gql("MATCH (n:Person) RETURN m.name");
        assert_eq!(
            result.unwrap_err(),
            SemanticError::UndefinedVariable("m".into())
        );
    }

    #[test]
    fn test_multiple_variables_valid() {
        assert!(
            analyze_gql("MATCH (a:Person)-[r:KNOWS]->(b:Person) RETURN a.name, r, b.name")
                .is_ok()
        );
    }

    #[test]
    fn test_variable_from_edge_pattern() {
        assert!(analyze_gql("MATCH (n)-[r]->(m) RETURN r").is_ok());
    }

    #[test]
    fn test_no_match_with_return() {
        // RETURN 42 — literal only, no variable references, should pass.
        assert!(analyze_gql("RETURN 42").is_ok());
    }

    #[test]
    fn test_match_without_variable() {
        // Unnamed node pattern is valid; returning a literal needs no scope.
        assert!(analyze_gql("MATCH (:Person) RETURN 1").is_ok());
    }

    #[test]
    fn test_insert_no_undefined_vars() {
        // INSERT is not checked by the semantic pass (handled as `_ => Ok(())`).
        assert!(analyze_gql("INSERT (:Person {name: 'Alice'})").is_ok());
    }

    #[test]
    fn test_duplicate_variable_detection() {
        let result = analyze_gql("MATCH (n:Person), (n:Company) RETURN n");
        assert_eq!(
            result.unwrap_err(),
            SemanticError::DuplicateVariable("n".into())
        );
    }
}