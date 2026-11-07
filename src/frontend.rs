use crate::son::{BinaryOp, CompareOp, FunctionMetadata, Graph, NodeId, NodeKind, UnaryOp};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

/// Parses the explicitly supported `i64` and `bool` Rust subset.
///
/// Functions and parameters retain their names and source ranges in `Graph::functions`.
/// `if` expressions and statements merge values through explicit jumps, control regions,
/// and Phi nodes. Mutable bindings and `while` loops use loop-header Phis, multiple back
/// edges from `continue`, and exit Phis for `break` snapshots.
/// Calls are checked against declared signatures. Comparisons and logical operations produce
/// booleans; `&&` and `||` evaluate their right operand conditionally. Unsupported Rust is
/// rejected with a source location.
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
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    Eq,
    EqEq,
    Ne,
    Not,
    AndAnd,
    OrOr,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Xor,
    Shl,
    Shr,
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
            Some('+') => {
                if self.rest().starts_with('=') {
                    self.advance();
                    TokenKind::PlusEq
                } else {
                    TokenKind::Plus
                }
            }
            Some('-') => {
                if self.rest().starts_with('>') {
                    self.advance();
                    TokenKind::Arrow
                } else if self.rest().starts_with('=') {
                    self.advance();
                    TokenKind::MinusEq
                } else {
                    TokenKind::Minus
                }
            }
            Some('*') => {
                if self.rest().starts_with('=') {
                    self.advance();
                    TokenKind::StarEq
                } else {
                    TokenKind::Star
                }
            }
            Some('/') => {
                if self.rest().starts_with('=') {
                    self.advance();
                    TokenKind::SlashEq
                } else {
                    TokenKind::Slash
                }
            }
            Some('%') => {
                if self.rest().starts_with('=') {
                    self.advance();
                    TokenKind::PercentEq
                } else {
                    TokenKind::Percent
                }
            }
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
                    TokenKind::Not
                }
            }
            Some('<') => {
                if self.rest().starts_with('<') {
                    self.advance();
                    TokenKind::Shl
                } else if self.rest().starts_with('=') {
                    self.advance();
                    TokenKind::Le
                } else {
                    TokenKind::Lt
                }
            }
            Some('>') => {
                if self.rest().starts_with('>') {
                    self.advance();
                    TokenKind::Shr
                } else if self.rest().starts_with('=') {
                    self.advance();
                    TokenKind::Ge
                } else {
                    TokenKind::Gt
                }
            }
            Some('&') => {
                if self.rest().starts_with('&') {
                    self.advance();
                    TokenKind::AndAnd
                } else {
                    TokenKind::And
                }
            }
            Some('|') => {
                if self.rest().starts_with('|') {
                    self.advance();
                    TokenKind::OrOr
                } else {
                    TokenKind::Or
                }
            }
            Some('^') => TokenKind::Xor,
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
            | "bool"
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScalarType {
    I64,
    Bool,
}

impl fmt::Display for ScalarType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::I64 => formatter.write_str("i64"),
            Self::Bool => formatter.write_str("bool"),
        }
    }
}

#[derive(Clone)]
struct FunctionSignature {
    parameter_types: Vec<ScalarType>,
    return_type: Option<ScalarType>,
}

#[derive(Clone, Copy)]
struct Binding {
    value: NodeId,
    mutable: bool,
    ty: ScalarType,
}

struct LoopContext {
    header: NodeId,
    phis: Vec<(String, NodeId)>,
    breaks: Vec<(NodeId, HashMap<String, Binding>)>,
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
    graph: Graph,
    functions: HashSet<String>,
    signatures: HashMap<String, FunctionSignature>,
    variables: HashMap<String, Binding>,
    types: HashMap<NodeId, ScalarType>,
    control: NodeId,
    return_type: Option<ScalarType>,
    loops: Vec<LoopContext>,
    terminated: bool,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            position: 0,
            graph: Graph::new(),
            functions: HashSet::new(),
            signatures: HashMap::new(),
            variables: HashMap::new(),
            types: HashMap::new(),
            control: NodeId(0),
            return_type: None,
            loops: Vec::new(),
            terminated: false,
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
                let mutable = if self.peek_identifier("mut") {
                    self.position += 1;
                    true
                } else {
                    false
                };
                let (parameter, parameter_span) = self.parse_identifier("parameter name")?;
                if !parameter_names.insert(parameter.clone()) {
                    return Err(ParseError::new(
                        parameter_span,
                        format!("duplicate parameter `{parameter}`"),
                    ));
                }
                self.expect(TokenKind::Colon, "`:` and explicit parameter type")?;
                let ty = self.parse_type()?;
                parameters.push((parameter, mutable, ty));
                if !self.eat(TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RParen) {
                    break;
                }
            }
        }
        self.expect(TokenKind::RParen, "`)` after parameters")?;
        let return_type = if self.eat(TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.return_type = return_type;
        self.signatures.insert(
            name.clone(),
            FunctionSignature {
                parameter_types: parameters.iter().map(|(_, _, ty)| *ty).collect(),
                return_type,
            },
        );

        self.expect(TokenKind::LBrace, "`{` before function body")?;
        let start = self.graph.start();
        let mut return_node = None;
        self.control = start;
        self.variables.clear();
        self.types.clear();
        for (index, (parameter, mutable, ty)) in parameters.iter().enumerate() {
            let node = self.graph.parameter(index);
            self.types.insert(node, *ty);
            self.variables.insert(
                parameter.clone(),
                Binding {
                    value: node,
                    mutable: *mutable,
                    ty: *ty,
                },
            );
        }

        while !self.check(&TokenKind::RBrace) {
            if self.check(&TokenKind::Eof) {
                return Err(self.error_here(format!("unterminated body of function `{name}`")));
            }
            if return_node.is_some() {
                return Err(self.error_here(
                    "statements after `return` are unsupported; `return` must end the body",
                ));
            }
            if let Some(node) = self.parse_statement(true)? {
                return_node = Some(node);
            }
        }
        let close = self.expect(TokenKind::RBrace, "`}` after function body")?;
        if return_node.is_some()
            && return_type.is_some()
            && self
                .graph
                .iter()
                .last()
                .is_some_and(|(_, node)| node.kind == NodeKind::Return && node.data.is_empty())
        {
            if let Some(expected) = return_type {
                return Err(ParseError::new(
                    close,
                    format!("expected a `{expected}` expression after `return`"),
                ));
            }
        }
        if return_node.is_none() && return_type.is_some() {
            return Err(ParseError::new(
                close,
                format!("function `{name}` must end with `return expr;`"),
            ));
        }
        let return_node = return_node.unwrap_or_else(|| self.graph.return_(self.control, None));
        self.graph.functions.push(FunctionMetadata {
            name,
            start,
            return_node,
            parameter_count: parameters.len(),
        });
        Ok(())
    }

    fn parse_statement(&mut self, allow_return: bool) -> Result<Option<NodeId>, ParseError> {
        self.terminated = false;
        if self.eat(TokenKind::Let) {
            self.parse_let()?;
            return Ok(None);
        }
        if self.peek_identifier("if") {
            self.parse_if_statement()?;
            return Ok(None);
        }
        if self.peek_identifier("while") {
            self.parse_while()?;
            return Ok(None);
        }
        if self.peek_identifier("break") {
            self.position += 1;
            self.expect(TokenKind::Semicolon, "`;` after break")?;
            self.parse_break()?;
            return Ok(None);
        }
        if self.peek_identifier("continue") {
            self.position += 1;
            self.expect(TokenKind::Semicolon, "`;` after continue")?;
            self.parse_continue()?;
            return Ok(None);
        }
        if self.eat(TokenKind::Return) {
            if !allow_return {
                return Err(
                    self.error_here("`return` inside a conditional or loop is not supported")
                );
            }
            self.terminated = true;
            if self.eat(TokenKind::Semicolon) {
                return Ok(Some(self.graph.return_(self.control, None)));
            }
            let value = self.parse_expression()?;
            if let Some(expected) = self.return_type {
                self.require_type(value, expected)?;
            }
            self.expect(TokenKind::Semicolon, "`;` after returned expression")?;
            return Ok(Some(self.graph.return_(self.control, Some(value))));
        }
        self.parse_assignment()?;
        Ok(None)
    }

    fn parse_let(&mut self) -> Result<(), ParseError> {
        let mutable = if self.peek_identifier("mut") {
            self.position += 1;
            true
        } else {
            false
        };
        let (name, name_span) = self.parse_identifier("binding name")?;
        if self.variables.contains_key(&name) {
            return Err(ParseError::new(
                name_span,
                format!("duplicate binding or parameter `{name}`; shadowing is not supported"),
            ));
        }
        let annotation = if self.eat(TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(TokenKind::Eq, "`=` in let binding")?;
        let value = self.parse_expression()?;
        let ty = annotation.unwrap_or_else(|| self.type_of(value));
        self.require_type(value, ty)?;
        self.expect(TokenKind::Semicolon, "`;` after let binding")?;
        self.variables.insert(name, Binding { value, mutable, ty });
        Ok(())
    }

    fn parse_assignment(&mut self) -> Result<(), ParseError> {
        let (name, name_span) = self.parse_identifier("assignment target")?;
        let binding = self
            .variables
            .get(&name)
            .copied()
            .ok_or_else(|| ParseError::new(name_span, format!("unknown variable `{name}`")))?;
        if !binding.mutable {
            return Err(ParseError::new(
                name_span,
                format!("cannot assign to immutable binding `{name}`"),
            ));
        }
        let operation = match self.advance().kind {
            TokenKind::Eq => None,
            TokenKind::PlusEq => Some(BinaryOp::Add),
            TokenKind::MinusEq => Some(BinaryOp::Sub),
            TokenKind::StarEq => Some(BinaryOp::Mul),
            TokenKind::SlashEq => Some(BinaryOp::Div),
            TokenKind::PercentEq => Some(BinaryOp::Rem),
            _ => return Err(self.error_here("expected an assignment operator")),
        };
        let right = self.parse_expression()?;
        let value = match operation {
            Some(operation) => {
                self.require_type(binding.value, ScalarType::I64)?;
                self.require_type(right, ScalarType::I64)?;
                self.graph.binary(operation, binding.value, right)
            }
            None => {
                self.require_type(right, binding.ty)?;
                right
            }
        };
        self.expect(TokenKind::Semicolon, "`;` after assignment")?;
        self.variables.insert(
            name,
            Binding {
                value,
                mutable: true,
                ty: binding.ty,
            },
        );
        Ok(())
    }

    fn parse_break(&mut self) -> Result<(), ParseError> {
        if self.loops.is_empty() {
            return Err(self.error_here("`break` outside a loop is not supported"));
        }
        let jump = self.graph.jump(self.control);
        let variables = self.variables.clone();
        self.loops
            .last_mut()
            .expect("loop context exists")
            .breaks
            .push((jump, variables));
        self.terminated = true;
        Ok(())
    }

    fn parse_continue(&mut self) -> Result<(), ParseError> {
        if self.loops.is_empty() {
            return Err(self.error_here("`continue` outside a loop is not supported"));
        }
        let jump = self.graph.jump(self.control);
        self.append_loop_edge(jump);
        self.terminated = true;
        Ok(())
    }

    fn append_loop_edge(&mut self, jump: NodeId) {
        let context = self.loops.last().expect("loop context exists");
        let header = context.header;
        let phis = context.phis.clone();
        for (name, phi) in phis {
            let value = self.variables[&name].value;
            let node = self.graph.node_mut(phi).expect("loop phi exists");
            node.control.push(jump);
            node.data.push(value);
        }
        self.graph
            .node_mut(header)
            .expect("loop header exists")
            .control
            .push(jump);
    }

    fn parse_statement_body(&mut self) -> Result<bool, ParseError> {
        let mut terminated = false;
        while !self.check(&TokenKind::RBrace) {
            if self.check(&TokenKind::Eof) {
                return Err(self.error_here("unterminated statement body"));
            }
            self.parse_statement(false)?;
            if self.terminated {
                terminated = true;
                break;
            }
        }
        if terminated && !self.check(&TokenKind::RBrace) {
            return Err(self.error_here("statements after `break` or `continue` are not supported"));
        }
        self.expect(TokenKind::RBrace, "`}` after statement body")?;
        Ok(terminated)
    }

    fn parse_if_statement(&mut self) -> Result<(), ParseError> {
        self.position += 1;
        let outer_variables = self.variables.clone();
        let condition = self.parse_expression()?;
        self.require_type(condition, ScalarType::Bool)?;
        let branch = self.graph.if_(self.control, condition);
        self.expect(TokenKind::LBrace, "`{` after if condition")?;

        let true_projection = self.graph.projection(branch, 0);
        self.control = true_projection;
        let true_terminated = self.parse_statement_body()?;
        let true_variables = self.variables.clone();
        let true_jump = if true_terminated {
            None
        } else {
            Some(self.graph.jump(self.control))
        };

        self.variables = outer_variables.clone();
        let false_projection = self.graph.projection(branch, 1);
        self.control = false_projection;
        let false_terminated = if self.peek_identifier("else") {
            self.position += 1;
            if self.peek_identifier("if") {
                self.parse_if_statement()?;
                self.terminated
            } else {
                self.expect(TokenKind::LBrace, "`{` after else")?;
                self.parse_statement_body()?
            }
        } else {
            self.terminated = false;
            false
        };
        let false_variables = self.variables.clone();
        let false_jump = if false_terminated {
            None
        } else {
            Some(self.graph.jump(self.control))
        };

        self.variables = outer_variables;
        if true_terminated && false_terminated {
            self.terminated = true;
            return Ok(());
        }
        let mut controls = Vec::new();
        if let Some(jump) = true_jump {
            controls.push(jump);
        }
        if let Some(jump) = false_jump {
            controls.push(jump);
        }
        self.control = self.graph.region(controls.clone());
        let mut merged_names = self
            .variables
            .iter()
            .filter(|(_, binding)| binding.mutable)
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        merged_names.sort();
        for name in merged_names {
            let ty = self.variables[&name].ty;
            let mut values = Vec::new();
            if true_jump.is_some() {
                values.push(true_variables[&name].value);
            }
            if false_jump.is_some() {
                values.push(false_variables[&name].value);
            }
            let phi = self.graph.phi(controls.clone(), values);
            self.types.insert(phi, ty);
            self.variables.insert(
                name,
                Binding {
                    value: phi,
                    mutable: true,
                    ty,
                },
            );
        }
        self.terminated = false;
        Ok(())
    }

    fn parse_while(&mut self) -> Result<(), ParseError> {
        self.position += 1;
        let outer_variables = self.variables.clone();
        let mut loop_variables = outer_variables
            .iter()
            .filter(|(_, binding)| binding.mutable)
            .map(|(name, binding)| (name.clone(), *binding))
            .collect::<Vec<_>>();
        loop_variables.sort_by(|left, right| left.0.cmp(&right.0));

        let header = self.graph.region(vec![self.control]);
        let mut phis = Vec::new();
        for (name, binding) in &loop_variables {
            let phi = self.graph.phi(vec![self.control], vec![binding.value]);
            self.types.insert(phi, binding.ty);
            self.variables.insert(
                name.clone(),
                Binding {
                    value: phi,
                    mutable: true,
                    ty: binding.ty,
                },
            );
            phis.push(phi);
        }
        self.control = header;
        let condition = self.parse_expression()?;
        self.require_type(condition, ScalarType::Bool)?;
        let branch = self.graph.if_(header, condition);
        self.expect(TokenKind::LBrace, "`{` after while condition")?;

        let body_projection = self.graph.projection(branch, 0);
        self.control = body_projection;
        self.loops.push(LoopContext {
            header,
            phis: loop_variables
                .iter()
                .zip(&phis)
                .map(|((name, _), phi)| (name.clone(), *phi))
                .collect(),
            breaks: Vec::new(),
        });
        let body_terminated = self.parse_statement_body()?;
        if !body_terminated {
            let back_edge = self.graph.jump(self.control);
            self.append_loop_edge(back_edge);
        }
        let exit_projection = self.graph.projection(branch, 1);
        let context = self.loops.pop().expect("loop context exists");

        let mut normal_variables = outer_variables.clone();
        for ((name, binding), phi) in loop_variables.iter().zip(&phis) {
            normal_variables.insert(
                name.clone(),
                Binding {
                    value: *phi,
                    mutable: true,
                    ty: binding.ty,
                },
            );
        }
        if context.breaks.is_empty() {
            self.variables = normal_variables;
            self.control = exit_projection;
            self.terminated = false;
            return Ok(());
        }

        let normal_exit = self.graph.jump(exit_projection);
        let mut controls = vec![normal_exit];
        controls.extend(context.breaks.iter().map(|(jump, _)| *jump));
        self.control = self.graph.region(controls.clone());
        self.variables = outer_variables;
        for (name, binding) in &loop_variables {
            let mut values = vec![normal_variables[name].value];
            values.extend(
                context
                    .breaks
                    .iter()
                    .map(|(_, variables)| variables[name].value),
            );
            let phi = self.graph.phi(controls.clone(), values);
            self.types.insert(phi, binding.ty);
            self.variables.insert(
                name.clone(),
                Binding {
                    value: phi,
                    mutable: true,
                    ty: binding.ty,
                },
            );
        }
        self.terminated = false;
        Ok(())
    }

    fn parse_type(&mut self) -> Result<ScalarType, ParseError> {
        let ty = if self.peek_identifier("i64") {
            ScalarType::I64
        } else if self.peek_identifier("bool") {
            ScalarType::Bool
        } else {
            return Err(self.error_here("only the `i64` and `bool` types are supported"));
        };
        self.position += 1;
        Ok(ty)
    }

    fn type_of(&self, value: NodeId) -> ScalarType {
        self.types.get(&value).copied().unwrap_or(ScalarType::I64)
    }

    fn require_type(&self, value: NodeId, expected: ScalarType) -> Result<(), ParseError> {
        let actual = self.type_of(value);
        if actual == expected {
            Ok(())
        } else {
            Err(self.error_here(format!(
                "type mismatch: expected `{expected}`, found `{actual}`"
            )))
        }
    }

    fn mark_bool(&mut self, value: NodeId) -> NodeId {
        self.types.insert(value, ScalarType::Bool);
        value
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
        if self.peek_identifier("if") {
            self.parse_if_expression()
        } else {
            self.parse_logical_or()
        }
    }

    fn parse_logical_or(&mut self) -> Result<NodeId, ParseError> {
        let mut value = self.parse_logical_and()?;
        while self.eat(TokenKind::OrOr) {
            value = self.parse_short_circuit(value, false)?;
        }
        Ok(value)
    }

    fn parse_logical_and(&mut self) -> Result<NodeId, ParseError> {
        let mut value = self.parse_comparison()?;
        while self.eat(TokenKind::AndAnd) {
            value = self.parse_short_circuit(value, true)?;
        }
        Ok(value)
    }

    fn parse_short_circuit(
        &mut self,
        left: NodeId,
        conjunction: bool,
    ) -> Result<NodeId, ParseError> {
        self.require_type(left, ScalarType::Bool)?;
        let branch = self.graph.if_(self.control, left);
        let true_projection = self.graph.projection(branch, 0);
        self.control = true_projection;
        let true_value = if conjunction {
            let value = self.parse_comparison()?;
            self.require_type(value, ScalarType::Bool)?;
            value
        } else {
            let value = self.graph.constant(1);
            self.mark_bool(value)
        };
        let true_jump = self.graph.jump(self.control);

        let false_projection = self.graph.projection(branch, 1);
        self.control = false_projection;
        let false_value = if conjunction {
            let value = self.graph.constant(0);
            self.mark_bool(value)
        } else {
            let value = self.parse_logical_and()?;
            self.require_type(value, ScalarType::Bool)?;
            value
        };
        let false_jump = self.graph.jump(self.control);

        let controls = vec![true_jump, false_jump];
        self.control = self.graph.region(controls.clone());
        let phi = self.graph.phi(controls, vec![true_value, false_value]);
        Ok(self.mark_bool(phi))
    }

    fn parse_if_expression(&mut self) -> Result<NodeId, ParseError> {
        self.position += 1;
        let condition = self.parse_expression()?;
        self.require_type(condition, ScalarType::Bool)?;
        let branch = self.graph.if_(self.control, condition);
        self.expect(TokenKind::LBrace, "`{` after if condition")?;

        let true_projection = self.graph.projection(branch, 0);
        self.control = true_projection;
        let true_value = self.parse_expression()?;
        let true_jump = self.graph.jump(self.control);
        self.expect(TokenKind::RBrace, "`}` after if branch")?;

        if !self.peek_identifier("else") {
            return Err(self.error_here("expected `else` after if branch"));
        }
        self.position += 1;
        self.expect(TokenKind::LBrace, "`{` after else")?;
        let false_projection = self.graph.projection(branch, 1);
        self.control = false_projection;
        let false_value = self.parse_expression()?;
        let false_jump = self.graph.jump(self.control);
        self.expect(TokenKind::RBrace, "`}` after else branch")?;

        let ty = self.type_of(true_value);
        self.require_type(false_value, ty)?;
        let controls = vec![true_jump, false_jump];
        let region = self.graph.region(controls.clone());
        self.control = region;
        let phi = self.graph.phi(controls, vec![true_value, false_value]);
        self.types.insert(phi, ty);
        Ok(phi)
    }

    fn parse_comparison(&mut self) -> Result<NodeId, ParseError> {
        let left = self.parse_bitwise_or()?;
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
        let right = self.parse_bitwise_or()?;
        if self.is_comparison(self.peek()) {
            return Err(self.error_here("chained comparisons are not supported"));
        }
        self.require_type(left, ScalarType::I64)?;
        self.require_type(right, ScalarType::I64)?;
        let comparison = self.graph.compare(operation, left, right);
        Ok(self.mark_bool(comparison))
    }

    fn integer_binary(
        &mut self,
        operation: BinaryOp,
        left: NodeId,
        right: NodeId,
    ) -> Result<NodeId, ParseError> {
        self.require_type(left, ScalarType::I64)?;
        self.require_type(right, ScalarType::I64)?;
        Ok(self.graph.binary(operation, left, right))
    }

    fn parse_bitwise_or(&mut self) -> Result<NodeId, ParseError> {
        let mut value = self.parse_bitwise_xor()?;
        while self.eat(TokenKind::Or) {
            let right = self.parse_bitwise_xor()?;
            value = self.integer_binary(BinaryOp::Or, value, right)?;
        }
        Ok(value)
    }

    fn parse_bitwise_xor(&mut self) -> Result<NodeId, ParseError> {
        let mut value = self.parse_bitwise_and()?;
        while self.eat(TokenKind::Xor) {
            let right = self.parse_bitwise_and()?;
            value = self.integer_binary(BinaryOp::Xor, value, right)?;
        }
        Ok(value)
    }

    fn parse_bitwise_and(&mut self) -> Result<NodeId, ParseError> {
        let mut value = self.parse_shift()?;
        while self.eat(TokenKind::And) {
            let right = self.parse_shift()?;
            value = self.integer_binary(BinaryOp::And, value, right)?;
        }
        Ok(value)
    }

    fn parse_shift(&mut self) -> Result<NodeId, ParseError> {
        let mut value = self.parse_additive()?;
        loop {
            let operation = match self.peek().kind {
                TokenKind::Shl => BinaryOp::Shl,
                TokenKind::Shr => BinaryOp::Shr,
                _ => return Ok(value),
            };
            self.position += 1;
            let right = self.parse_additive()?;
            value = self.integer_binary(operation, value, right)?;
        }
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
            value = self.integer_binary(operation, value, right)?;
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
            value = self.integer_binary(operation, value, right)?;
        }
    }

    fn parse_unary(&mut self) -> Result<NodeId, ParseError> {
        if self.eat(TokenKind::Not) {
            let operand = self.parse_unary()?;
            self.require_type(operand, ScalarType::Bool)?;
            let zero = self.graph.constant(0);
            let inversion = self.graph.compare(CompareOp::Eq, operand, zero);
            return Ok(self.mark_bool(inversion));
        }
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
        self.require_type(operand, ScalarType::I64)?;
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
                    self.position += 1;
                    let mut arguments = Vec::new();
                    if !self.check(&TokenKind::RParen) {
                        loop {
                            arguments.push(self.parse_expression()?);
                            if !self.eat(TokenKind::Comma) {
                                break;
                            }
                            if self.check(&TokenKind::RParen) {
                                break;
                            }
                        }
                    }
                    self.expect(TokenKind::RParen, "`)` after function arguments")?;
                    if !self.functions.contains(&identifier) {
                        return Err(ParseError::new(
                            token.span,
                            format!("unknown function `{identifier}`"),
                        ));
                    }
                    let signature = self.signatures[&identifier].clone();
                    if arguments.len() != signature.parameter_types.len() {
                        return Err(ParseError::new(
                            token.span,
                            format!(
                                "function `{identifier}` expects {} arguments, found {}",
                                signature.parameter_types.len(),
                                arguments.len()
                            ),
                        ));
                    }
                    for (argument, expected) in arguments.iter().zip(signature.parameter_types) {
                        self.require_type(*argument, expected)?;
                    }
                    let return_type = signature.return_type.ok_or_else(|| {
                        ParseError::new(
                            token.span,
                            format!(
                                "unit-returning function `{identifier}` cannot be used as a value"
                            ),
                        )
                    })?;
                    let target = self
                        .graph
                        .functions
                        .iter()
                        .position(|function| function.name == identifier)
                        .unwrap_or(self.graph.functions.len());
                    let call = self.graph.add_node(
                        crate::son::NodeKind::Call(target),
                        vec![self.control],
                        arguments,
                    );
                    self.types.insert(call, return_type);
                    return Ok(call);
                }
                if identifier == "true" {
                    let value = self.graph.constant(1);
                    return Ok(self.mark_bool(value));
                }
                if identifier == "false" {
                    let value = self.graph.constant(0);
                    return Ok(self.mark_bool(value));
                }
                self.variables
                    .get(&identifier)
                    .map(|binding| binding.value)
                    .ok_or_else(|| {
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
            error("fn main() { return missing(); }")
                .message
                .contains("unknown function")
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
    fn parses_mutation_and_loop_phis() {
        let parsed = graph(
            "fn main() -> i64 { let mut sum = 0; let mut i = 0; while i < 10 { sum = sum + i; i = i + 1; } return sum; }",
        );
        let header = parsed
            .iter()
            .find(|(_, node)| node.kind == NodeKind::Region && node.control.len() == 2)
            .expect("loop header");
        assert_eq!(
            parsed
                .iter()
                .filter(|(_, node)| node.kind == NodeKind::Phi && node.control == header.1.control)
                .count(),
            2
        );
        graph("fn main() { let mut x = 0; while x < 2 { let y = x + 1; x = y; } return; }");
    }

    #[test]
    fn rejects_invalid_mutation_and_non_i64_types() {
        assert!(
            error("fn main() { let x = 1; x = 2; return x; }")
                .message
                .contains("immutable")
        );
        assert!(
            error("fn main() { missing = 2; }")
                .message
                .contains("unknown variable")
        );
        assert!(
            error("fn main() { let mut x = 0; while x < 2 { return; } }")
                .message
                .contains("conditional or loop")
        );
        assert!(
            error("fn main() { while false { let local = 1; } return local; }")
                .message
                .contains("unknown variable")
        );
        assert!(
            error("fn main() -> i64 { return; }")
                .message
                .contains("expected a `i64` expression")
        );
        assert!(
            error("fn f(x: i32) -> i64 { return x; }")
                .message
                .contains("only the `i64` and `bool` types")
        );
        assert!(
            error("fn main() -> u32 { return 1; }")
                .message
                .contains("only the `i64` and `bool` types")
        );
    }

    #[test]
    fn rejects_missing_return_and_unreachable_statements() {
        assert!(
            error("fn main() -> i64 { let x = 1; }")
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
                .contains("assignment target")
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
            error("fn main() { return 0b10; }")
                .message
                .contains("literal")
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
            error("fn main() { loop { } }")
                .message
                .contains("assignment target")
        );
        assert!(
            error("fn main() { return 1 && 0; }")
                .message
                .contains("type mismatch")
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
