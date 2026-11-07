use crate::son::{BinaryOp, CompareOp, Graph, NodeId};
use std::collections::HashMap;
use std::error::Error;
use std::fmt;

/// Parses one single-basic-block MIR function with an `i64` return value.
///
/// Locals must be declared before `bb0`, assigned at most once, and defined
/// before use. Only local operands, decimal `i64` constants, arithmetic,
/// comparisons, and a final `return;` are supported.
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
    Ident(String),
    Integer(u64),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Colon,
    Semicolon,
    Comma,
    Arrow,
    Eq,
    Minus,
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
            let start = self.span();
            if self.offset == self.source.len() {
                tokens.push(Token {
                    kind: TokenKind::Eof,
                    span: start,
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
                    ..start
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
        TokenKind::Ident(self.source[start..self.offset].to_owned())
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
        if suffix != "i64" {
            return Err(ParseError::new(
                Span {
                    end: self.offset,
                    ..start
                },
                "only decimal integer literals with an explicit `i64` suffix are supported",
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
                "integer literal is outside the supported i64 range",
            )
        })?;
        Ok(TokenKind::Integer(value))
    }

    fn lex_punctuation(&mut self) -> Result<TokenKind, ParseError> {
        let start = self.span();
        let ch = self.advance().expect("punctuation requires a character");
        let kind = match ch {
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            ':' => TokenKind::Colon,
            ';' => TokenKind::Semicolon,
            ',' => TokenKind::Comma,
            '=' => TokenKind::Eq,
            '-' => {
                if self.rest().starts_with('>') {
                    self.advance();
                    TokenKind::Arrow
                } else {
                    TokenKind::Minus
                }
            }
            '[' | ']' => {
                return Err(ParseError::new(
                    start,
                    "array, slice, and aggregate syntax is not supported",
                ));
            }
            '&' | '|' | '!' => {
                return Err(ParseError::new(
                    start,
                    format!("operator or reference token `{ch}` is not supported"),
                ));
            }
            '.' => {
                return Err(ParseError::new(
                    start,
                    "floating-point literals, field access, and paths are not supported",
                ));
            }
            '"' | '\'' => {
                return Err(ParseError::new(
                    start,
                    "string, character, and lifetime syntax is not supported",
                ));
            }
            _ => {
                return Err(ParseError::new(
                    start,
                    format!("unsupported token `{ch}` in MIR input"),
                ));
            }
        };
        Ok(kind)
    }
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_ident_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

#[derive(Clone, Copy)]
struct Local {
    value: Option<NodeId>,
    parameter: bool,
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
    graph: Graph,
    start: Option<NodeId>,
    locals: HashMap<usize, Local>,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            position: 0,
            graph: Graph::new(),
            start: None,
            locals: HashMap::new(),
        }
    }

    fn parse_program(mut self) -> Result<Graph, ParseError> {
        if self.check(&TokenKind::Eof) {
            return Err(self.error_here("expected exactly one MIR function"));
        }
        self.parse_function()?;
        if !self.check(&TokenKind::Eof) {
            return Err(self.error_here("only one MIR function is supported"));
        }
        debug_assert!(
            self.graph.validate().is_ok(),
            "MIR importer emitted an invalid graph"
        );
        Ok(self.graph)
    }

    fn parse_function(&mut self) -> Result<(), ParseError> {
        self.expect_keyword("fn", "`fn` at the start of the MIR function")?;
        let name = self.advance();
        if !matches!(name.kind, TokenKind::Ident(_)) {
            return Err(ParseError::new(name.span, "expected a function name"));
        }

        self.start = Some(self.graph.start());
        self.expect(TokenKind::LParen, "`(` after the function name")?;
        let mut parameter_count = 0usize;
        if !self.check(&TokenKind::RParen) {
            loop {
                parameter_count += 1;
                let (index, span) = self.parse_local("parameter")?;
                if index != parameter_count {
                    return Err(ParseError::new(
                        span,
                        format!(
                            "parameters must be consecutive and named `_1`, `_2`, ...; expected `_{parameter_count}`"
                        ),
                    ));
                }
                self.expect(TokenKind::Colon, "`:` and an explicit parameter type")?;
                self.parse_i64_type()?;
                let value = self.graph.parameter(index - 1);
                self.locals.insert(
                    index,
                    Local {
                        value: Some(value),
                        parameter: true,
                    },
                );
                if !self.eat(TokenKind::Comma) || self.check(&TokenKind::RParen) {
                    break;
                }
            }
        }
        self.expect(TokenKind::RParen, "`)` after function parameters")?;
        self.expect(TokenKind::Arrow, "an explicit `-> i64` return type")?;
        self.parse_i64_type()?;
        self.expect(TokenKind::LBrace, "`{` before the function body")?;

        while self.check_keyword("let") {
            self.parse_declaration()?;
        }
        if !self.locals.contains_key(&0) {
            return Err(self.error_here("the function must declare return local `_0: i64`"));
        }

        let label = self.advance();
        match &label.kind {
            TokenKind::Ident(identifier) if identifier == "bb0" => {}
            TokenKind::Ident(identifier) if identifier.starts_with("bb") => {
                return Err(ParseError::new(
                    label.span,
                    "only the single basic block `bb0` is supported",
                ));
            }
            _ => {
                return Err(ParseError::new(
                    label.span,
                    "expected `bb0`; only local declarations may precede the basic block",
                ));
            }
        }
        self.expect(TokenKind::Colon, "`:` after `bb0`")?;
        self.expect(TokenKind::LBrace, "`{` after `bb0:`")?;

        let mut returned = false;
        while !self.check(&TokenKind::RBrace) {
            if self.check(&TokenKind::Eof) {
                return Err(self.error_here("unterminated `bb0` body"));
            }
            if self.check_keyword("return") {
                let return_token = self.advance();
                self.expect(TokenKind::Semicolon, "`return;` without an operand")?;
                let value = self
                    .locals
                    .get(&0)
                    .and_then(|local| local.value)
                    .ok_or_else(|| {
                        ParseError::new(return_token.span, "`_0` must be assigned before `return;`")
                    })?;
                self.graph
                    .return_(self.start.expect("function start"), Some(value));
                returned = true;
                if !self.check(&TokenKind::RBrace) {
                    return Err(self.error_here(
                        "`return;` must be the final statement in the only basic block",
                    ));
                }
            } else {
                if self.check_keyword("let") {
                    return Err(self
                        .error_here("local declarations must appear before `bb0`, not inside it"));
                }
                if let TokenKind::Ident(identifier) = &self.peek().kind {
                    if !is_local_name(identifier) {
                        return Err(self.error_here(format!(
                            "unsupported statement or terminator `{identifier}`; only local assignments and `return;` are supported"
                        )));
                    }
                }
                self.parse_assignment()?;
            }
        }
        let block_close = self.expect(TokenKind::RBrace, "`}` after `bb0`")?;
        if !returned {
            return Err(ParseError::new(
                block_close,
                "the basic block must end with `return;`",
            ));
        }
        if !self.check(&TokenKind::RBrace) {
            return Err(self.error_here(
                "expected the function closing `}`; additional basic blocks and items are not supported",
            ));
        }
        self.expect(TokenKind::RBrace, "`}` after the function body")?;
        Ok(())
    }

    fn parse_declaration(&mut self) -> Result<(), ParseError> {
        self.expect_keyword("let", "`let`")?;
        self.eat_keyword("mut");
        let (index, span) = self.parse_local("local declaration")?;
        if self.locals.contains_key(&index) {
            return Err(ParseError::new(
                span,
                format!("duplicate local declaration `_{index}`"),
            ));
        }
        self.expect(TokenKind::Colon, "`:` and an explicit local type")?;
        self.parse_i64_type()?;
        if self.check(&TokenKind::Eq) {
            return Err(self.error_here(
                "initializers in `let` declarations are not supported; assign the local in `bb0`",
            ));
        }
        self.expect(TokenKind::Semicolon, "`;` after the local declaration")?;
        self.locals.insert(
            index,
            Local {
                value: None,
                parameter: false,
            },
        );
        Ok(())
    }

    fn parse_assignment(&mut self) -> Result<(), ParseError> {
        let (index, span) = self.parse_local("assignment destination")?;
        let Some(local) = self.locals.get(&index).copied() else {
            return Err(ParseError::new(
                span,
                format!("assignment to undeclared local `_{index}`"),
            ));
        };
        if local.parameter {
            return Err(ParseError::new(
                span,
                "assignment to a function parameter is not supported",
            ));
        }
        if local.value.is_some() {
            return Err(ParseError::new(
                span,
                format!("multiple assignments to `_{index}` are not supported"),
            ));
        }
        self.expect(TokenKind::Eq, "`=` in a local assignment")?;
        let value = self.parse_rvalue()?;
        self.expect(TokenKind::Semicolon, "`;` after the assignment")?;
        self.locals
            .get_mut(&index)
            .expect("local still exists")
            .value = Some(value);
        Ok(())
    }

    fn parse_rvalue(&mut self) -> Result<NodeId, ParseError> {
        if self.check_keyword("const")
            || self.check_keyword("copy")
            || self.check_keyword("move")
            || matches!(&self.peek().kind, TokenKind::Ident(name) if is_local_name(name))
        {
            return self.parse_operand();
        }
        if self.check(&TokenKind::LParen) {
            return Err(self.error_here("tuple and aggregate rvalues are not supported"));
        }

        let operation = self.advance();
        let TokenKind::Ident(name) = &operation.kind else {
            return Err(ParseError::new(
                operation.span,
                "expected a constant, local operand, arithmetic operation, or comparison",
            ));
        };
        let binary = match name.as_str() {
            "Add" => Some(BinaryOp::Add),
            "Sub" => Some(BinaryOp::Sub),
            "Mul" => Some(BinaryOp::Mul),
            "Div" => Some(BinaryOp::Div),
            "Rem" => Some(BinaryOp::Rem),
            _ => None,
        };
        let compare = match name.as_str() {
            "Eq" => Some(CompareOp::Eq),
            "Ne" => Some(CompareOp::Ne),
            "Lt" => Some(CompareOp::Lt),
            "Le" => Some(CompareOp::Le),
            "Gt" => Some(CompareOp::Gt),
            "Ge" => Some(CompareOp::Ge),
            _ => None,
        };
        if binary.is_none() && compare.is_none() {
            return Err(ParseError::new(
                operation.span,
                format!(
                    "unsupported rvalue `{name}`; only local values, i64 constants, Add/Sub/Mul/Div/Rem, and Eq/Ne/Lt/Le/Gt/Ge are supported"
                ),
            ));
        }

        self.expect(TokenKind::LParen, "`(` after the rvalue operation")?;
        let left = self.parse_operand()?;
        self.expect(TokenKind::Comma, "`,` between binary operands")?;
        let right = self.parse_operand()?;
        self.expect(TokenKind::RParen, "`)` after binary operands")?;
        Ok(if let Some(operation) = binary {
            self.graph.binary(operation, left, right)
        } else {
            self.graph
                .compare(compare.expect("comparison was checked"), left, right)
        })
    }

    fn parse_operand(&mut self) -> Result<NodeId, ParseError> {
        if self.eat_keyword("const") {
            let value = self.parse_constant()?;
            return Ok(self.graph.constant(value));
        }
        if !self.eat_keyword("copy") {
            self.eat_keyword("move");
        }
        let (index, span) = self.parse_local("operand")?;
        let Some(local) = self.locals.get(&index) else {
            return Err(ParseError::new(
                span,
                format!("use of undeclared local `_{index}`"),
            ));
        };
        local.value.ok_or_else(|| {
            ParseError::new(
                span,
                format!("local `_{index}` is used before its single assignment"),
            )
        })
    }

    fn parse_constant(&mut self) -> Result<i64, ParseError> {
        let negative = self.eat(TokenKind::Minus);
        let token = self.advance();
        let TokenKind::Integer(magnitude) = token.kind else {
            return Err(ParseError::new(
                token.span,
                "expected a decimal constant with an explicit `i64` suffix",
            ));
        };
        if negative {
            if magnitude == (i64::MAX as u64) + 1 {
                Ok(i64::MIN)
            } else if magnitude <= i64::MAX as u64 {
                Ok(-(magnitude as i64))
            } else {
                Err(ParseError::new(
                    token.span,
                    "negative constant is outside the i64 range",
                ))
            }
        } else if magnitude <= i64::MAX as u64 {
            Ok(magnitude as i64)
        } else {
            Err(ParseError::new(
                token.span,
                "positive constant is outside the i64 range",
            ))
        }
    }

    fn parse_i64_type(&mut self) -> Result<(), ParseError> {
        if self.eat_keyword("i64") {
            Ok(())
        } else {
            Err(self.error_here("only the scalar type `i64` is supported"))
        }
    }

    fn parse_local(&mut self, description: &str) -> Result<(usize, Span), ParseError> {
        let token = self.advance();
        let TokenKind::Ident(identifier) = &token.kind else {
            return Err(ParseError::new(
                token.span,
                format!("expected a MIR local for {description}"),
            ));
        };
        if !is_local_name(identifier) {
            return Err(ParseError::new(
                token.span,
                format!("expected `_0`, `_1`, ... as a MIR local for {description}"),
            ));
        }
        let digits = &identifier[1..];
        if digits.len() > 1 && digits.starts_with('0') {
            return Err(ParseError::new(
                token.span,
                format!("non-canonical local name `{identifier}` is not supported"),
            ));
        }
        let index = digits.parse::<usize>().map_err(|_| {
            ParseError::new(
                token.span,
                format!("local index in `{identifier}` is too large"),
            )
        })?;
        Ok((index, token.span))
    }

    fn expect_keyword(&mut self, expected: &str, description: &str) -> Result<Span, ParseError> {
        if self.eat_keyword(expected) {
            Ok(self.tokens[self.position - 1].span)
        } else {
            Err(self.error_here(format!("expected {description}")))
        }
    }

    fn check_keyword(&self, expected: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Ident(identifier) if identifier == expected)
    }

    fn eat_keyword(&mut self, expected: &str) -> bool {
        if self.check_keyword(expected) {
            self.position += 1;
            true
        } else {
            false
        }
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

fn is_local_name(identifier: &str) -> bool {
    identifier
        .strip_prefix('_')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|ch| ch.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::{ParseError, parse};
    use crate::son::{BinaryOp, CompareOp, Graph, NodeKind};

    fn graph(source: &str) -> Graph {
        let graph =
            parse(source).unwrap_or_else(|error| panic!("MIR unexpectedly failed: {error}"));
        graph
            .validate()
            .expect("MIR importer emitted an invalid graph");
        graph
    }

    fn error(source: &str) -> ParseError {
        match parse(source) {
            Ok(_) => panic!("MIR unexpectedly parsed: {source}"),
            Err(error) => error,
        }
    }

    fn returned_value(graph: &Graph) -> &crate::son::Node {
        let return_node = graph
            .iter()
            .find(|(_, node)| node.kind == NodeKind::Return)
            .expect("return node")
            .1;
        graph.node(return_node.data[0]).expect("returned value")
    }

    #[test]
    fn imports_constants_aliases_and_return() {
        let minimum_graph = graph(
            "fn minimum() -> i64 {\n\
                 let mut _0: i64;\n\
                 bb0: {\n\
                     _0 = const -9_223_372_036_854_775_808_i64;\n\
                     return;\n\
                 }\n\
             }",
        );
        assert_eq!(
            returned_value(&minimum_graph).kind,
            NodeKind::Const(i64::MIN)
        );
        assert_eq!(minimum_graph.len(), 3);

        let graph = graph(
            "fn identity(_1: i64) -> i64 {
                 let mut _0: i64;
                 let _2: i64;
                 bb0: {
                     _2 = copy _1;
                     _0 = move _2;
                     return;
                 }
             }",
        );
        assert_eq!(
            returned_value(&graph).kind,
            NodeKind::Parameter { index: 0 }
        );
    }

    #[test]
    fn imports_every_arithmetic_operation() {
        let cases = [
            ("Add", BinaryOp::Add),
            ("Sub", BinaryOp::Sub),
            ("Mul", BinaryOp::Mul),
            ("Div", BinaryOp::Div),
            ("Rem", BinaryOp::Rem),
        ];
        for (name, expected) in cases {
            let source = format!(
                "fn arithmetic(_1: i64, _2: i64) -> i64 {{
                     let mut _0: i64;
                     bb0: {{
                         _0 = {name}(copy _1, move _2);
                         return;
                     }}
                 }}"
            );
            let graph = graph(&source);
            assert_eq!(returned_value(&graph).kind, NodeKind::Binary(expected));
            let parameters: Vec<_> = graph
                .iter()
                .filter_map(|(_, node)| match node.kind {
                    NodeKind::Parameter { index } => Some(index),
                    _ => None,
                })
                .collect();
            assert_eq!(parameters, [0, 1]);
        }
    }

    #[test]
    fn imports_every_comparison_and_chained_let_values() {
        let cases = [
            ("Eq", CompareOp::Eq),
            ("Ne", CompareOp::Ne),
            ("Lt", CompareOp::Lt),
            ("Le", CompareOp::Le),
            ("Gt", CompareOp::Gt),
            ("Ge", CompareOp::Ge),
        ];
        for (name, expected) in cases {
            let source = format!(
                "fn compare(_1: i64, _2: i64) -> i64 {{
                     let mut _0: i64;
                     let _3: i64;
                     bb0: {{
                         _3 = Add(_1, const 2i64);
                         _0 = {name}(_3, _2);
                         return;
                     }}
                 }}"
            );
            let graph = graph(&source);
            let comparison = returned_value(&graph);
            assert_eq!(comparison.kind, NodeKind::Compare(expected));
            assert_eq!(
                graph.node(comparison.data[0]).unwrap().kind,
                NodeKind::Binary(BinaryOp::Add)
            );
        }
    }

    #[test]
    fn accepts_comments_and_trailing_parameter_comma() {
        let source = "/* outer /* nested */ comment */
            fn f(_1: i64,) -> i64 {
                let mut _0: i64; // return slot
                bb0: {
                    _0 = const 7_i64;
                    return;
                }
            }";
        assert_eq!(returned_value(&graph(source)).kind, NodeKind::Const(7));
    }

    #[test]
    fn rejects_multiple_functions_blocks_and_control_flow() {
        let prefix = "fn f() -> i64 { let mut _0: i64; bb0: { _0 = const 0_i64; return; } }";
        assert!(
            error(&format!("{prefix} {prefix}"))
                .message
                .contains("only one MIR function")
        );
        assert!(
            error("fn f() -> i64 { let mut _0: i64; bb1: { return; } }")
                .message
                .contains("only the single basic block")
        );
        assert!(
            error("fn f() -> i64 { let mut _0: i64; bb0: { _0 = const 0_i64; return; } bb1: { return; } }")
                .message
                .contains("additional basic blocks")
        );
        assert!(
            error("fn f(_1: i64) -> i64 { let mut _0: i64; bb0: { switchInt(_1); } }")
                .message
                .contains("unsupported statement or terminator")
        );
        assert!(
            error("fn f() -> i64 { let mut _0: i64; bb0: { StorageLive(_0); return; } }")
                .message
                .contains("unsupported statement or terminator")
        );
    }

    #[test]
    fn rejects_unsupported_types_rvalues_and_aggregates() {
        assert!(
            error("fn f() -> i32 { let mut _0: i32; bb0: { return; } }")
                .message
                .contains("only the scalar type `i64`")
        );
        assert!(
            error("fn f() -> i64 { let mut _0: (i64, i64); bb0: { return; } }")
                .message
                .contains("only the scalar type `i64`")
        );
        assert!(
            error("fn f(_1: i64) -> i64 { let mut _0: i64; bb0: { _0 = Ref(_1); return; } }")
                .message
                .contains("unsupported rvalue `Ref`")
        );
        assert!(
            error("fn f(_1: i64) -> i64 { let mut _0: i64; bb0: { _0 = (_1, _1); return; } }")
                .message
                .contains("aggregate")
        );
        assert!(
            error("fn f(_1: i64) -> i64 { let mut _0: i64; bb0: { _0 = CheckedAdd(_1, _1); return; } }")
                .message
                .contains("unsupported rvalue")
        );
    }

    #[test]
    fn rejects_invalid_local_lifetimes_and_assignments() {
        assert!(
            error("fn f(_2: i64) -> i64 { let mut _0: i64; bb0: { return; } }")
                .message
                .contains("parameters must be consecutive")
        );
        assert!(
            error("fn f() -> i64 { bb0: { return; } }")
                .message
                .contains("must declare return local")
        );
        assert!(
            error("fn f() -> i64 { let mut _0: i64; let _1: i64; bb0: { _0 = _1; return; } }")
                .message
                .contains("used before")
        );
        assert!(
            error("fn f() -> i64 { let mut _0: i64; bb0: { _0 = const 1_i64; _0 = const 2_i64; return; } }")
                .message
                .contains("multiple assignments")
        );
        assert!(
            error("fn f(_1: i64) -> i64 { let mut _0: i64; bb0: { _1 = const 2_i64; return; } }")
                .message
                .contains("assignment to a function parameter")
        );
        assert!(
            error("fn f() -> i64 { let mut _0: i64; bb0: { _0 = const 1_i64; } }")
                .message
                .contains("must end with `return;`")
        );
        assert!(
            error("fn f() -> i64 { let mut _0: i64; bb0: { _0 = const 1_i64; return _0; } }")
                .message
                .contains("`return;` without an operand")
        );
    }

    #[test]
    fn rejects_bad_constants_and_reports_exact_location() {
        let source = "fn f() -> i64 {\n    let mut _0: i64;\n    bb0: {\n        _0 = const 1_i32;\n        return;\n    }\n}";
        let parse_error = error(source);
        assert_eq!(parse_error.line, 4);
        assert_eq!(parse_error.column, 20);
        assert_eq!(parse_error.offset, source.find("1_i32").unwrap());
        assert!(parse_error.message.contains("explicit `i64` suffix"));
        assert!(parse_error.to_string().starts_with("4:20:"));

        assert!(
            error("fn f() -> i64 { let mut _0: i64; bb0: { _0 = const 9223372036854775808_i64; return; } }")
                .message
                .contains("positive constant")
        );
        assert!(
            error("fn f() -> i64 { let mut _0: i64; bb0: { _0 = const -9223372036854775809_i64; return; } }")
                .message
                .contains("negative constant")
        );
        assert!(
            error("fn f() -> i64 { let mut _0: i64; bb0: { _0 = const 1; return; } }")
                .message
                .contains("suffix")
        );
    }
}
