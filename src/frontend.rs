use crate::son::{BinaryOp, CompareOp, Graph, NodeId, UnaryOp};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

/// Parses the explicitly supported, single-basic-block i64 Rust subset.
///
/// Functions may omit `-> i64`, but must contain a final `return expr;`.
/// Each function is emitted as a `Start`/`Return` component in source order;
/// the current `Graph` has no function-name or parameter-name metadata.
/// Comparisons produce the integer values zero or one and cannot be chained.
/// Unsupported Rust syntax and semantics are rejected with a source location.
pub fn parse(source: &str) -> Result<Graph, ParseError> {
    let tokens = Lexer::new(source).tokenize()?;
    Parser::new(tokens).parse_program()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub offset: usize,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl ParseError {
    fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            offset: span.start,
            line: span.line,
            column: span.column,
            message: message.into(),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl Error for ParseError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    start: usize,
    end: usize,
    line: usize,
    column: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TokenKind {
    Fn,
    Let,
    Return,
    Ident(String),
    Integer(u64),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Semicolon,
    Arrow,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Eof,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Token {
    kind: TokenKind,
    span: Span,
}

struct Lexer<'a> {
    source: &'a str,
    offset: usize,
    line: usize,
    column: usize,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            offset: 0,
            line: 1,
            column: 1,
        }
    }

    fn tokenize(mut self) -> Result<Vec<Token>, ParseError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_trivia()?;
            let span = self.span();
            if self.offset == self.source.len() {
                tokens.push(Token {
                    kind: TokenKind::Eof,
                    span,
                });
                return Ok(tokens);
            }

            let ch = self.current_char().expect("offset is in bounds");
            let kind = if is_ident_start(ch) {
                self.lex_identifier()
            } else if ch.is_ascii_digit() {
                self.lex_integer()?
            } else {
                self.lex_punctuation()?
            };
            tokens.push(Token {
                kind,
                span: Span {
                    end: self.offset,
                    ..span
                },
            });
        }
    }

    fn span(&self) -> Span {
        Span {
            start: self.offset,
            end: self.offset,
            line: self.line,
            column: self.column,
        }
    }

    fn current_char(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }

    fn rest(&self) -> &'a str {
        &self.source[self.offset..]
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.current_char()?;
        self.offset += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    fn skip_trivia(&mut self) -> Result<(), ParseError> {
        loop {
            while self.current_char().is_some_and(char::is_whitespace) {
                self.advance();
            }
            if self.rest().starts_with("//") {
                while self.advance().is_some_and(|ch| ch != '\n') {}
                continue;
            }
            if self.rest().starts_with("/*") {
                let start = self.span();
                self.advance();
                self.advance();
                let mut depth = 1usize;
                while depth != 0 {
                    if self.offset == self.source.len() {
                        return Err(ParseError::new(start, "unterminated block comment"));
                    }
                    if self.rest().starts_with("/*") {
                        self.advance();
                        self.advance();
                        depth = depth.checked_add(1).ok_or_else(|| {
                            ParseError::new(start, "block comment nesting is too deep")
                        })?;
                    } else if self.rest().starts_with("*/") {
                        self.advance();
                        self.advance();
                        depth -= 1;
                    } else {
                        self.advance();
                    }
                }
                continue;
            }
            return Ok(());
        }
    }

    fn lex_identifier(&mut self) -> TokenKind {
        let start = self.offset;
        self.advance();
        while self.current_char().is_some_and(is_ident_continue) {
            self.advance();
        }
        match &self.source[start..self.offset] {
            "fn" => TokenKind::Fn,
            "let" => TokenKind::Let,
            "return" => TokenKind::Return,
            identifier => TokenKind::Ident(identifier.to_owned()),
        }
    }

    fn lex_integer(&mut self) -> Result<TokenKind, ParseError> {
        let start = self.span();
        while self
            .current_char()
            .is_some_and(|ch| ch.is_ascii_digit() || ch == '_')
        {
            self.advance();
        }
        let digits_end = self.offset;
        while self.current_char().is_some_and(is_ident_continue) {
            self.advance();
        }
        let suffix = &self.source[digits_end..self.offset];
        if !suffix.is_empty() && suffix != "i64" {
            return Err(ParseError::new(
                Span {
                    end: self.offset,
                    ..start
                },
                format!(
                    "unsupported integer literal suffix `{suffix}`; only decimal i64 literals are supported"
                ),
            ));
        }
        let digits: String = self.source[start.start..digits_end]
            .chars()
            .filter(|ch| *ch != '_')
            .collect();
        let value = digits.parse::<u64>().map_err(|_| {
            ParseError::new(
                Span {
                    end: self.offset,
                    ..start
                },
                "integer literal does not fit in the supported i64 magnitude range",
            )
        })?;
        Ok(TokenKind::Integer(value))
    }

    fn lex_punctuation(&mut self) -> Result<TokenKind, ParseError> {
        let start = self.span();
        self.advance();
        let simple = match self.source[start.start..self.offset].chars().next() {
            Some('(') => TokenKind::LParen,
            Some(')') => TokenKind::RParen,
            Some('{') => TokenKind::LBrace,
            Some('}') => TokenKind::RBrace,
            Some(',') => TokenKind::Comma,
            Some(':') => TokenKind::Colon,
            Some(';') => TokenKind::Semicolon,
            Some('+') => TokenKind::Plus,
            Some('-') => {
                if self.rest().starts_with('>') {
                    self.advance();
                    TokenKind::Arrow
                } else {
                    TokenKind::Minus
                }
            }
            Some('*') => TokenKind::Star,
            Some('/') => TokenKind::Slash,
            Some('%') => TokenKind::Percent,
            Some('=') => {
                if self.rest().starts_with('=') {
                    self.advance();
                    TokenKind::EqEq
                } else {
                    TokenKind::Eq
                }
            }
            Some('!') => {
                if self.rest().starts_with('=') {
                    self.advance();
                    TokenKind::Ne
                } else {
                    return Err(ParseError::new(start, "unary `!` is not supported"));
                }
            }
            Some('<') => {
                if self.rest().starts_with('=') {
                    self.advance();
                    TokenKind::Le
                } else {
                    TokenKind::Lt
                }
            }
            Some('>') => {
                if self.rest().starts_with('=') {
                    self.advance();
                    TokenKind::Ge
                } else {
                    TokenKind::Gt
                }
            }
            Some('.') => {
                return Err(ParseError::new(
                    start,
                    "floating-point literals and member access are not supported",
                ));
            }
            Some('"' | '\'') => {
                return Err(ParseError::new(
                    start,
                    "string, character, and lifetime syntax is not supported",
                ));
            }
            Some(ch) if !ch.is_ascii() => {
                return Err(ParseError::new(
                    start,
                    format!("non-ASCII identifier or token `{ch}` is not supported"),
                ));
            }
            Some(ch) => {
                return Err(ParseError::new(start, format!("unsupported token `{ch}`")));
            }
            None => unreachable!("punctuation requires a character"),
        };
        Ok(simple)
    }
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_ident_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn is_reserved_identifier(identifier: &str) -> bool {
    matches!(
        identifier,
        "abstract"
            | "as"
            | "async"
            | "await"
            | "become"
            | "box"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "do"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "final"
            | "for"
            | "i64"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "macro"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "override"
            | "priv"
            | "pub"
            | "ref"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "try"
            | "type"
            | "typeof"
            | "union"
            | "unsafe"
            | "unsized"
            | "use"
            | "virtual"
            | "where"
            | "while"
            | "yield"
    )
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
    graph: Graph,
    functions: HashSet<String>,
    variables: HashMap<String, NodeId>,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            position: 0,
            graph: Graph::new(),
            functions: HashSet::new(),
            variables: HashMap::new(),
        }
    }

    fn parse_program(mut self) -> Result<Graph, ParseError> {
        if self.check(&TokenKind::Eof) {
            return Err(self.error_here("expected at least one function"));
        }
        while !self.check(&TokenKind::Eof) {
            if !self.check(&TokenKind::Fn) {
                return Err(self.error_here("expected `fn`; this item is not supported"));
            }
            self.parse_function()?;
        }
        debug_assert!(
            self.graph.validate().is_ok(),
            "frontend emitted an invalid graph"
        );
        Ok(self.graph)
    }

    fn parse_function(&mut self) -> Result<(), ParseError> {
        self.expect(TokenKind::Fn, "`fn`")?;
        let (name, name_span) = self.parse_identifier("function name")?;
        if !self.functions.insert(name.clone()) {
            return Err(ParseError::new(
                name_span,
                format!("duplicate function `{name}`"),
            ));
        }

        self.expect(TokenKind::LParen, "`(` after function name")?;
        let mut parameters = Vec::new();
        let mut parameter_names = HashSet::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                if self.peek_identifier("mut") {
                    return Err(self.error_here("mutable parameters are not supported"));
                }
                let (parameter, parameter_span) = self.parse_identifier("parameter name")?;
                if !parameter_names.insert(parameter.clone()) {
                    return Err(ParseError::new(
                        parameter_span,
                        format!("duplicate parameter `{parameter}`"),
                    ));
                }
                self.expect(TokenKind::Colon, "`:` and explicit `i64` parameter type")?;
                self.parse_i64_type()?;
                parameters.push(parameter);
                if !self.eat(TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RParen) {
                    break;
                }
            }
        }
        self.expect(TokenKind::RParen, "`)` after parameters")?;
        if self.eat(TokenKind::Arrow) {
            self.parse_i64_type()?;
        }

        self.expect(TokenKind::LBrace, "`{` before function body")?;
        let start = self.graph.start();
        self.variables.clear();
        for (index, parameter) in parameters.iter().enumerate() {
            let node = self.graph.parameter(index);
            self.variables.insert(parameter.clone(), node);
        }

        let mut returned = false;
        while !self.check(&TokenKind::RBrace) {
            if self.check(&TokenKind::Eof) {
                return Err(self.error_here(format!("unterminated body of function `{name}`")));
            }
            if returned {
                return Err(self.error_here(
                    "statements after `return` are unsupported; `return` must end the body",
                ));
            }
            if self.eat(TokenKind::Let) {
                self.parse_let()?;
            } else if self.eat(TokenKind::Return) {
                let value = self.parse_expression()?;
                self.expect(TokenKind::Semicolon, "`;` after returned expression")?;
                self.graph.return_(start, Some(value));
                returned = true;
            } else {
                return Err(
                    self.error_here("only `let` bindings and a final `return expr;` are supported")
                );
            }
        }
        let close = self.expect(TokenKind::RBrace, "`}` after function body")?;
        if !returned {
            return Err(ParseError::new(
                close,
                format!("function `{name}` must end with `return expr;`"),
            ));
        }
        Ok(())
    }

    fn parse_let(&mut self) -> Result<(), ParseError> {
        if self.peek_identifier("mut") {
            return Err(self.error_here("mutable bindings are not supported"));
        }
        let (name, name_span) = self.parse_identifier("binding name")?;
        if self.variables.contains_key(&name) {
            return Err(ParseError::new(
                name_span,
                format!("duplicate binding or parameter `{name}`; shadowing is not supported"),
            ));
        }
        if self.eat(TokenKind::Colon) {
            self.parse_i64_type()?;
        }
        self.expect(TokenKind::Eq, "`=` in let binding")?;
        let value = self.parse_expression()?;
        self.expect(TokenKind::Semicolon, "`;` after let binding")?;
        self.variables.insert(name, value);
        Ok(())
    }

    fn parse_i64_type(&mut self) -> Result<(), ParseError> {
        if self.peek_identifier("i64") {
            self.position += 1;
            Ok(())
        } else {
            Err(self.error_here("only the `i64` type is supported"))
        }
    }

    fn parse_identifier(&mut self, description: &str) -> Result<(String, Span), ParseError> {
        let token = self.advance();
        match token.kind {
            TokenKind::Ident(identifier) if identifier == "_" => Err(ParseError::new(
                token.span,
                format!("`_` cannot be used as a {description}"),
            )),
            TokenKind::Ident(identifier) if is_reserved_identifier(&identifier) => {
                Err(ParseError::new(
                    token.span,
                    format!("reserved keyword `{identifier}` cannot be used as a {description}"),
                ))
            }
            TokenKind::Ident(identifier) => Ok((identifier, token.span)),
            _ => Err(ParseError::new(
                token.span,
                format!("expected {description}"),
            )),
        }
    }

    fn parse_expression(&mut self) -> Result<NodeId, ParseError> {
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<NodeId, ParseError> {
        let left = self.parse_additive()?;
        let operation = match self.peek().kind {
            TokenKind::EqEq => Some(CompareOp::Eq),
            TokenKind::Ne => Some(CompareOp::Ne),
            TokenKind::Lt => Some(CompareOp::Lt),
            TokenKind::Le => Some(CompareOp::Le),
            TokenKind::Gt => Some(CompareOp::Gt),
            TokenKind::Ge => Some(CompareOp::Ge),
            _ => None,
        };
        let Some(operation) = operation else {
            return Ok(left);
        };
        self.position += 1;
        let right = self.parse_additive()?;
        if self.is_comparison(self.peek()) {
            return Err(self.error_here("chained comparisons are not supported"));
        }
        Ok(self.graph.compare(operation, left, right))
    }

    fn parse_additive(&mut self) -> Result<NodeId, ParseError> {
        let mut value = self.parse_multiplicative()?;
        loop {
            let operation = match self.peek().kind {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => return Ok(value),
            };
            self.position += 1;
            let right = self.parse_multiplicative()?;
            value = self.graph.binary(operation, value, right);
        }
    }

    fn parse_multiplicative(&mut self) -> Result<NodeId, ParseError> {
        let mut value = self.parse_unary()?;
        loop {
            let operation = match self.peek().kind {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Rem,
                _ => return Ok(value),
            };
            self.position += 1;
            let right = self.parse_unary()?;
            value = self.graph.binary(operation, value, right);
        }
    }

    fn parse_unary(&mut self) -> Result<NodeId, ParseError> {
        if !self.eat(TokenKind::Minus) {
            return self.parse_primary();
        }
        if let TokenKind::Integer(magnitude) = self.peek().kind {
            let token = self.advance();
            let value = if magnitude == (i64::MAX as u64) + 1 {
                i64::MIN
            } else if magnitude <= i64::MAX as u64 {
                -(magnitude as i64)
            } else {
                return Err(ParseError::new(
                    token.span,
                    "negative integer literal is outside the i64 range",
                ));
            };
            return Ok(self.graph.constant(value));
        }
        let operand = self.parse_unary()?;
        Ok(self.graph.unary(UnaryOp::Neg, operand))
    }

    fn parse_primary(&mut self) -> Result<NodeId, ParseError> {
        let token = self.advance();
        match token.kind {
            TokenKind::Integer(value) => {
                if value > i64::MAX as u64 {
                    return Err(ParseError::new(
                        token.span,
                        "integer literal is outside the positive i64 range",
                    ));
                }
                Ok(self.graph.constant(value as i64))
            }
            TokenKind::Ident(identifier) => {
                if self.check(&TokenKind::LParen) {
                    return Err(ParseError::new(
                        token.span,
                        format!("function call `{identifier}(...)` is not supported"),
                    ));
                }
                if identifier == "true" || identifier == "false" {
                    return Err(ParseError::new(
                        token.span,
                        "Boolean literals are not supported",
                    ));
                }
                self.variables.get(&identifier).copied().ok_or_else(|| {
                    ParseError::new(
                        token.span,
                        format!(
                            "unknown variable `{identifier}`; variables must be declared before use"
                        ),
                    )
                })
            }
            TokenKind::LParen => {
                let value = self.parse_expression()?;
                self.expect(TokenKind::RParen, "`)` after parenthesized expression")?;
                Ok(value)
            }
            TokenKind::Eof => Err(ParseError::new(
                token.span,
                "expected expression, found end of file",
            )),
            _ => Err(ParseError::new(token.span, "expected i64 expression")),
        }
    }

    fn is_comparison(&self, token: &Token) -> bool {
        matches!(
            token.kind,
            TokenKind::EqEq
                | TokenKind::Ne
                | TokenKind::Lt
                | TokenKind::Le
                | TokenKind::Gt
                | TokenKind::Ge
        )
    }

    fn peek_identifier(&self, expected: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Ident(identifier) if identifier == expected)
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.position]
    }

    fn advance(&mut self) -> Token {
        let token = self.tokens[self.position].clone();
        if token.kind != TokenKind::Eof {
            self.position += 1;
        }
        token
    }

    fn check(&self, expected: &TokenKind) -> bool {
        std::mem::discriminant(&self.peek().kind) == std::mem::discriminant(expected)
    }

    fn eat(&mut self, expected: TokenKind) -> bool {
        if self.check(&expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, expected: TokenKind, description: &str) -> Result<Span, ParseError> {
        if self.check(&expected) {
            Ok(self.advance().span)
        } else {
            Err(self.error_here(format!("expected {description}")))
        }
    }

    fn error_here(&self, message: impl Into<String>) -> ParseError {
        ParseError::new(self.peek().span, message)
    }
}

#[cfg(test)]
mod tests {
    use super::{ParseError, parse};
    use crate::son::{BinaryOp, Graph, NodeKind, UnaryOp};

    fn graph(source: &str) -> Graph {
        match parse(source) {
            Ok(graph) => {
                graph.validate().expect("frontend emitted an invalid graph");
                graph
            }
            Err(error) => panic!("source unexpectedly failed: {error}"),
        }
    }

    fn error(source: &str) -> ParseError {
        match parse(source) {
            Ok(_) => panic!("source unexpectedly parsed: {source}"),
            Err(error) => error,
        }
    }

    #[test]
    fn parses_functions_and_bindings() {
        let graph = graph(
            "fn twice(x: i64,) -> i64 { return x * 2; }\n\
             fn main() -> i64 { let x: i64 = 2 + 3 * 4; let y = (x - 5) / 3 % 2; return y; }",
        );
        assert_eq!(
            graph
                .iter()
                .filter(|(_, node)| node.kind == NodeKind::Start)
                .count(),
            2
        );
    }

    #[test]
    fn lowers_precedence_and_unary_negation_to_expected_nodes() {
        let graph = graph("fn main() { let x = 2 + 3 * 4; return -x; }");
        let return_node = graph
            .iter()
            .find(|(_, node)| node.kind == NodeKind::Return)
            .expect("return node")
            .1;
        let negation = graph.node(return_node.data[0]).unwrap();
        assert_eq!(negation.kind, NodeKind::Unary(UnaryOp::Neg));
        let addition = graph.node(negation.data[0]).unwrap();
        assert_eq!(addition.kind, NodeKind::Binary(BinaryOp::Add));
        assert_eq!(
            graph.node(addition.data[0]).unwrap().kind,
            NodeKind::Const(2)
        );
        let multiplication = graph.node(addition.data[1]).unwrap();
        assert_eq!(multiplication.kind, NodeKind::Binary(BinaryOp::Mul));
        assert_eq!(
            graph.node(multiplication.data[0]).unwrap().kind,
            NodeKind::Const(3)
        );
        assert_eq!(
            graph.node(multiplication.data[1]).unwrap().kind,
            NodeKind::Const(4)
        );
    }

    #[test]
    fn parses_every_comparison() {
        for comparison in ["==", "!=", "<", "<=", ">", ">="] {
            let source = format!("fn main() {{ return 2 {comparison} 3; }}");
            graph(&source);
        }
    }

    #[test]
    fn parses_comments_literals_and_i64_minimum() {
        assert!(
            parse(
                "/* outer /* nested */ comment */\n\
                 fn main() { // line comment\n\
                   let min = -9_223_372_036_854_775_808i64;\n\
                   return min + -(-1); }",
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_unknown_variables_and_calls() {
        assert!(
            error("fn main() { return missing; }")
                .message
                .contains("unknown variable")
        );
        assert!(
            error("fn id(x: i64) -> i64 { return x; } fn main() { return id(1); }")
                .message
                .contains("not supported")
        );
    }

    #[test]
    fn rejects_duplicate_names_and_shadowing() {
        assert!(
            error("fn f() { return 0; } fn f() { return 1; }")
                .message
                .contains("duplicate function")
        );
        assert!(
            error("fn f(x: i64, x: i64) { return x; }")
                .message
                .contains("duplicate parameter")
        );
        assert!(
            error("fn main() { let x = 1; let x = 2; return x; }")
                .message
                .contains("shadowing")
        );
    }

    #[test]
    fn rejects_mutation_and_non_i64_types() {
        assert!(
            error("fn main() { let mut x = 1; return x; }")
                .message
                .contains("mutable")
        );
        assert!(
            error("fn f(x: i32) -> i64 { return x; }")
                .message
                .contains("only the `i64` type")
        );
        assert!(
            error("fn main() -> bool { return 1; }")
                .message
                .contains("only the `i64` type")
        );
    }

    #[test]
    fn rejects_missing_return_and_unreachable_statements() {
        assert!(
            error("fn main() { let x = 1; }")
                .message
                .contains("must end with `return")
        );
        assert!(
            error("fn main() { return 1; let x = 2; }")
                .message
                .contains("after `return`")
        );
        assert!(
            error("fn main() { 1 + 2; }")
                .message
                .contains("only `let` bindings")
        );
    }

    #[test]
    fn rejects_invalid_or_unsupported_literals() {
        assert!(
            error("fn main() { return 9223372036854775808; }")
                .message
                .contains("positive i64 range")
        );
        assert!(
            error("fn main() { return -9223372036854775809; }")
                .message
                .contains("outside the i64 range")
        );
        assert!(
            error("fn main() { return 1i32; }")
                .message
                .contains("suffix")
        );
        assert!(
            error("fn main() { return 0xff; }")
                .message
                .contains("suffix")
        );
        assert!(
            error("fn main() { return 1.0; }")
                .message
                .contains("floating-point")
        );
        assert!(
            error("fn main() { return true; }")
                .message
                .contains("Boolean")
        );
    }

    #[test]
    fn rejects_chained_comparisons() {
        assert!(
            error("fn main() { return 1 < 2 < 3; }")
                .message
                .contains("chained comparisons")
        );
    }

    #[test]
    fn rejects_other_rust_items_and_control_flow() {
        assert!(
            error("pub fn main() { return 0; }")
                .message
                .contains("expected `fn`")
        );
        assert!(
            error("fn main() { if 1 { return 2; } }")
                .message
                .contains("only `let` bindings")
        );
        assert!(
            error("fn main() { return 1 && 0; }")
                .message
                .contains("unsupported token")
        );
        assert!(
            error("fn if() { return 0; }")
                .message
                .contains("reserved keyword")
        );
        assert!(
            error("fn main() { let i64 = 1; return 0; }")
                .message
                .contains("reserved keyword")
        );
    }

    #[test]
    fn rejects_unterminated_comments_and_reports_location() {
        let parse_error = error("\n fn main() { /* no end\n return 1; }");
        assert!(parse_error.message.contains("unterminated block comment"));
        assert_eq!(parse_error.line, 2);
        assert!(parse_error.column > 1);
    }

    #[test]
    fn rejects_empty_program_and_bad_punctuation() {
        assert!(
            error(" // nothing\n")
                .message
                .contains("at least one function")
        );
        assert!(
            error("fn main() { return 1 }")
                .message
                .contains("expected `;`")
        );
        assert!(
            error("fn main() { return (1 + 2; }")
                .message
                .contains("expected `)`")
        );
    }
}
