use std::fmt;

// ---------------------------------------------------------------------------
// Span & error types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpannedToken {
    pub token: Token,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LexerError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for LexerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Lexer error at {}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for LexerError {}

// ---------------------------------------------------------------------------
// Token
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    Match, Where, Return, Create, Insert, Delete, Set, Remove, Drop,
    Graph, Type, Node, Edge,
    True, False, Null,
    Not, And, Or, Xor, Is, In, Like,
    As, By, Order, Asc, Desc, Limit, Offset,
    Group, Having, With, Let, For, Filter, Call, Yield,
    Optional, Mandatory, Exists,
    Case, When, Then, Else, End,
    Union, Intersect, Except, All, Distinct,
    Count, Sum, Avg, Min, Max, Collect,
    Walk, Trail, Simple, Acyclic, Shortest, Path, Any,
    Commit, Rollback, Begin, If, Detach, Nodetach,
    Cost, Of, To, From,
    Index, Unique, On,

    // Literals
    IntegerLit(i64),
    FloatLit(f64),
    StringLit(String),
    BoolLit(bool),
    NullLit,

    // Identifiers & parameters
    Ident(String),
    Parameter(String),

    // Operators
    Plus, Minus, Star, Slash, Percent,
    Eq, Neq, Lt, Gt, Le, Ge,
    DoublePipe, Dot, DoubleDot,
    Arrow, LeftArrow, Tilde,

    // Delimiters
    LParen, RParen, LBracket, RBracket, LBrace, RBrace,
    Comma, Colon, Semicolon, At,

    // Special
    Eof,
    Comment(String),
}

// ---------------------------------------------------------------------------
// Keyword lookup
// ---------------------------------------------------------------------------

fn keyword_token(word: &str) -> Option<Token> {
    match word.to_ascii_uppercase().as_str() {
        "MATCH" => Some(Token::Match),
        "WHERE" => Some(Token::Where),
        "RETURN" => Some(Token::Return),
        "CREATE" => Some(Token::Create),
        "INSERT" => Some(Token::Insert),
        "DELETE" => Some(Token::Delete),
        "SET" => Some(Token::Set),
        "REMOVE" => Some(Token::Remove),
        "DROP" => Some(Token::Drop),
        "GRAPH" => Some(Token::Graph),
        "TYPE" => Some(Token::Type),
        "NODE" => Some(Token::Node),
        "EDGE" => Some(Token::Edge),
        "TRUE" => Some(Token::True),
        "FALSE" => Some(Token::False),
        "NULL" => Some(Token::Null),
        "NOT" => Some(Token::Not),
        "AND" => Some(Token::And),
        "OR" => Some(Token::Or),
        "XOR" => Some(Token::Xor),
        "IS" => Some(Token::Is),
        "IN" => Some(Token::In),
        "LIKE" => Some(Token::Like),
        "AS" => Some(Token::As),
        "BY" => Some(Token::By),
        "ORDER" => Some(Token::Order),
        "ASC" => Some(Token::Asc),
        "DESC" => Some(Token::Desc),
        "LIMIT" => Some(Token::Limit),
        "OFFSET" => Some(Token::Offset),
        "GROUP" => Some(Token::Group),
        "HAVING" => Some(Token::Having),
        "WITH" => Some(Token::With),
        "LET" => Some(Token::Let),
        "FOR" => Some(Token::For),
        "FILTER" => Some(Token::Filter),
        "CALL" => Some(Token::Call),
        "YIELD" => Some(Token::Yield),
        "OPTIONAL" => Some(Token::Optional),
        "MANDATORY" => Some(Token::Mandatory),
        "EXISTS" => Some(Token::Exists),
        "CASE" => Some(Token::Case),
        "WHEN" => Some(Token::When),
        "THEN" => Some(Token::Then),
        "ELSE" => Some(Token::Else),
        "END" => Some(Token::End),
        "UNION" => Some(Token::Union),
        "INTERSECT" => Some(Token::Intersect),
        "EXCEPT" => Some(Token::Except),
        "ALL" => Some(Token::All),
        "DISTINCT" => Some(Token::Distinct),
        "COUNT" => Some(Token::Count),
        "SUM" => Some(Token::Sum),
        "AVG" => Some(Token::Avg),
        "MIN" => Some(Token::Min),
        "MAX" => Some(Token::Max),
        "COLLECT" => Some(Token::Collect),
        "WALK" => Some(Token::Walk),
        "TRAIL" => Some(Token::Trail),
        "SIMPLE" => Some(Token::Simple),
        "ACYCLIC" => Some(Token::Acyclic),
        "SHORTEST" => Some(Token::Shortest),
        "PATH" => Some(Token::Path),
        "ANY" => Some(Token::Any),
        "COMMIT" => Some(Token::Commit),
        "ROLLBACK" => Some(Token::Rollback),
        "BEGIN" => Some(Token::Begin),
        "IF" => Some(Token::If),
        "DETACH" => Some(Token::Detach),
        "NODETACH" => Some(Token::Nodetach),
        "COST" => Some(Token::Cost),
        "OF" => Some(Token::Of),
        "TO" => Some(Token::To),
        "FROM" => Some(Token::From),
        "INDEX" => Some(Token::Index),
        "UNIQUE" => Some(Token::Unique),
        "ON" => Some(Token::On),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

pub struct Lexer {
    input: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
}

impl Lexer {
    pub fn new(input: &str) -> Lexer {
        Lexer {
            input: input.chars().collect(),
            pos: 0,
            line: 1,
            column: 1,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<SpannedToken>, LexerError> {
        let mut tokens = Vec::new();
        loop {
            let st = self.next_token()?;
            let is_eof = st.token == Token::Eof;
            tokens.push(st);
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }

    pub fn next_token(&mut self) -> Result<SpannedToken, LexerError> {
        self.skip_whitespace();

        let span = self.current_span();

        if self.is_eof() {
            return Ok(SpannedToken { token: Token::Eof, span });
        }

        let ch = self.peek();

        // Single-line comment --
        if ch == '-' && self.peek_at(1) == Some('-') {
            return self.lex_single_line_comment(span);
        }

        // Multi-line comment /* */
        if ch == '/' && self.peek_at(1) == Some('*') {
            return self.lex_multi_line_comment(span);
        }

        // String literal
        if ch == '\'' {
            return self.lex_string(span);
        }

        // Number literal
        if ch.is_ascii_digit() {
            return self.lex_number(span);
        }

        // Identifier / keyword
        if ch.is_ascii_alphabetic() || ch == '_' {
            return self.lex_identifier(span);
        }

        // Parameter $name
        if ch == '$' {
            return self.lex_parameter(span);
        }

        // Operators & delimiters
        self.lex_operator_or_delimiter(span)
    }

    // -- helpers -------------------------------------------------------------

    fn is_eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn peek(&self) -> char {
        if self.pos < self.input.len() {
            self.input[self.pos]
        } else {
            '\0'
        }
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.input.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> char {
        if self.pos >= self.input.len() {
            return '\0';
        }
        let ch = self.input[self.pos];
        self.pos += 1;
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        ch
    }

    fn current_span(&self) -> Span {
        Span { line: self.line, column: self.column, offset: self.pos }
    }

    fn error(&self, msg: impl Into<String>) -> LexerError {
        LexerError { message: msg.into(), line: self.line, column: self.column }
    }

    fn skip_whitespace(&mut self) {
        while !self.is_eof() && self.peek().is_ascii_whitespace() {
            self.advance();
        }
    }

    // -- lexing routines -----------------------------------------------------

    fn lex_single_line_comment(&mut self, span: Span) -> Result<SpannedToken, LexerError> {
        self.advance(); // -
        self.advance(); // -
        let mut text = String::new();
        while !self.is_eof() && self.peek() != '\n' {
            text.push(self.advance());
        }
        Ok(SpannedToken { token: Token::Comment(text), span })
    }

    fn lex_multi_line_comment(&mut self, span: Span) -> Result<SpannedToken, LexerError> {
        self.advance(); // /
        self.advance(); // *
        let mut text = String::new();
        loop {
            if self.is_eof() {
                return Err(LexerError {
                    message: "unterminated multi-line comment".into(),
                    line: span.line,
                    column: span.column,
                });
            }
            if self.peek() == '*' && self.peek_at(1) == Some('/') {
                self.advance(); // *
                self.advance(); // /
                break;
            }
            text.push(self.advance());
        }
        Ok(SpannedToken { token: Token::Comment(text), span })
    }

    fn lex_string(&mut self, span: Span) -> Result<SpannedToken, LexerError> {
        self.advance(); // opening '
        let mut s = String::new();
        loop {
            if self.is_eof() {
                return Err(LexerError {
                    message: "unterminated string literal".into(),
                    line: span.line,
                    column: span.column,
                });
            }
            let ch = self.advance();
            if ch == '\'' {
                // '' escape
                if !self.is_eof() && self.peek() == '\'' {
                    self.advance();
                    s.push('\'');
                } else {
                    break;
                }
            } else {
                s.push(ch);
            }
        }
        Ok(SpannedToken { token: Token::StringLit(s), span })
    }

    fn lex_number(&mut self, span: Span) -> Result<SpannedToken, LexerError> {
        let start = self.pos;
        // integer part
        while !self.is_eof() && self.peek().is_ascii_digit() {
            self.advance();
        }
        let mut is_float = false;
        // fractional part
        if !self.is_eof() && self.peek() == '.' && self.peek_at(1).map_or(false, |c| c.is_ascii_digit()) {
            is_float = true;
            self.advance(); // .
            while !self.is_eof() && self.peek().is_ascii_digit() {
                self.advance();
            }
        }
        // exponent part
        if !self.is_eof() && (self.peek() == 'e' || self.peek() == 'E') {
            is_float = true;
            self.advance(); // e/E
            if !self.is_eof() && (self.peek() == '+' || self.peek() == '-') {
                self.advance();
            }
            if self.is_eof() || !self.peek().is_ascii_digit() {
                return Err(self.error("invalid numeric literal: expected digit after exponent"));
            }
            while !self.is_eof() && self.peek().is_ascii_digit() {
                self.advance();
            }
        }
        let text: String = self.input[start..self.pos].iter().collect();
        if is_float {
            let val: f64 = text.parse().map_err(|_| self.error(format!("invalid float literal: {text}")))?;
            Ok(SpannedToken { token: Token::FloatLit(val), span })
        } else {
            let val: i64 = text.parse().map_err(|_| self.error(format!("invalid integer literal: {text}")))?;
            Ok(SpannedToken { token: Token::IntegerLit(val), span })
        }
    }

    fn lex_identifier(&mut self, span: Span) -> Result<SpannedToken, LexerError> {
        let start = self.pos;
        while !self.is_eof() && (self.peek().is_ascii_alphanumeric() || self.peek() == '_') {
            self.advance();
        }
        let word: String = self.input[start..self.pos].iter().collect();
        let token = keyword_token(&word).unwrap_or(Token::Ident(word));
        Ok(SpannedToken { token, span })
    }

    fn lex_parameter(&mut self, span: Span) -> Result<SpannedToken, LexerError> {
        self.advance(); // $
        if self.is_eof() || !(self.peek().is_ascii_alphabetic() || self.peek() == '_') {
            return Err(self.error("expected parameter name after '$'"));
        }
        let start = self.pos;
        while !self.is_eof() && (self.peek().is_ascii_alphanumeric() || self.peek() == '_') {
            self.advance();
        }
        let name: String = self.input[start..self.pos].iter().collect();
        Ok(SpannedToken { token: Token::Parameter(name), span })
    }

    fn lex_operator_or_delimiter(&mut self, span: Span) -> Result<SpannedToken, LexerError> {
        let ch = self.advance();
        let token = match ch {
            '+' => Token::Plus,
            '*' => Token::Star,
            '/' => Token::Slash,
            '%' => Token::Percent,
            '~' => Token::Tilde,
            '(' => Token::LParen,
            ')' => Token::RParen,
            '[' => Token::LBracket,
            ']' => Token::RBracket,
            '{' => Token::LBrace,
            '}' => Token::RBrace,
            ',' => Token::Comma,
            ':' => Token::Colon,
            ';' => Token::Semicolon,
            '@' => Token::At,
            '=' => Token::Eq,
            '.' => {
                if !self.is_eof() && self.peek() == '.' {
                    self.advance();
                    Token::DoubleDot
                } else {
                    Token::Dot
                }
            }
            '|' => {
                if !self.is_eof() && self.peek() == '|' {
                    self.advance();
                    Token::DoublePipe
                } else {
                    return Err(LexerError {
                        message: "unexpected character '|'; did you mean '||'?".into(),
                        line: span.line,
                        column: span.column,
                    });
                }
            }
            '!' => {
                if !self.is_eof() && self.peek() == '=' {
                    self.advance();
                    Token::Neq
                } else {
                    return Err(LexerError {
                        message: "unexpected character '!'; did you mean '!='?".into(),
                        line: span.line,
                        column: span.column,
                    });
                }
            }
            '<' => {
                if !self.is_eof() && self.peek() == '=' {
                    self.advance();
                    Token::Le
                } else if !self.is_eof() && self.peek() == '>' {
                    self.advance();
                    Token::Neq
                } else if !self.is_eof() && self.peek() == '-' {
                    self.advance();
                    Token::LeftArrow
                } else {
                    Token::Lt
                }
            }
            '>' => {
                if !self.is_eof() && self.peek() == '=' {
                    self.advance();
                    Token::Ge
                } else {
                    Token::Gt
                }
            }
            '-' => {
                if !self.is_eof() && self.peek() == '>' {
                    self.advance();
                    Token::Arrow
                } else {
                    Token::Minus
                }
            }
            _ => {
                return Err(LexerError {
                    message: format!("unexpected character '{ch}'"),
                    line: span.line,
                    column: span.column,
                });
            }
        };
        Ok(SpannedToken { token, span })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: &str) -> Vec<Token> {
        Lexer::new(input)
            .tokenize()
            .unwrap()
            .into_iter()
            .map(|st| st.token)
            .collect()
    }

    fn tokens_no_eof(input: &str) -> Vec<Token> {
        let mut v = tokens(input);
        if v.last() == Some(&Token::Eof) {
            v.pop();
        }
        v
    }

    #[test]
    fn test_simple_match_return() {
        let toks = tokens_no_eof("MATCH (n:Person) WHERE n.age > 30 RETURN n.name");
        assert_eq!(toks, vec![
            Token::Match,
            Token::LParen,
            Token::Ident("n".into()),
            Token::Colon,
            Token::Ident("Person".into()),
            Token::RParen,
            Token::Where,
            Token::Ident("n".into()),
            Token::Dot,
            Token::Ident("age".into()),
            Token::Gt,
            Token::IntegerLit(30),
            Token::Return,
            Token::Ident("n".into()),
            Token::Dot,
            Token::Ident("name".into()),
        ]);
    }

    #[test]
    fn test_case_insensitive_keywords() {
        let toks = tokens_no_eof("match Where ReTuRn");
        assert_eq!(toks, vec![Token::Match, Token::Where, Token::Return]);
    }

    #[test]
    fn test_integer_literals() {
        assert_eq!(tokens_no_eof("0 42 1000"), vec![
            Token::IntegerLit(0),
            Token::IntegerLit(42),
            Token::IntegerLit(1000),
        ]);
    }

    #[test]
    fn test_float_literals() {
        assert_eq!(tokens_no_eof("3.14"), vec![Token::FloatLit(3.14)]);
        assert_eq!(tokens_no_eof("0.5"), vec![Token::FloatLit(0.5)]);
    }

    #[test]
    fn test_scientific_notation() {
        let toks = tokens_no_eof("1.5e10 2E3 3.0e-2");
        assert_eq!(toks.len(), 3);
        assert_eq!(toks[0], Token::FloatLit(1.5e10));
        assert_eq!(toks[1], Token::FloatLit(2e3));
        assert_eq!(toks[2], Token::FloatLit(3.0e-2));
    }

    #[test]
    fn test_string_literals() {
        assert_eq!(tokens_no_eof("'hello'"), vec![Token::StringLit("hello".into())]);
        // '' escape for literal quote
        assert_eq!(tokens_no_eof("'it''s'"), vec![Token::StringLit("it's".into())]);
        assert_eq!(tokens_no_eof("''"), vec![Token::StringLit(String::new())]);
    }

    #[test]
    fn test_all_operators() {
        let toks = tokens_no_eof("+ - * / % = != <> < > <= >= || . .. -> <- ~");
        assert_eq!(toks, vec![
            Token::Plus, Token::Minus, Token::Star, Token::Slash, Token::Percent,
            Token::Eq, Token::Neq, Token::Neq,
            Token::Lt, Token::Gt, Token::Le, Token::Ge,
            Token::DoublePipe, Token::Dot, Token::DoubleDot,
            Token::Arrow, Token::LeftArrow, Token::Tilde,
        ]);
    }

    #[test]
    fn test_delimiters() {
        let toks = tokens_no_eof("( ) [ ] { } , : ; @");
        assert_eq!(toks, vec![
            Token::LParen, Token::RParen,
            Token::LBracket, Token::RBracket,
            Token::LBrace, Token::RBrace,
            Token::Comma, Token::Colon, Token::Semicolon, Token::At,
        ]);
    }

    #[test]
    fn test_single_line_comment() {
        let toks = tokens_no_eof("MATCH -- this is a comment\nRETURN");
        assert_eq!(toks, vec![
            Token::Match,
            Token::Comment(" this is a comment".into()),
            Token::Return,
        ]);
    }

    #[test]
    fn test_multi_line_comment() {
        let toks = tokens_no_eof("MATCH /* a\nmultiline\ncomment */ RETURN");
        assert_eq!(toks, vec![
            Token::Match,
            Token::Comment(" a\nmultiline\ncomment ".into()),
            Token::Return,
        ]);
    }

    #[test]
    fn test_parameter() {
        let toks = tokens_no_eof("$name $age_1");
        assert_eq!(toks, vec![
            Token::Parameter("name".into()),
            Token::Parameter("age_1".into()),
        ]);
    }

    #[test]
    fn test_span_tracking() {
        let spanned = Lexer::new("MATCH\n  (n)").tokenize().unwrap();
        // MATCH at line 1 col 1
        assert_eq!(spanned[0].span, Span { line: 1, column: 1, offset: 0 });
        // ( at line 2 col 3
        assert_eq!(spanned[1].span, Span { line: 2, column: 3, offset: 8 });
    }

    #[test]
    fn test_unterminated_string_error() {
        let err = Lexer::new("'oops").tokenize().unwrap_err();
        assert!(err.message.contains("unterminated"));
    }

    #[test]
    fn test_eof_token() {
        let toks = tokens("");
        assert_eq!(toks, vec![Token::Eof]);
    }
}