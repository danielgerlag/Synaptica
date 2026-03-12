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

const MAX_EXPR_DEPTH: usize = 256;

pub struct Parser {
    tokens: Vec<SpannedToken>,
    pos: usize,
    depth: usize,
}

impl Parser {
    pub fn new(tokens: Vec<SpannedToken>) -> Parser {
        let tokens: Vec<SpannedToken> = tokens
            .into_iter()
            .filter(|st| !matches!(st.token, Token::Comment(_)))
            .collect();
        Parser { tokens, pos: 0, depth: 0 }
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

    fn enter_depth(&mut self) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAX_EXPR_DEPTH {
            Err(self.error("expression nesting depth exceeded"))
        } else {
            Ok(())
        }
    }

    fn exit_depth(&mut self) {
        self.depth -= 1;
    }

    fn lookahead_is_lparen(&self) -> bool {
        self.peek_at(1) == Some(&Token::LParen)
    }

    fn keyword_to_string(&self) -> String {
        match self.peek() {
            Token::Type => "type".to_string(),
            Token::Count => "count".to_string(),
            Token::Sum => "sum".to_string(),
            Token::Avg => "avg".to_string(),
            Token::Min => "min".to_string(),
            Token::Max => "max".to_string(),
            Token::Collect => "collect".to_string(),
            _ => format!("{:?}", self.peek()).to_lowercase(),
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

    /// Accept an identifier or a keyword token as a property/field name.
    /// GQL allows reserved words in property positions (e.g. `n.offset`, `{value: 1}`).
    fn expect_ident_or_keyword(&mut self) -> Result<String, ParseError> {
        if let Token::Ident(name) = self.peek().clone() {
            self.advance();
            return Ok(name);
        }
        // Map keyword tokens to their lowercase string form
        let name = match self.peek() {
            Token::Match => "match",
            Token::Return => "return",
            Token::Where => "where",
            Token::Insert => "insert",
            Token::Create => "create",
            Token::Set => "set",
            Token::Delete => "delete",
            Token::Detach => "detach",
            Token::Remove => "remove",
            Token::Drop => "drop",
            Token::With => "with",
            Token::Order => "order",
            Token::By => "by",
            Token::Limit => "limit",
            Token::Offset => "offset",
            Token::Asc => "asc",
            Token::Desc => "desc",
            Token::Distinct => "distinct",
            Token::As => "as",
            Token::And => "and",
            Token::Or => "or",
            Token::Not => "not",
            Token::Xor => "xor",
            Token::Is => "is",
            Token::Null => "null",
            Token::True => "true",
            Token::False => "false",
            Token::In => "in",
            Token::Exists => "exists",
            Token::Case => "case",
            Token::When => "when",
            Token::Then => "then",
            Token::Else => "else",
            Token::End => "end",
            Token::Union => "union",
            Token::All => "all",
            Token::Optional => "optional",
            Token::Call => "call",
            Token::Yield => "yield",
            Token::Like => "like",
            Token::Node => "node",
            Token::Edge => "edge",
            Token::Graph => "graph",
            Token::Type => "type",
            Token::Group => "group",
            Token::Having => "having",
            Token::Let => "let",
            Token::For => "for",
            Token::Filter => "filter",
            Token::Count => "count",
            Token::Sum => "sum",
            Token::Avg => "avg",
            Token::Min => "min",
            Token::Max => "max",
            Token::Collect => "collect",
            Token::Path => "path",
            Token::Cost => "cost",
            Token::From => "from",
            Token::To => "to",
            _ => {
                return Err(self.error(format!(
                    "expected identifier, found {:?}",
                    self.peek()
                )));
            }
        };
        self.advance();
        Ok(name.to_string())
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
            Token::With => self.parse_with_statement(),
            Token::Call => self.parse_call_statement(),
            Token::Insert => self.parse_insert_statement(),
            Token::Set => self.parse_set_statement(),
            Token::Delete => self.parse_delete_statement(),
            Token::Detach => self.parse_delete_statement(),
            Token::Remove => self.parse_remove_statement(),
            Token::Create => self.parse_create_statement(),
            Token::Drop => self.parse_drop_statement(),
            Token::List => self.parse_list_statement(),
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

    fn parse_with_statement(&mut self) -> Result<GqlStatement, ParseError> {
        self.expect(&Token::With)?;
        let distinct = self.match_token(&Token::Distinct);
        let items = self.parse_return_items()?;

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

        Ok(GqlStatement::With(WithStatement {
            distinct,
            items,
            where_clause,
            order_by,
            limit_offset,
        }))
    }

    fn parse_call_statement(&mut self) -> Result<GqlStatement, ParseError> {
        self.expect(&Token::Call)?;
        let mut name = self.expect_ident()?;
        // Handle dotted names like db.labels
        while self.match_token(&Token::Dot) {
            let part = self.expect_ident()?;
            name = format!("{}.{}", name, part);
        }
        // Parse arguments
        let arguments = if self.match_token(&Token::LParen) {
            let mut args = Vec::new();
            if self.peek() != &Token::RParen {
                args.push(self.parse_expression()?);
                while self.match_token(&Token::Comma) {
                    args.push(self.parse_expression()?);
                }
            }
            self.expect(&Token::RParen)?;
            args
        } else {
            Vec::new()
        };
        // Parse YIELD
        let yield_items = if self.match_token(&Token::Yield) {
            let mut items = vec![self.expect_ident()?];
            while self.match_token(&Token::Comma) {
                items.push(self.expect_ident()?);
            }
            Some(items)
        } else {
            None
        };
        Ok(GqlStatement::Call(CallStatement {
            procedure: name,
            arguments,
            yield_items,
        }))
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
        match self.peek() {
            Token::Unique | Token::Index => self.parse_create_index_rest(false),
            Token::Graph => {
                self.advance(); // consume Graph
                if self.peek() == &Token::Type {
                    self.advance();
                    self.parse_create_graph_type_rest()
                } else {
                    self.parse_create_graph_rest()
                }
            }
            _ => Err(self.error(format!("expected GRAPH, INDEX, or UNIQUE after CREATE, found {:?}", self.peek()))),
        }
    }

    /// Parse: CREATE [UNIQUE] INDEX [IF NOT EXISTS] name FOR (v:Label) ON (v.prop1, ...)
    fn parse_create_index_rest(&mut self, _from_unique: bool) -> Result<GqlStatement, ParseError> {
        let unique = if self.peek() == &Token::Unique {
            self.advance();
            true
        } else {
            false
        };
        self.expect(&Token::Index)?;
        let if_not_exists = self.parse_if_not_exists()?;
        let name = self.expect_ident_or_keyword()?;

        // FOR (v:Label) — the entity pattern
        self.expect(&Token::For)?;
        self.expect(&Token::LParen)?;
        let _var = self.expect_ident_or_keyword()?;
        let (entity_type, label) = if self.peek() == &Token::Colon {
            self.advance();
            let lbl = self.expect_ident_or_keyword()?;
            ("node".to_string(), Some(lbl))
        } else {
            ("node".to_string(), None)
        };
        self.expect(&Token::RParen)?;

        // ON (prop1, prop2, ...)
        self.expect(&Token::On)?;
        self.expect(&Token::LParen)?;
        let mut property_names = Vec::new();
        loop {
            // Accept "v.prop" or just "prop"
            let first = self.expect_ident_or_keyword()?;
            let prop = if self.peek() == &Token::Dot {
                self.advance();
                self.expect_ident_or_keyword()?
            } else {
                first
            };
            property_names.push(prop);
            if self.peek() == &Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        self.expect(&Token::RParen)?;

        Ok(GqlStatement::CreateIndex(CreateIndexStatement {
            name,
            unique,
            entity_type,
            label,
            property_names,
            if_not_exists,
        }))
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

    fn parse_drop_statement(&mut self) -> Result<GqlStatement, ParseError> {
        self.expect(&Token::Drop)?;
        match self.peek() {
            Token::Index => {
                self.advance();
                let if_exists = self.parse_if_exists()?;
                let name = self.expect_ident_or_keyword()?;
                Ok(GqlStatement::DropIndex(DropIndexStatement { name, if_exists }))
            }
            Token::Graph => {
                self.advance();
                let if_exists = self.parse_if_exists()?;
                let name = self.expect_ident()?;
                Ok(GqlStatement::DropGraph(DropGraphStatement { name, if_exists }))
            }
            _ => Err(self.error(format!("expected GRAPH or INDEX after DROP, found {:?}", self.peek()))),
        }
    }

    fn parse_list_statement(&mut self) -> Result<GqlStatement, ParseError> {
        self.expect(&Token::List)?;
        match self.peek() {
            Token::Graph => {
                self.advance();
                // Accept both "LIST GRAPH" and "LIST GRAPHS" (the 'S' is an ident)
                if let Token::Ident(s) = self.peek() {
                    if s.eq_ignore_ascii_case("s") || s.eq_ignore_ascii_case("graphs") {
                        self.advance();
                    }
                }
                Ok(GqlStatement::ListGraphs)
            }
            Token::Ident(s) if s.eq_ignore_ascii_case("GRAPHS") => {
                self.advance();
                Ok(GqlStatement::ListGraphs)
            }
            _ => Err(self.error(format!("expected GRAPHS after LIST, found {:?}", self.peek()))),
        }
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
            let prop = self.expect_ident_or_keyword()?;
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
            direction: Direction::Outgoing,
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

        self.enter_depth()?;
        let all = self.match_token(&Token::All);
        let right_stmt = self.parse_statement()?;
        self.exit_depth();

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
                direction = Direction::Incoming;
            }
            Token::Arrow => {
                self.advance();
                direction = Direction::Outgoing;
            }
            Token::Minus => {
                self.advance(); // -
                self.expect(&Token::LBracket)?;
                self.parse_edge_internals(&mut variable, &mut labels, &mut properties)?;
                self.expect(&Token::RBracket)?;
                if self.match_token(&Token::Arrow) {
                    direction = Direction::Outgoing;
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
            let key = self.expect_ident_or_keyword()?;
            self.expect(&Token::Colon)?;
            let value = self.parse_expression()?;
            props.push((key, value));
            while self.match_token(&Token::Comma) {
                let key = self.expect_ident_or_keyword()?;
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
            if n < 0 {
                return Err(self.error("quantifier min must be non-negative"));
            }
            Some(n as u64)
        } else {
            None
        };
        let max = if self.match_token(&Token::Comma) {
            if let Token::IntegerLit(n) = self.peek().clone() {
                self.advance();
                if n < 0 {
                    return Err(self.error("quantifier max must be non-negative"));
                }
                Some(n as u64)
            } else {
                None // unbounded
            }
        } else {
            min // exact repetition
        };
        if let (Some(min_val), Some(max_val)) = (min, max) {
            if max_val < min_val {
                return Err(self.error(&format!(
                    "invalid quantifier: max ({}) cannot be less than min ({})",
                    max_val, min_val
                )));
            }
        }
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
            self.enter_depth()?;
            let operand = self.parse_not_expr()?;
            self.exit_depth();
            Ok(Expression::UnaryOp {
                op: UnaryOp::Not,
                operand: Box::new(operand),
            })
        } else {
            self.parse_comparison_expr()
        }
    }

    fn parse_comparison_expr(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_concat_expr()?;
        let mut had_comparison = false;
        loop {
            match self.peek() {
                Token::Eq | Token::Neq | Token::Lt | Token::Gt | Token::Le | Token::Ge => {
                    if had_comparison {
                        return Err(self.error("chained comparison operators are not allowed; use AND to combine conditions"));
                    }
                    had_comparison = true;
                    let op = match self.peek() {
                        Token::Eq => BinaryOp::Eq,
                        Token::Neq => BinaryOp::Neq,
                        Token::Lt => BinaryOp::Lt,
                        Token::Gt => BinaryOp::Gt,
                        Token::Le => BinaryOp::Le,
                        Token::Ge => BinaryOp::Ge,
                        _ => unreachable!(),
                    };
                    self.advance();
                    let right = self.parse_concat_expr()?;
                    left = Expression::BinaryOp {
                        left: Box::new(left),
                        op,
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
                    let list = self.parse_concat_expr()?;
                    left = Expression::In {
                        operand: Box::new(left),
                        list: Box::new(list),
                    };
                }
                Token::Like => {
                    self.advance();
                    let pattern = self.parse_concat_expr()?;
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

    fn parse_concat_expr(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_addition_expr()?;
        while self.peek() == &Token::DoublePipe {
            self.advance();
            let right = self.parse_addition_expr()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::Concat,
                right: Box::new(right),
            };
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
                self.enter_depth()?;
                let operand = self.parse_unary_expr()?;
                self.exit_depth();
                Ok(Expression::UnaryOp {
                    op: UnaryOp::Neg,
                    operand: Box::new(operand),
                })
            }
            Token::Plus => {
                self.advance();
                self.enter_depth()?;
                let operand = self.parse_unary_expr()?;
                self.exit_depth();
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
            let property = self.expect_ident_or_keyword()?;
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
                self.enter_depth()?;
                self.advance();
                let expr = self.parse_expression()?;
                self.expect(&Token::RParen)?;
                self.exit_depth();
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
            // Aggregate functions — only when followed by '('
            Token::Count if self.lookahead_is_lparen() => self.parse_aggregate(AggregateFunction::Count),
            Token::Sum if self.lookahead_is_lparen() => self.parse_aggregate(AggregateFunction::Sum),
            Token::Avg if self.lookahead_is_lparen() => self.parse_aggregate(AggregateFunction::Avg),
            Token::Min if self.lookahead_is_lparen() => self.parse_aggregate(AggregateFunction::Min),
            Token::Max if self.lookahead_is_lparen() => self.parse_aggregate(AggregateFunction::Max),
            Token::Collect if self.lookahead_is_lparen() => self.parse_aggregate(AggregateFunction::Collect),
            // Keywords that can also be function calls (e.g. TYPE(e))
            Token::Type => {
                let name = self.keyword_to_string();
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
            // Keywords used as identifiers (e.g. AS sum, AS type)
            Token::Count | Token::Sum | Token::Avg | Token::Min | Token::Max | Token::Collect => {
                let name = self.keyword_to_string();
                self.advance();
                Ok(Expression::Identifier(name))
            }
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
            Some(self.expect_ident_or_keyword()?)
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
                        assert_eq!(e.direction, Direction::Outgoing);
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

    // ===================================================================
    // Pattern tests
    // ===================================================================

    #[test]
    fn test_undirected_edge() {
        let program = parse("MATCH (a)-[r:FRIEND]-(b) RETURN a, b").unwrap();
        assert_eq!(program.statements.len(), 2);
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let elems = &m.pattern.paths[0].elements;
                assert_eq!(elems.len(), 3);
                match &elems[1] {
                    PatternElement::Edge(e) => {
                        assert_eq!(e.variable.as_deref(), Some("r"));
                        assert_eq!(e.labels, vec!["FRIEND"]);
                        assert_eq!(e.direction, Direction::Undirected);
                    }
                    _ => panic!("expected edge"),
                }
            }
            _ => panic!("expected MATCH"),
        }
    }

    #[test]
    fn test_left_arrow_edge() {
        let program = parse("MATCH (a)<-[r:KNOWS]-(b) RETURN a").unwrap();
        assert_eq!(program.statements.len(), 2);
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let elems = &m.pattern.paths[0].elements;
                assert_eq!(elems.len(), 3);
                match &elems[1] {
                    PatternElement::Edge(e) => {
                        assert_eq!(e.variable.as_deref(), Some("r"));
                        assert_eq!(e.labels, vec!["KNOWS"]);
                        assert_eq!(e.direction, Direction::Incoming);
                    }
                    _ => panic!("expected edge"),
                }
            }
            _ => panic!("expected MATCH"),
        }
    }

    #[test]
    fn test_node_no_variable() {
        let program =
            parse("MATCH (:Person {name: 'X'}) RETURN 1").unwrap();
        assert_eq!(program.statements.len(), 2);
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let node = match &m.pattern.paths[0].elements[0] {
                    PatternElement::Node(n) => n,
                    _ => panic!("expected node"),
                };
                assert!(node.variable.is_none());
                assert_eq!(node.labels, vec!["Person"]);
                assert_eq!(node.properties.len(), 1);
            }
            _ => panic!("expected MATCH"),
        }
    }

    #[test]
    fn test_node_multiple_labels() {
        let program = parse("MATCH (n:Person:Employee) RETURN n").unwrap();
        assert_eq!(program.statements.len(), 2);
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let node = match &m.pattern.paths[0].elements[0] {
                    PatternElement::Node(n) => n,
                    _ => panic!("expected node"),
                };
                assert_eq!(node.variable.as_deref(), Some("n"));
                assert_eq!(node.labels, vec!["Person", "Employee"]);
            }
            _ => panic!("expected MATCH"),
        }
    }

    #[test]
    fn test_node_with_properties() {
        let program =
            parse("MATCH (n:Person {name: 'Alice', age: 30}) RETURN n").unwrap();
        assert_eq!(program.statements.len(), 2);
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let node = match &m.pattern.paths[0].elements[0] {
                    PatternElement::Node(n) => n,
                    _ => panic!("expected node"),
                };
                assert_eq!(node.variable.as_deref(), Some("n"));
                assert_eq!(node.labels, vec!["Person"]);
                assert_eq!(node.properties.len(), 2);
                assert_eq!(node.properties[0].0, "name");
                assert_eq!(
                    node.properties[0].1,
                    Expression::Literal(Literal::String("Alice".into()))
                );
                assert_eq!(node.properties[1].0, "age");
                assert_eq!(
                    node.properties[1].1,
                    Expression::Literal(Literal::Integer(30))
                );
            }
            _ => panic!("expected MATCH"),
        }
    }

    #[test]
    fn test_chain_of_edges() {
        let program =
            parse("MATCH (a)-[r1:KNOWS]->(b)-[r2:KNOWS]->(c) RETURN a, b, c")
                .unwrap();
        assert_eq!(program.statements.len(), 2);
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let elems = &m.pattern.paths[0].elements;
                // 3 nodes + 2 edges = 5 elements
                assert_eq!(elems.len(), 5);
                assert!(matches!(&elems[0], PatternElement::Node(_)));
                assert!(matches!(&elems[1], PatternElement::Edge(_)));
                assert!(matches!(&elems[2], PatternElement::Node(_)));
                assert!(matches!(&elems[3], PatternElement::Edge(_)));
                assert!(matches!(&elems[4], PatternElement::Node(_)));

                match &elems[1] {
                    PatternElement::Edge(e) => {
                        assert_eq!(e.variable.as_deref(), Some("r1"));
                        assert_eq!(e.direction, Direction::Outgoing);
                    }
                    _ => unreachable!(),
                }
                match &elems[3] {
                    PatternElement::Edge(e) => {
                        assert_eq!(e.variable.as_deref(), Some("r2"));
                        assert_eq!(e.direction, Direction::Outgoing);
                    }
                    _ => unreachable!(),
                }
            }
            _ => panic!("expected MATCH"),
        }
    }

    // ===================================================================
    // Expression tests
    // ===================================================================

    #[test]
    fn test_nested_arithmetic() {
        let program =
            parse("MATCH (n) WHERE n.age + 5 > 30 RETURN n").unwrap();
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let cond = m.where_clause.as_ref().unwrap().condition.as_ref();
                match cond {
                    Expression::BinaryOp { op, left, .. } => {
                        assert_eq!(*op, BinaryOp::Gt);
                        // left should be n.age + 5
                        match left.as_ref() {
                            Expression::BinaryOp { op, .. } => {
                                assert_eq!(*op, BinaryOp::Add);
                            }
                            _ => panic!("expected Add"),
                        }
                    }
                    _ => panic!("expected BinaryOp"),
                }
            }
            _ => panic!("expected MATCH"),
        }
    }

    #[test]
    fn test_string_comparison() {
        let program =
            parse("MATCH (n) WHERE n.name = 'Alice' RETURN n").unwrap();
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let cond = m.where_clause.as_ref().unwrap().condition.as_ref();
                match cond {
                    Expression::BinaryOp { op, right, .. } => {
                        assert_eq!(*op, BinaryOp::Eq);
                        assert_eq!(
                            **right,
                            Expression::Literal(Literal::String("Alice".into()))
                        );
                    }
                    _ => panic!("expected BinaryOp"),
                }
            }
            _ => panic!("expected MATCH"),
        }
    }

    #[test]
    fn test_boolean_and_or_not() {
        let program = parse(
            "MATCH (n) WHERE NOT (n.age < 25 AND n.active = TRUE) RETURN n",
        )
        .unwrap();
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let cond = m.where_clause.as_ref().unwrap().condition.as_ref();
                match cond {
                    Expression::UnaryOp { op, operand } => {
                        assert_eq!(*op, UnaryOp::Not);
                        // operand is the parenthesized AND expression
                        match operand.as_ref() {
                            Expression::BinaryOp { op, .. } => {
                                assert_eq!(*op, BinaryOp::And);
                            }
                            _ => panic!("expected AND inside NOT"),
                        }
                    }
                    _ => panic!("expected UnaryOp NOT"),
                }
            }
            _ => panic!("expected MATCH"),
        }
    }

    #[test]
    fn test_is_null() {
        let program =
            parse("MATCH (n) WHERE n.email IS NULL RETURN n").unwrap();
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let cond = m.where_clause.as_ref().unwrap().condition.as_ref();
                match cond {
                    Expression::IsNull(inner) => {
                        assert_eq!(
                            **inner,
                            Expression::PropertyAccess {
                                object: Box::new(Expression::Identifier("n".into())),
                                property: "email".into(),
                            }
                        );
                    }
                    _ => panic!("expected IsNull"),
                }
            }
            _ => panic!("expected MATCH"),
        }
    }

    #[test]
    fn test_case_expression() {
        let program = parse(
            "MATCH (n) RETURN CASE WHEN n.age > 30 THEN 'old' ELSE 'young' END",
        )
        .unwrap();
        match &program.statements[1] {
            GqlStatement::Return(r) => {
                assert_eq!(r.items.len(), 1);
                match &r.items[0].expression {
                    Expression::Case {
                        operand,
                        when_clauses,
                        else_clause,
                    } => {
                        assert!(operand.is_none());
                        assert_eq!(when_clauses.len(), 1);
                        assert_eq!(
                            else_clause.as_deref(),
                            Some(&Expression::Literal(Literal::String("young".into())))
                        );
                    }
                    _ => panic!("expected Case expression"),
                }
            }
            _ => panic!("expected RETURN"),
        }
    }

    #[test]
    fn test_function_call() {
        let program = parse("MATCH (n) RETURN toString(n.age)").unwrap();
        match &program.statements[1] {
            GqlStatement::Return(r) => {
                assert_eq!(r.items.len(), 1);
                match &r.items[0].expression {
                    Expression::FunctionCall { name, args } => {
                        assert_eq!(name, "toString");
                        assert_eq!(args.len(), 1);
                        assert_eq!(
                            args[0],
                            Expression::PropertyAccess {
                                object: Box::new(Expression::Identifier("n".into())),
                                property: "age".into(),
                            }
                        );
                    }
                    _ => panic!("expected FunctionCall"),
                }
            }
            _ => panic!("expected RETURN"),
        }
    }

    #[test]
    fn test_nested_property_access() {
        let program = parse("MATCH (n) RETURN n.name").unwrap();
        match &program.statements[1] {
            GqlStatement::Return(r) => {
                assert_eq!(r.items.len(), 1);
                assert_eq!(
                    r.items[0].expression,
                    Expression::PropertyAccess {
                        object: Box::new(Expression::Identifier("n".into())),
                        property: "name".into(),
                    }
                );
            }
            _ => panic!("expected RETURN"),
        }
    }

    #[test]
    fn test_negative_number() {
        let program =
            parse("MATCH (n) WHERE n.balance > -100 RETURN n").unwrap();
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let cond = m.where_clause.as_ref().unwrap().condition.as_ref();
                match cond {
                    Expression::BinaryOp { op, right, .. } => {
                        assert_eq!(*op, BinaryOp::Gt);
                        assert_eq!(
                            **right,
                            Expression::UnaryOp {
                                op: UnaryOp::Neg,
                                operand: Box::new(Expression::Literal(
                                    Literal::Integer(100)
                                )),
                            }
                        );
                    }
                    _ => panic!("expected BinaryOp"),
                }
            }
            _ => panic!("expected MATCH"),
        }
    }

    #[test]
    fn test_float_literal_in_where() {
        let program =
            parse("MATCH (n) WHERE n.score > 3.14 RETURN n").unwrap();
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let cond = m.where_clause.as_ref().unwrap().condition.as_ref();
                match cond {
                    Expression::BinaryOp { op, right, .. } => {
                        assert_eq!(*op, BinaryOp::Gt);
                        assert_eq!(
                            **right,
                            Expression::Literal(Literal::Float(3.14))
                        );
                    }
                    _ => panic!("expected BinaryOp"),
                }
            }
            _ => panic!("expected MATCH"),
        }
    }

    #[test]
    fn test_parenthesized_expression() {
        let program = parse(
            "MATCH (n) WHERE (n.age > 20 AND n.age < 40) OR n.name = 'admin' RETURN n",
        )
        .unwrap();
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let cond = m.where_clause.as_ref().unwrap().condition.as_ref();
                // Top-level should be OR
                match cond {
                    Expression::BinaryOp { op, left, right } => {
                        assert_eq!(*op, BinaryOp::Or);
                        // left is the parenthesized AND
                        match left.as_ref() {
                            Expression::BinaryOp { op, .. } => {
                                assert_eq!(*op, BinaryOp::And);
                            }
                            _ => panic!("expected AND on left"),
                        }
                        // right is n.name = 'admin'
                        match right.as_ref() {
                            Expression::BinaryOp { op, .. } => {
                                assert_eq!(*op, BinaryOp::Eq);
                            }
                            _ => panic!("expected Eq on right"),
                        }
                    }
                    _ => panic!("expected OR"),
                }
            }
            _ => panic!("expected MATCH"),
        }
    }

    // ===================================================================
    // Statement tests
    // ===================================================================

    #[test]
    fn test_delete_statement() {
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

    #[test]
    fn test_set_statement() {
        let program =
            parse("MATCH (n:Person {name: 'Alice'}) SET n.age = 31").unwrap();
        assert_eq!(program.statements.len(), 2);
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
    fn test_remove_statement() {
        let program =
            parse("MATCH (n:Person) REMOVE n.email").unwrap();
        assert_eq!(program.statements.len(), 2);
        match &program.statements[1] {
            GqlStatement::Remove(r) => {
                assert_eq!(r.items.len(), 1);
                match &r.items[0] {
                    RemoveItem::Property { target, property } => {
                        assert_eq!(*target, Expression::Identifier("n".into()));
                        assert_eq!(property, "email");
                    }
                    _ => panic!("expected RemoveItem::Property"),
                }
            }
            _ => panic!("expected REMOVE"),
        }
    }

    #[test]
    fn test_return_distinct() {
        let program =
            parse("MATCH (n:Person) RETURN DISTINCT n.name").unwrap();
        assert_eq!(program.statements.len(), 2);
        match &program.statements[1] {
            GqlStatement::Return(r) => {
                assert!(r.distinct);
                assert_eq!(r.items.len(), 1);
                assert_eq!(
                    r.items[0].expression,
                    Expression::PropertyAccess {
                        object: Box::new(Expression::Identifier("n".into())),
                        property: "name".into(),
                    }
                );
            }
            _ => panic!("expected RETURN"),
        }
    }

    #[test]
    fn test_return_with_alias() {
        let program = parse(
            "MATCH (n:Person) RETURN n.name AS personName, n.age AS years",
        )
        .unwrap();
        assert_eq!(program.statements.len(), 2);
        match &program.statements[1] {
            GqlStatement::Return(r) => {
                assert_eq!(r.items.len(), 2);
                assert_eq!(r.items[0].alias.as_deref(), Some("personName"));
                assert_eq!(r.items[1].alias.as_deref(), Some("years"));
            }
            _ => panic!("expected RETURN"),
        }
    }

    #[test]
    fn test_order_by_desc() {
        let program = parse(
            "MATCH (n) RETURN n.name ORDER BY n.age DESC",
        )
        .unwrap();
        match &program.statements[1] {
            GqlStatement::Return(r) => {
                let ob = r.order_by.as_ref().expect("expected ORDER BY");
                assert_eq!(ob.items.len(), 1);
                assert_eq!(ob.items[0].direction, SortDirection::Desc);
            }
            _ => panic!("expected RETURN"),
        }
    }

    #[test]
    fn test_order_by_multiple() {
        let program = parse(
            "MATCH (n) RETURN n.name, n.age ORDER BY n.age ASC, n.name DESC",
        )
        .unwrap();
        match &program.statements[1] {
            GqlStatement::Return(r) => {
                let ob = r.order_by.as_ref().expect("expected ORDER BY");
                assert_eq!(ob.items.len(), 2);
                assert_eq!(ob.items[0].direction, SortDirection::Asc);
                assert_eq!(ob.items[1].direction, SortDirection::Desc);
            }
            _ => panic!("expected RETURN"),
        }
    }

    #[test]
    fn test_group_by() {
        let program = parse(
            "MATCH (n:Person) RETURN n.age, COUNT(n) GROUP BY n.age",
        )
        .unwrap();
        match &program.statements[1] {
            GqlStatement::Return(r) => {
                assert_eq!(r.items.len(), 2);
                let gb = r.group_by.as_ref().expect("expected GROUP BY");
                assert_eq!(gb.expressions.len(), 1);
                assert_eq!(
                    gb.expressions[0],
                    Expression::PropertyAccess {
                        object: Box::new(Expression::Identifier("n".into())),
                        property: "age".into(),
                    }
                );
            }
            _ => panic!("expected RETURN"),
        }
    }

    #[test]
    fn test_create_graph_if_not_exists() {
        let program =
            parse("CREATE GRAPH IF NOT EXISTS myGraph").unwrap();
        assert_eq!(program.statements.len(), 1);
        match &program.statements[0] {
            GqlStatement::CreateGraph(cg) => {
                assert_eq!(cg.name, "myGraph");
                assert!(cg.if_not_exists);
                assert!(cg.graph_type.is_none());
            }
            _ => panic!("expected CREATE GRAPH"),
        }
    }

    #[test]
    fn test_drop_graph_if_exists() {
        let program = parse("DROP GRAPH IF EXISTS myGraph").unwrap();
        assert_eq!(program.statements.len(), 1);
        match &program.statements[0] {
            GqlStatement::DropGraph(dg) => {
                assert_eq!(dg.name, "myGraph");
                assert!(dg.if_exists);
            }
            _ => panic!("expected DROP GRAPH"),
        }
    }

    // ===================================================================
    // Edge case tests
    // ===================================================================

    #[test]
    fn test_empty_input() {
        let program = parse("").unwrap();
        assert!(program.statements.is_empty());
    }

    #[test]
    fn test_semicolon_separated() {
        let program = parse(
            "MATCH (n) RETURN n; MATCH (m) RETURN m",
        )
        .unwrap();
        // Each MATCH + RETURN pair is 2 statements, with semicolon separator
        assert_eq!(program.statements.len(), 4);
        assert!(matches!(&program.statements[0], GqlStatement::Match(_)));
        assert!(matches!(&program.statements[1], GqlStatement::Return(_)));
        assert!(matches!(&program.statements[2], GqlStatement::Match(_)));
        assert!(matches!(&program.statements[3], GqlStatement::Return(_)));
    }

    #[test]
    fn test_unicode_string_literal() {
        let program =
            parse("MATCH (n) WHERE n.name = '日本語' RETURN n").unwrap();
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let cond = m.where_clause.as_ref().unwrap().condition.as_ref();
                match cond {
                    Expression::BinaryOp { right, .. } => {
                        assert_eq!(
                            **right,
                            Expression::Literal(Literal::String("日本語".into()))
                        );
                    }
                    _ => panic!("expected BinaryOp"),
                }
            }
            _ => panic!("expected MATCH"),
        }
    }

    #[test]
    fn test_escaped_quotes_in_string() {
        let program =
            parse("MATCH (n) WHERE n.name = 'O''Brien' RETURN n").unwrap();
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                let cond = m.where_clause.as_ref().unwrap().condition.as_ref();
                match cond {
                    Expression::BinaryOp { right, .. } => {
                        assert_eq!(
                            **right,
                            Expression::Literal(Literal::String("O'Brien".into()))
                        );
                    }
                    _ => panic!("expected BinaryOp"),
                }
            }
            _ => panic!("expected MATCH"),
        }
    }

    #[test]
    fn test_chained_comparison_rejected() {
        let result = parse("MATCH (n) WHERE n.a < 1 < 2 RETURN n");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("chained"),
            "expected 'chained' in error message, got: {}",
            err.message
        );
    }

    #[test]
    fn test_single_comparison_still_works() {
        let program = parse("MATCH (n) WHERE n.a < 1 RETURN n");
        assert!(program.is_ok());
    }

    #[test]
    fn test_deep_not_nesting_rejected() {
        let nots = "NOT ".repeat(300);
        let query = format!("MATCH (n) WHERE {}n.a RETURN n", nots);
        let result = parse(&query);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("depth"),
            "expected 'depth' in error message, got: {}",
            err.message
        );
    }

    #[test]
    fn test_quantifier_min_greater_than_max() {
        let result = parse("MATCH (a)-[r]{5,2}->(b) RETURN a");
        assert!(result.is_err());
    }

    #[test]
    fn test_direction_outgoing() {
        let program = parse("MATCH (a)-[r]->(b) RETURN a").unwrap();
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                match &m.pattern.paths[0].elements[1] {
                    PatternElement::Edge(e) => {
                        assert_eq!(e.direction, Direction::Outgoing);
                    }
                    _ => panic!("expected edge"),
                }
            }
            _ => panic!("expected MATCH"),
        }
    }

    #[test]
    fn test_direction_incoming() {
        let program = parse("MATCH (a)<-[r]-(b) RETURN a").unwrap();
        match &program.statements[0] {
            GqlStatement::Match(m) => {
                match &m.pattern.paths[0].elements[1] {
                    PatternElement::Edge(e) => {
                        assert_eq!(e.direction, Direction::Incoming);
                    }
                    _ => panic!("expected edge"),
                }
            }
            _ => panic!("expected MATCH"),
        }
    }
}