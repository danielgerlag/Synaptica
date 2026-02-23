//! GQL recursive descent parser.
//!
//! Parses a stream of [`SpannedToken`]s produced by the lexer into the AST
//! defined in [`crate::ast`].

use std::fmt;

use crate::ast::*;
use crate::lexer::{Lexer, Span, SpannedToken, Token};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// An error produced during parsing.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Parse error at {}:{}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for ParseError {}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

pub struct Parser {
    tokens: Vec<SpannedToken>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<SpannedToken>) -> Parser {
        let tokens: Vec<SpannedToken> = tokens
            .into_iter()
            .filter(|st| !matches!(st.token, Token::Comment(_)))
            .collect();
        Parser { tokens, pos: 0 }
    }

    pub fn parse(&mut self) -> Result<GqlProgram, ParseError> {
        let mut statements = Vec::new();
        while !self.at_end() {
            let stmt = self.parse_statement()?;
            statements.push(stmt);
            self.match_token(&Token::Semicolon);
        }
        Ok(GqlProgram { statements })
    }

    // -- helpers ---------------------------------------------------------

    fn peek(&self) -> &Token {
        self.tokens
            .get(self.pos)
            .map_or(&Token::Eof, |st| &st.token)
    }

    fn peek_span(&self) -> Span {
        self.tokens.get(self.pos).map_or(
            Span {
                line: 0,
                column: 0,
                offset: 0,
            },
            |st| st.span,
        )
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset).map(|st| &st.token)
    }

    fn advance(&mut self) {
        if !self.at_end() {
            self.pos += 1;
        }
    }

    fn at_end(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }

    fn match_token(&mut self, expected: &Token) -> bool {
        if self.peek() == expected {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, expected: &Token) -> Result<(), ParseError> {
        if self.peek() == expected {
            self.advance();
            Ok(())
        } else {
            Err(self.error(format!("expected {expected:?}, found {:?}", self.peek())))
        }
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        let span = self.peek_span();
        ParseError {
            message: message.into(),
            line: span.line,
            column: span.column,
        }
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        if let Token::Ident(name) = self.peek().clone() {
            self.advance();
            Ok(name)
        } else {
            Err(self.error(format!(
                "expected identifier, found {:?}",
                self.peek()
            )))
        }
    }

    // -- statement parsing -----------------------------------------------

    fn parse_statement(&mut self) -> Result<GqlStatement, ParseError> {
        match self.peek() {
            Token::Match => self.parse_match_statement(false),
            Token::Optional => {
                self.advance();
                self.parse_match_statement(true)
            }
            Token::Return => self.parse_return_statement(),
            Token::Insert => self.parse_insert_statement(),
            Token::Set => self.parse_set_statement(),
            Token::Delete => self.parse_delete_statement(),
            Token::Detach => self.parse_delete_statement(),
            Token::Remove => self.parse_remove_statement(),
            Token::Create => self.parse_create_statement(),
            Token::Drop => self.parse_drop_graph_statement(),
            _ => Err(self.error(format!("unexpected token {:?}", self.peek()))),
        }
    }

    fn parse_match_statement(&mut self, optional: bool) -> Result<GqlStatement, ParseError> {
        self.expect(&Token::Match)?;
        let pattern = self.parse_graph_pattern()?;

        let where_clause = if self.peek() == &Token::Where {
            Some(self.parse_where_clause()?)
        } else {
            None
        };

        let order_by = if self.peek() == &Token::Order {
            Some(self.parse_order_by_clause()?)
        } else {
            None
        };

        let limit_offset = self.parse_limit_offset()?;

        let stmt = GqlStatement::Match(MatchStatement {
            graph: None,
            optional,
            pattern,
            where_clause,
            order_by,
            limit_offset,
        });

        self.maybe_parse_composite(stmt)
    }

    fn parse_return_statement(&mut self) -> Result<GqlStatement, ParseError> {
        self.expect(&Token::Return)?;
        let distinct = self.match_token(&Token::Distinct);
        let items = self.parse_return_items()?;

        let group_by = if self.peek() == &Token::Group {
            Some(self.parse_group_by_clause()?)
        } else {
            None
        };

        let having = if self.peek() == &Token::Having {
            self.advance();
            let condition = self.parse_expression()?;
            Some(HavingClause { condition })
        } else {
            None
        };

        let order_by = if self.peek() == &Token::Order {
            Some(self.parse_order_by_clause()?)
        } else {
            None
        };

        let limit_offset = self.parse_limit_offset()?;

        let stmt = GqlStatement::Return(ReturnStatement {
            distinct,
            items,
            group_by,
            having,
            order_by,
            limit_offset,
        });

        self.maybe_parse_composite(stmt)
    }

    fn parse_insert_statement(&mut self) -> Result<GqlStatement, ParseError> {
        self.expect(&Token::Insert)?;
        let mut patterns = vec![self.parse_path_pattern()?];
        while self.match_token(&Token::Comma) {
            patterns.push(self.parse_path_pattern()?);
        }
        Ok(GqlStatement::Insert(InsertStatement { patterns }))
    }

    fn parse_set_statement(&mut self) -> Result<GqlStatement, ParseError> {
        self.expect(&Token::Set)?;
        let mut items = vec![self.parse_set_item()?];
        while self.match_token(&Token::Comma) {
            items.push(self.parse_set_item()?);
        }
        Ok(GqlStatement::Set(SetStatement { items }))
    }

    fn parse_set_item(&mut self) -> Result<SetItem, ParseError> {
        let target = self.parse_target_expr()?;

        if self.match_token(&Token::Colon) {
            let label = self.expect_ident()?;
            Ok(SetItem::Label { target, label })
        } else if self.match_token(&Token::Eq) {
            let value = self.parse_expression()?;
            match target {
                Expression::PropertyAccess { object, property } => {
                    Ok(SetItem::Property {
                        target: *object,
                        property,
                        value,
                    })
                }
                _ => Ok(SetItem::AllProperties { target, value }),
            }
        } else {
            Err(self.error("expected '=' or ':' in SET clause"))
        }
    }

    fn parse_delete_statement(&mut self) -> Result<GqlStatement, ParseError> {
        let detach = self.match_token(&Token::Detach);
        self.expect(&Token::Delete)?;
        let mut targets = vec![self.parse_expression()?];
        while self.match_token(&Token::Comma) {
            targets.push(self.parse_expression()?);
        }
        Ok(GqlStatement::Delete(DeleteStatement { detach, targets }))
    }

    fn parse_remove_statement(&mut self) -> Result<GqlStatement, ParseError> {
        self.expect(&Token::Remove)?;
        let mut items = vec![self.parse_remove_item()?];
        while self.match_token(&Token::Comma) {
            items.push(self.parse_remove_item()?);
        }
        Ok(GqlStatement::Remove(RemoveStatement { items }))
    }

    fn parse_remove_item(&mut self) -> Result<RemoveItem, ParseError> {
        let target = self.parse_target_expr()?;
        if self.match_token(&Token::Colon) {
            let label = self.expect_ident()?;
            Ok(RemoveItem::Label { target, label })
        } else {
            match target {
                Expression::PropertyAccess { object, property } => {
                    Ok(RemoveItem::Property {
                        target: *object,
                        property,
                    })
                }
                _ => Err(self.error("expected property access or label in REMOVE")),
            }
        }
    }

    fn parse_create_statement(&mut self) -> Result<GqlStatement, ParseError> {
        self.expect(&Token::Create)?;
        self.expect(&Token::Graph)?;
        if self.peek() == &Token::Type {
            self.advance();
            self.parse_create_graph_type_rest()
        } else {
            self.parse_create_graph_rest()
        }
    }

    fn parse_create_graph_rest(&mut self) -> Result<GqlStatement, ParseError> {
        let if_not_exists = self.parse_if_not_exists()?;
        let name = self.expect_ident()?;
        let graph_type = if self.peek() == &Token::LBrace {
            Some(self.parse_graph_type_def()?)
        } else {
            None
        };
        Ok(GqlStatement::CreateGraph(CreateGraphStatement {
            name,
            if_not_exists,
            graph_type,
        }))
    }

    fn parse_create_graph_type_rest(&mut self) -> Result<GqlStatement, ParseError> {
        let if_not_exists = self.parse_if_not_exists()?;
        let name = self.expect_ident()?;
        let graph_type = self.parse_graph_type_def()?;
        Ok(GqlStatement::CreateGraphType(CreateGraphTypeStatement {
            name,
            if_not_exists,
            graph_type,
        }))
    }

    fn parse_drop_graph_statement(&mut self) -> Result<GqlStatement, ParseError> {
        self.expect(&Token::Drop)?;
        self.expect(&Token::Graph)?;
        let if_exists = self.parse_if_exists()?;
        let name = self.expect_ident()?;
        Ok(GqlStatement::DropGraph(DropGraphStatement {
            name,
            if_exists,
        }))
    }

    fn parse_if_not_exists(&mut self) -> Result<bool, ParseError> {
        if self.peek() == &Token::If {
            self.advance();
            self.expect(&Token::Not)?;
            self.expect(&Token::Exists)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn parse_if_exists(&mut self) -> Result<bool, ParseError> {
        if self.peek() == &Token::If {
            self.advance();
            self.expect(&Token::Exists)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Parse an expression that is the target of SET / REMOVE (identifier with
    /// optional property-access chain).  Does **not** consume operators like `=`.
    fn parse_target_expr(&mut self) -> Result<Expression, ParseError> {
        let name = self.expect_ident()?;
        let mut expr = Expression::Identifier(name);
        while self.peek() == &Token::Dot {
            self.advance();
            let prop = self.expect_ident()?;
            expr = Expression::PropertyAccess {
                object: Box::new(expr),
                property: prop,
            };
        }
        Ok(expr)
    }

    // -- graph type definition -------------------------------------------

    fn parse_graph_type_def(&mut self) -> Result<GraphType, ParseError> {
        self.expect(&Token::LBrace)?;
        let mut node_types = Vec::new();
        let mut edge_types = Vec::new();
        while self.peek() != &Token::RBrace && !self.at_end() {
            match self.peek() {
                Token::Node => {
                    self.advance();
                    node_types.push(self.parse_node_type_decl()?);
                }
                Token::Edge => {
                    self.advance();
                    edge_types.push(self.parse_edge_type_decl()?);
                }
                _ => return Err(self.error("expected NODE or EDGE in graph type")),
            }
            self.match_token(&Token::Comma);
        }
        self.expect(&Token::RBrace)?;
        Ok(GraphType {
            node_types,
            edge_types,
        })
    }

    fn parse_node_type_decl(&mut self) -> Result<NodeType, ParseError> {
        let mut labels = Vec::new();
        while self.match_token(&Token::Colon) {
            labels.push(self.expect_ident()?);
        }
        let properties = if self.peek() == &Token::LBrace {
            self.parse_property_decls()?
        } else {
            Vec::new()
        };
        Ok(NodeType { labels, properties })
    }

    fn parse_edge_type_decl(&mut self) -> Result<EdgeType, ParseError> {
        let mut labels = Vec::new();
        while self.match_token(&Token::Colon) {
            labels.push(self.expect_ident()?);
        }
        let source = if self.peek() == &Token::From {
            self.advance();
            vec![self.expect_ident()?]
        } else {
            Vec::new()
        };
        let destination = if self.peek() == &Token::To {
            self.advance();
            vec![self.expect_ident()?]
        } else {
            Vec::new()
        };
        let properties = if self.peek() == &Token::LBrace {
            self.parse_property_decls()?
        } else {
            Vec::new()
        };
        Ok(EdgeType {
            labels,
            source,
            destination,
            direction: Direction::Left,
            properties,
        })
    }

    fn parse_property_decls(&mut self) -> Result<Vec<PropertyDecl>, ParseError> {
        self.expect(&Token::LBrace)?;
        let mut props = Vec::new();
        while self.peek() != &Token::RBrace && !self.at_end() {
            let name = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            let property_type = self.expect_ident()?;
            let required = if self.peek() == &Token::Not {
                self.advance();
                self.expect(&Token::Null)?;
                true
            } else {
                false
            };
            props.push(PropertyDecl {
                name,
                property_type,
                required,
            });
            self.match_token(&Token::Comma);
        }
        self.expect(&Token::RBrace)?;
        Ok(props)
    }

    // -- composite queries / set operations ------------------------------

    fn maybe_parse_composite(
        &mut self,
        first_stmt: GqlStatement,
    ) -> Result<GqlStatement, ParseError> {
        let op = match self.peek() {
            Token::Union => {
                self.advance();
                SetOp::Union
            }
            Token::Intersect => {
                self.advance();
                SetOp::Intersect
            }
            Token::Except => {
                self.advance();
                SetOp::Except
            }
            _ => return Ok(first_stmt),
        };

        let all = self.match_token(&Token::All);
        let right_stmt = self.parse_statement()?;

        let right = match right_stmt {
            GqlStatement::CompositeQuery(cq) => SetOperand::SetOp(cq.body),
            other => SetOperand::Query(vec![other]),
        };

        Ok(GqlStatement::CompositeQuery(CompositeQueryStatement {
            body: SetOperation {
                op,
                all,
                left: Box::new(SetOperand::Query(vec![first_stmt])),
                right: Box::new(right),
            },
            order_by: None,
            limit_offset: None,
        }))
    }

    // -- graph pattern parsing -------------------------------------------

    fn parse_graph_pattern(&mut self) -> Result<GraphPattern, ParseError> {
        let mut paths = vec![self.parse_path_pattern()?];
        while self.match_token(&Token::Comma) {
            paths.push(self.parse_path_pattern()?);
        }
        Ok(GraphPattern { paths, mode: None })
    }

    fn parse_path_pattern(&mut self) -> Result<PathPattern, ParseError> {
        let mode = self.parse_optional_path_mode();
        let mut elements = Vec::new();

        if self.peek() == &Token::LParen {
            elements.push(PatternElement::Node(self.parse_node_pattern()?));
        }

        while self.is_edge_start() {
            elements.push(PatternElement::Edge(self.parse_edge_pattern()?));
            if self.peek() == &Token::LParen {
                elements.push(PatternElement::Node(self.parse_node_pattern()?));
            }
        }

        let quantifier = self.parse_optional_quantifier()?;

        Ok(PathPattern {
            variable: None,
            mode,
            shortest: None,
            elements,
            quantifier,
            where_clause: None,
        })
    }

    fn parse_optional_path_mode(&mut self) -> Option<PathMode> {
        match self.peek() {
            Token::Walk => {
                self.advance();
                Some(PathMode::Walk)
            }
            Token::Trail => {
                self.advance();
                Some(PathMode::Trail)
            }
            Token::Simple => {
                self.advance();
                Some(PathMode::Simple)
            }
            Token::Acyclic => {
                self.advance();
                Some(PathMode::Acyclic)
            }
            _ => None,
        }
    }

    fn parse_node_pattern(&mut self) -> Result<NodePattern, ParseError> {
        self.expect(&Token::LParen)?;
        let mut variable = None;
        let mut labels = Vec::new();
        let mut properties = Vec::new();

        // Optional variable – an identifier followed by `:`, `)`, or `{`.
        if matches!(self.peek(), Token::Ident(_)) {
            if self.peek_at(1).map_or(false, |t| {
                matches!(t, Token::Colon | Token::RParen | Token::LBrace)
            }) {
                variable = Some(self.expect_ident()?);
            }
        }

        while self.match_token(&Token::Colon) {
            labels.push(self.expect_ident()?);
        }

        if self.peek() == &Token::LBrace {
            properties = self.parse_property_map()?;
        }

        self.expect(&Token::RParen)?;
        Ok(NodePattern {
            variable,
            labels,
            properties,
        })
    }

    fn is_edge_start(&self) -> bool {
        match self.peek() {
            Token::LeftArrow | Token::Arrow => true,
            Token::Minus => self
                .peek_at(1)
                .map_or(false, |t| matches!(t, Token::LBracket)),
            _ => false,
        }
    }

    fn parse_edge_pattern(&mut self) -> Result<EdgePattern, ParseError> {
        let mut variable = None;
        let mut labels = Vec::new();
        let mut properties = Vec::new();
        let direction;

        match self.peek().clone() {
            Token::LeftArrow => {
                self.advance();
                if self.peek() == &Token::LBracket {
                    self.advance();
                    self.parse_edge_internals(&mut variable, &mut labels, &mut properties)?;
                    self.expect(&Token::RBracket)?;
                    self.expect(&Token::Minus)?;
                }
                direction = Direction::Right;
            }
            Token::Arrow => {
                self.advance();
                direction = Direction::Left;
            }
            Token::Minus => {
                self.advance(); // -
                self.expect(&Token::LBracket)?;
                self.parse_edge_internals(&mut variable, &mut labels, &mut properties)?;
                self.expect(&Token::RBracket)?;
                if self.match_token(&Token::Arrow) {
                    direction = Direction::Left;
                } else {
                    self.expect(&Token::Minus)?;
                    direction = Direction::Undirected;
                }
            }
            _ => return Err(self.error("expected edge pattern")),
        }

        let quantifier = self.parse_optional_quantifier()?;

        Ok(EdgePattern {
            variable,
            direction,
            labels,
            properties,
            quantifier,
        })
    }

    fn parse_edge_internals(
        &mut self,
        variable: &mut Option<String>,
        labels: &mut Vec<String>,
        properties: &mut Vec<(String, Expression)>,
    ) -> Result<(), ParseError> {
        if matches!(self.peek(), Token::Ident(_)) {
            if self.peek_at(1).map_or(false, |t| {
                matches!(t, Token::Colon | Token::RBracket | Token::LBrace)
            }) {
                *variable = Some(self.expect_ident()?);
            }
        }
        while self.match_token(&Token::Colon) {
            labels.push(self.expect_ident()?);
        }
        if self.peek() == &Token::LBrace {
            *properties = self.parse_property_map()?;
        }
        Ok(())
    }

    fn parse_property_map(&mut self) -> Result<Vec<(String, Expression)>, ParseError> {
        self.expect(&Token::LBrace)?;
        let mut props = Vec::new();
        if self.peek() != &Token::RBrace {
            let key = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            let value = self.parse_expression()?;
            props.push((key, value));
            while self.match_token(&Token::Comma) {
                let key = self.expect_ident()?;
                self.expect(&Token::Colon)?;
                let value = self.parse_expression()?;
                props.push((key, value));
            }
        }
        self.expect(&Token::RBrace)?;
        Ok(props)
    }

    fn parse_optional_quantifier(&mut self) -> Result<Option<Quantifier>, ParseError> {
        if self.peek() != &Token::LBrace {
            return Ok(None);
        }
        // Only treat as quantifier if it begins with an integer or comma.
        let is_quant = self
            .peek_at(1)
            .map_or(false, |t| matches!(t, Token::IntegerLit(_) | Token::Comma));
        if !is_quant {
            return Ok(None);
        }
        self.advance(); // {
        let min = if let Token::IntegerLit(n) = self.peek().clone() {
            self.advance();
            Some(n as u64)
        } else {
            None
        };
        let max = if self.match_token(&Token::Comma) {
            if let Token::IntegerLit(n) = self.peek().clone() {
                self.advance();
                Some(n as u64)
            } else {
                None // unbounded
            }
        } else {
            min // exact repetition
        };
        self.expect(&Token::RBrace)?;
        Ok(Some(Quantifier { min, max }))
    }

    // -- expression parsing (precedence climbing) ------------------------

    fn parse_expression(&mut self) -> Result<Expression, ParseError> {
        self.parse_or_expr()
    }

    fn parse_or_expr(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_xor_expr()?;
        while self.peek() == &Token::Or {
            self.advance();
            let right = self.parse_xor_expr()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::Or,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_xor_expr(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_and_expr()?;
        while self.peek() == &Token::Xor {
            self.advance();
            let right = self.parse_and_expr()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::Xor,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and_expr(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_not_expr()?;
        while self.peek() == &Token::And {
            self.advance();
            let right = self.parse_not_expr()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::And,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_not_expr(&mut self) -> Result<Expression, ParseError> {
        if self.peek() == &Token::Not {
            self.advance();
            let operand = self.parse_not_expr()?;
            Ok(Expression::UnaryOp {
                op: UnaryOp::Not,
                operand: Box::new(operand),
            })
        } else {
            self.parse_comparison_expr()
        }
    }

    fn parse_comparison_expr(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_addition_expr()?;
        loop {
            match self.peek() {
                Token::Eq => {
                    self.advance();
                    let right = self.parse_addition_expr()?;
                    left = Expression::BinaryOp {
                        left: Box::new(left),
                        op: BinaryOp::Eq,
                        right: Box::new(right),
                    };
                }
                Token::Neq => {
                    self.advance();
                    let right = self.parse_addition_expr()?;
                    left = Expression::BinaryOp {
                        left: Box::new(left),
                        op: BinaryOp::Neq,
                        right: Box::new(right),
                    };
                }
                Token::Lt => {
                    self.advance();
                    let right = self.parse_addition_expr()?;
                    left = Expression::BinaryOp {
                        left: Box::new(left),
                        op: BinaryOp::Lt,
                        right: Box::new(right),
                    };
                }
                Token::Gt => {
                    self.advance();
                    let right = self.parse_addition_expr()?;
                    left = Expression::BinaryOp {
                        left: Box::new(left),
                        op: BinaryOp::Gt,
                        right: Box::new(right),
                    };
                }
                Token::Le => {
                    self.advance();
                    let right = self.parse_addition_expr()?;
                    left = Expression::BinaryOp {
                        left: Box::new(left),
                        op: BinaryOp::Le,
                        right: Box::new(right),
                    };
                }
                Token::Ge => {
                    self.advance();
                    let right = self.parse_addition_expr()?;
                    left = Expression::BinaryOp {
                        left: Box::new(left),
                        op: BinaryOp::Ge,
                        right: Box::new(right),
                    };
                }
                Token::Is => {
                    self.advance();
                    if self.match_token(&Token::Not) {
                        self.expect(&Token::Null)?;
                        left = Expression::IsNotNull(Box::new(left));
                    } else {
                        self.expect(&Token::Null)?;
                        left = Expression::IsNull(Box::new(left));
                    }
                }
                Token::In => {
                    self.advance();
                    let list = self.parse_addition_expr()?;
                    left = Expression::In {
                        operand: Box::new(left),
                        list: Box::new(list),
                    };
                }
                Token::Like => {
                    self.advance();
                    let pattern = self.parse_addition_expr()?;
                    left = Expression::Like {
                        operand: Box::new(left),
                        pattern: Box::new(pattern),
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_addition_expr(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_multiplication_expr()?;
        loop {
            match self.peek() {
                Token::Plus => {
                    self.advance();
                    let right = self.parse_multiplication_expr()?;
                    left = Expression::BinaryOp {
                        left: Box::new(left),
                        op: BinaryOp::Add,
                        right: Box::new(right),
                    };
                }
                Token::Minus => {
                    self.advance();
                    let right = self.parse_multiplication_expr()?;
                    left = Expression::BinaryOp {
                        left: Box::new(left),
                        op: BinaryOp::Sub,
                        right: Box::new(right),
                    };
                }
                Token::DoublePipe => {
                    self.advance();
                    let right = self.parse_multiplication_expr()?;
                    left = Expression::BinaryOp {
                        left: Box::new(left),
                        op: BinaryOp::Concat,
                        right: Box::new(right),
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_multiplication_expr(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_unary_expr()?;
        loop {
            match self.peek() {
                Token::Star => {
                    self.advance();
                    let right = self.parse_unary_expr()?;
                    left = Expression::BinaryOp {
                        left: Box::new(left),
                        op: BinaryOp::Mul,
                        right: Box::new(right),
                    };
                }
                Token::Slash => {
                    self.advance();
                    let right = self.parse_unary_expr()?;
                    left = Expression::BinaryOp {
                        left: Box::new(left),
                        op: BinaryOp::Div,
                        right: Box::new(right),
                    };
                }
                Token::Percent => {
                    self.advance();
                    let right = self.parse_unary_expr()?;
                    left = Expression::BinaryOp {
                        left: Box::new(left),
                        op: BinaryOp::Mod,
                        right: Box::new(right),
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_unary_expr(&mut self) -> Result<Expression, ParseError> {
        match self.peek() {
            Token::Minus => {
                self.advance();
                let operand = self.parse_unary_expr()?;
                Ok(Expression::UnaryOp {
                    op: UnaryOp::Neg,
                    operand: Box::new(operand),
                })
            }
            Token::Plus => {
                self.advance();
                let operand = self.parse_unary_expr()?;
                Ok(Expression::UnaryOp {
                    op: UnaryOp::Pos,
                    operand: Box::new(operand),
                })
            }
            _ => self.parse_postfix_expr(),
        }
    }

    fn parse_postfix_expr(&mut self) -> Result<Expression, ParseError> {
        let mut expr = self.parse_primary()?;
        while self.peek() == &Token::Dot {
            self.advance();
            let property = self.expect_ident()?;
            expr = Expression::PropertyAccess {
                object: Box::new(expr),
                property,
            };
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expression, ParseError> {
        match self.peek().clone() {
            Token::IntegerLit(n) => {
                self.advance();
                Ok(Expression::Literal(Literal::Integer(n)))
            }
            Token::FloatLit(n) => {
                self.advance();
                Ok(Expression::Literal(Literal::Float(n)))
            }
            Token::StringLit(s) => {
                self.advance();
                Ok(Expression::Literal(Literal::String(s)))
            }
            Token::True | Token::BoolLit(true) => {
                self.advance();
                Ok(Expression::Literal(Literal::Bool(true)))
            }
            Token::False | Token::BoolLit(false) => {
                self.advance();
                Ok(Expression::Literal(Literal::Bool(false)))
            }
            Token::Null | Token::NullLit => {
                self.advance();
                Ok(Expression::Literal(Literal::Null))
            }
            Token::Parameter(name) => {
                self.advance();
                Ok(Expression::Parameter(name))
            }
            Token::LParen => {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            Token::LBracket => {
                self.advance();
                let mut items = Vec::new();
                if self.peek() != &Token::RBracket {
                    items.push(self.parse_expression()?);
                    while self.match_token(&Token::Comma) {
                        items.push(self.parse_expression()?);
                    }
                }
                self.expect(&Token::RBracket)?;
                Ok(Expression::List(items))
            }
            Token::LBrace => {
                self.advance();
                let mut entries = Vec::new();
                if self.peek() != &Token::RBrace {
                    let key = self.expect_ident()?;
                    self.expect(&Token::Colon)?;
                    let value = self.parse_expression()?;
                    entries.push((key, value));
                    while self.match_token(&Token::Comma) {
                        let key = self.expect_ident()?;
                        self.expect(&Token::Colon)?;
                        let value = self.parse_expression()?;
                        entries.push((key, value));
                    }
                }
                self.expect(&Token::RBrace)?;
                Ok(Expression::Map(entries))
            }
            Token::Case => self.parse_case_expression(),
            Token::Exists => {
                self.advance();
                self.expect(&Token::LBrace)?;
                let stmt = self.parse_statement()?;
                self.expect(&Token::RBrace)?;
                Ok(Expression::Exists {
                    subquery: Box::new(stmt),
                })
            }
            // Aggregate functions
            Token::Count => self.parse_aggregate(AggregateFunction::Count),
            Token::Sum => self.parse_aggregate(AggregateFunction::Sum),
            Token::Avg => self.parse_aggregate(AggregateFunction::Avg),
            Token::Min => self.parse_aggregate(AggregateFunction::Min),
            Token::Max => self.parse_aggregate(AggregateFunction::Max),
            Token::Collect => self.parse_aggregate(AggregateFunction::Collect),
            // Identifiers / function calls
            Token::Ident(name) => {
                self.advance();
                if self.peek() == &Token::LParen {
                    self.advance();
                    let mut args = Vec::new();
                    if self.peek() != &Token::RParen {
                        args.push(self.parse_expression()?);
                        while self.match_token(&Token::Comma) {
                            args.push(self.parse_expression()?);
                        }
                    }
                    self.expect(&Token::RParen)?;
                    Ok(Expression::FunctionCall { name, args })
                } else {
                    Ok(Expression::Identifier(name))
                }
            }
            Token::Star => {
                self.advance();
                Ok(Expression::Identifier("*".to_string()))
            }
            _ => Err(self.error(format!(
                "unexpected token in expression: {:?}",
                self.peek()
            ))),
        }
    }

    fn parse_aggregate(
        &mut self,
        function: AggregateFunction,
    ) -> Result<Expression, ParseError> {
        self.advance(); // consume aggregate keyword
        self.expect(&Token::LParen)?;

        if self.match_token(&Token::Star) {
            self.expect(&Token::RParen)?;
            return Ok(Expression::Aggregate {
                function,
                distinct: false,
                arg: None,
            });
        }

        let distinct = self.match_token(&Token::Distinct);
        let arg = if self.peek() != &Token::RParen {
            Some(Box::new(self.parse_expression()?))
        } else {
            None
        };
        self.expect(&Token::RParen)?;
        Ok(Expression::Aggregate {
            function,
            distinct,
            arg,
        })
    }

    fn parse_case_expression(&mut self) -> Result<Expression, ParseError> {
        self.expect(&Token::Case)?;

        let operand = if self.peek() != &Token::When {
            Some(Box::new(self.parse_expression()?))
        } else {
            None
        };

        let mut when_clauses = Vec::new();
        while self.match_token(&Token::When) {
            let condition = self.parse_expression()?;
            self.expect(&Token::Then)?;
            let result = self.parse_expression()?;
            when_clauses.push((condition, result));
        }

        let else_clause = if self.match_token(&Token::Else) {
            Some(Box::new(self.parse_expression()?))
        } else {
            None
        };

        self.expect(&Token::End)?;
        Ok(Expression::Case {
            operand,
            when_clauses,
            else_clause,
        })
    }

    // -- clause parsing --------------------------------------------------

    fn parse_where_clause(&mut self) -> Result<WhereClause, ParseError> {
        self.expect(&Token::Where)?;
        let condition = self.parse_expression()?;
        Ok(WhereClause {
            condition: Box::new(condition),
        })
    }

    fn parse_order_by_clause(&mut self) -> Result<OrderByClause, ParseError> {
        self.expect(&Token::Order)?;
        self.expect(&Token::By)?;
        let mut items = vec![self.parse_order_by_item()?];
        while self.match_token(&Token::Comma) {
            items.push(self.parse_order_by_item()?);
        }
        Ok(OrderByClause { items })
    }

    fn parse_order_by_item(&mut self) -> Result<OrderByItem, ParseError> {
        let expression = self.parse_expression()?;
        let direction = if self.match_token(&Token::Desc) {
            SortDirection::Desc
        } else {
            self.match_token(&Token::Asc);
            SortDirection::Asc
        };
        Ok(OrderByItem {
            expression,
            direction,
        })
    }

    fn parse_limit_offset(&mut self) -> Result<Option<LimitOffsetClause>, ParseError> {
        let limit = if self.match_token(&Token::Limit) {
            Some(self.parse_expression()?)
        } else {
            None
        };
        let offset = if self.match_token(&Token::Offset) {
            Some(self.parse_expression()?)
        } else {
            None
        };
        if limit.is_some() || offset.is_some() {
            Ok(Some(LimitOffsetClause { limit, offset }))
        } else {
            Ok(None)
        }
    }

    fn parse_group_by_clause(&mut self) -> Result<GroupByClause, ParseError> {
        self.expect(&Token::Group)?;
        self.expect(&Token::By)?;
        let mut expressions = vec![self.parse_expression()?];
        while self.match_token(&Token::Comma) {
            expressions.push(self.parse_expression()?);
        }
        Ok(GroupByClause { expressions })
    }

    fn parse_return_items(&mut self) -> Result<Vec<ReturnItem>, ParseError> {
        let mut items = vec![self.parse_return_item()?];
        while self.match_token(&Token::Comma) {
            items.push(self.parse_return_item()?);
        }
        Ok(items)
    }

    fn parse_return_item(&mut self) -> Result<ReturnItem, ParseError> {
        let expression = self.parse_expression()?;
        let alias = if self.match_token(&Token::As) {
            Some(self.expect_ident()?)
        } else {
            None
        };
        Ok(ReturnItem { expression, alias })
    }
}

// ---------------------------------------------------------------------------
// Convenience function
// ---------------------------------------------------------------------------

/// Lex and parse a GQL source string in one step.
pub fn parse(input: &str) -> Result<GqlProgram, ParseError> {
    let tokens = Lexer::new(input).tokenize().map_err(|e| ParseError {
        message: e.message,
        line: e.line,
        column: e.column,
    })?;
    let mut parser = Parser::new(tokens);
    parser.parse()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_return_simple() {
        let program = parse("MATCH (n:Person) RETURN n").unwrap();
        assert_eq!(program.statements.len(), 2);

        match &program.statements[0] {
            GqlStatement::Match(m) => {
                assert!(!m.optional);
                assert_eq!(m.pattern.paths.len(), 1);
                let elems = &m.pattern.paths[0].elements;
                assert_eq!(elems.len(), 1);
                match &elems[0] {
                    PatternElement::Node(n) => {
                        assert_eq!(n.variable.as_deref(), Some("n"));
                        assert_eq!(n.labels, vec!["Person"]);
                    }
                    _ => panic!("expected node"),
                }
            }
            _ => panic!("expected MATCH"),
        }

        match &program.statements[1] {
            GqlStatement::Return(r) => {
                assert_eq!(r.items.len(), 1);
                assert_eq!(
                    r.items[0].expression,
                    Expression::Identifier("n".into())
                );
            }
            _ => panic!("expected RETURN"),
        }
    }

    #[test]
    fn test_match_edge_where_return() {
        let program =
            parse("MATCH (a)-[r:KNOWS]->(b) WHERE a.name = 'Alice' RETURN b.name").unwrap();
        assert_eq!(program.statements.len(), 2);

        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let elems = &m.pattern.paths[0].elements;
                assert_eq!(elems.len(), 3);

                match &elems[0] {
                    PatternElement::Node(n) => {
                        assert_eq!(n.variable.as_deref(), Some("a"));
                    }
                    _ => panic!("expected node"),
                }
                match &elems[1] {
                    PatternElement::Edge(e) => {
                        assert_eq!(e.variable.as_deref(), Some("r"));
                        assert_eq!(e.labels, vec!["KNOWS"]);
                        assert_eq!(e.direction, Direction::Left);
                    }
                    _ => panic!("expected edge"),
                }
                match &elems[2] {
                    PatternElement::Node(n) => {
                        assert_eq!(n.variable.as_deref(), Some("b"));
                    }
                    _ => panic!("expected node"),
                }

                let wc = m.where_clause.as_ref().expect("expected WHERE");
                match wc.condition.as_ref() {
                    Expression::BinaryOp { left, op, right } => {
                        assert_eq!(*op, BinaryOp::Eq);
                        assert_eq!(
                            **left,
                            Expression::PropertyAccess {
                                object: Box::new(Expression::Identifier("a".into())),
                                property: "name".into(),
                            }
                        );
                        assert_eq!(
                            **right,
                            Expression::Literal(Literal::String("Alice".into()))
                        );
                    }
                    _ => panic!("expected binary op"),
                }
            }
            _ => panic!("expected MATCH"),
        }

        match &program.statements[1] {
            GqlStatement::Return(r) => {
                assert_eq!(r.items.len(), 1);
                assert_eq!(
                    r.items[0].expression,
                    Expression::PropertyAccess {
                        object: Box::new(Expression::Identifier("b".into())),
                        property: "name".into(),
                    }
                );
            }
            _ => panic!("expected RETURN"),
        }
    }

    #[test]
    fn test_match_where_and_order_limit() {
        let program = parse(
            "MATCH (n) WHERE n.age > 30 AND n.active = TRUE \
             RETURN n.name, n.age ORDER BY n.age DESC LIMIT 10",
        )
        .unwrap();
        assert_eq!(program.statements.len(), 2);

        match &program.statements[0] {
            GqlStatement::Match(m) => {
                assert!(m.where_clause.is_some());
                let cond = m.where_clause.as_ref().unwrap().condition.as_ref();
                match cond {
                    Expression::BinaryOp { op, .. } => assert_eq!(*op, BinaryOp::And),
                    _ => panic!("expected AND"),
                }
            }
            _ => panic!("expected MATCH"),
        }

        match &program.statements[1] {
            GqlStatement::Return(r) => {
                assert_eq!(r.items.len(), 2);
                let ob = r.order_by.as_ref().expect("expected ORDER BY");
                assert_eq!(ob.items.len(), 1);
                assert_eq!(ob.items[0].direction, SortDirection::Desc);
                let lo = r.limit_offset.as_ref().expect("expected LIMIT");
                assert_eq!(lo.limit, Some(Expression::Literal(Literal::Integer(10))));
                assert!(lo.offset.is_none());
            }
            _ => panic!("expected RETURN"),
        }
    }

    #[test]
    fn test_insert_node() {
        let program = parse("INSERT (:Person {name: 'Bob', age: 25})").unwrap();
        assert_eq!(program.statements.len(), 1);

        match &program.statements[0] {
            GqlStatement::Insert(ins) => {
                assert_eq!(ins.patterns.len(), 1);
                let elems = &ins.patterns[0].elements;
                assert_eq!(elems.len(), 1);
                match &elems[0] {
                    PatternElement::Node(n) => {
                        assert!(n.variable.is_none());
                        assert_eq!(n.labels, vec!["Person"]);
                        assert_eq!(n.properties.len(), 2);
                        assert_eq!(n.properties[0].0, "name");
                        assert_eq!(
                            n.properties[0].1,
                            Expression::Literal(Literal::String("Bob".into()))
                        );
                        assert_eq!(n.properties[1].0, "age");
                        assert_eq!(
                            n.properties[1].1,
                            Expression::Literal(Literal::Integer(25))
                        );
                    }
                    _ => panic!("expected node"),
                }
            }
            _ => panic!("expected INSERT"),
        }
    }

    #[test]
    fn test_match_set() {
        let program =
            parse("MATCH (n:Person {name: 'Alice'}) SET n.age = 31").unwrap();
        assert_eq!(program.statements.len(), 2);

        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let node = match &m.pattern.paths[0].elements[0] {
                    PatternElement::Node(n) => n,
                    _ => panic!("expected node"),
                };
                assert_eq!(node.variable.as_deref(), Some("n"));
                assert_eq!(node.labels, vec!["Person"]);
                assert_eq!(node.properties.len(), 1);
            }
            _ => panic!("expected MATCH"),
        }

        match &program.statements[1] {
            GqlStatement::Set(s) => {
                assert_eq!(s.items.len(), 1);
                match &s.items[0] {
                    SetItem::Property {
                        target,
                        property,
                        value,
                    } => {
                        assert_eq!(*target, Expression::Identifier("n".into()));
                        assert_eq!(property, "age");
                        assert_eq!(
                            *value,
                            Expression::Literal(Literal::Integer(31))
                        );
                    }
                    _ => panic!("expected SetItem::Property"),
                }
            }
            _ => panic!("expected SET"),
        }
    }

    #[test]
    fn test_match_delete() {
        let program =
            parse("MATCH (n:Person {name: 'Alice'}) DELETE n").unwrap();
        assert_eq!(program.statements.len(), 2);

        match &program.statements[1] {
            GqlStatement::Delete(d) => {
                assert!(!d.detach);
                assert_eq!(d.targets.len(), 1);
                assert_eq!(d.targets[0], Expression::Identifier("n".into()));
            }
            _ => panic!("expected DELETE"),
        }
    }
}
