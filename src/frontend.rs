use crate::son::{BinaryOp, CompareOp, FunctionMetadata, Graph, NodeId, NodeKind, UnaryOp};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

/// Parses the explicitly supported `i64` (`isize`), `u64` (`usize`), and `bool` Rust subset.
///
/// Functions and parameters retain their names and source ranges in `Graph::functions`.
/// `if` expressions and statements and scalar `match` expressions merge values through explicit
/// jumps, control regions, and Phi nodes. Mutable bindings and `while` loops use loop-header Phis,
/// multiple back edges from `continue`, and exit Phis for `break` snapshots.
/// A pre-scan registers scalar constant declarations and function signatures, supporting forward
/// constant references and recursive calls with checked types. Constant expressions are evaluated
/// without emitting runtime nodes. Comparisons and logical operations produce booleans; `&&` and
/// `||` evaluate their right operand conditionally. Unsupported Rust is rejected with a source
/// location.
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
    I64Integer(u64),
    U64Integer(u64),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Semicolon,
    Arrow,
    FatArrow,
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
    AndEq,
    OrEq,
    XorEq,
    ShlEq,
    ShrEq,
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
    DotDot,
    DotDotEq,
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
        let radix = if self.rest().starts_with("0x") || self.rest().starts_with("0X") {
            self.advance();
            self.advance();
            16
        } else if self.rest().starts_with("0o") || self.rest().starts_with("0O") {
            self.advance();
            self.advance();
            8
        } else if self.rest().starts_with("0b") || self.rest().starts_with("0B") {
            self.advance();
            self.advance();
            2
        } else {
            10
        };
        let digits_start = self.offset;
        while self
            .current_char()
            .is_some_and(|ch| ch == '_' || ch.is_digit(radix))
        {
            self.advance();
        }
        let digits_end = self.offset;
        while self.current_char().is_some_and(is_ident_continue) {
            self.advance();
        }
        let literal_span = Span {
            end: self.offset,
            ..start
        };
        let suffix = &self.source[digits_end..self.offset];
        if !matches!(suffix, "" | "i64" | "isize" | "u64" | "usize") {
            return Err(ParseError::new(
                literal_span,
                format!(
                    "unsupported integer literal suffix `{suffix}`; only i64/isize/u64/usize literals are supported"
                ),
            ));
        }
        let digits: String = self.source[digits_start..digits_end]
            .chars()
            .filter(|ch| *ch != '_')
            .collect();
        if digits.is_empty() {
            return Err(ParseError::new(
                literal_span,
                "integer literal has no digits",
            ));
        }
        let value = u64::from_str_radix(&digits, radix).map_err(|_| {
            ParseError::new(
                literal_span,
                "integer literal does not fit in the supported 64-bit magnitude range",
            )
        })?;
        Ok(match suffix {
            "" => TokenKind::Integer(value),
            "i64" | "isize" => TokenKind::I64Integer(value),
            "u64" | "usize" => TokenKind::U64Integer(value),
            _ => unreachable!(),
        })
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
                } else if self.rest().starts_with('>') {
                    self.advance();
                    TokenKind::FatArrow
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
                if self.rest().starts_with("<=") {
                    self.advance();
                    self.advance();
                    TokenKind::ShlEq
                } else if self.rest().starts_with('<') {
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
                if self.rest().starts_with(">=") {
                    self.advance();
                    self.advance();
                    TokenKind::ShrEq
                } else if self.rest().starts_with('>') {
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
                } else if self.rest().starts_with('=') {
                    self.advance();
                    TokenKind::AndEq
                } else {
                    TokenKind::And
                }
            }
            Some('|') => {
                if self.rest().starts_with('|') {
                    self.advance();
                    TokenKind::OrOr
                } else if self.rest().starts_with('=') {
                    self.advance();
                    TokenKind::OrEq
                } else {
                    TokenKind::Or
                }
            }
            Some('^') => {
                if self.rest().starts_with('=') {
                    self.advance();
                    TokenKind::XorEq
                } else {
                    TokenKind::Xor
                }
            }
            Some('.') => {
                if self.rest().starts_with(".=") {
                    self.advance();
                    self.advance();
                    TokenKind::DotDotEq
                } else if self.rest().starts_with('.') {
                    self.advance();
                    TokenKind::DotDot
                } else {
                    return Err(ParseError::new(
                        start,
                        "floating-point literals and member access are not supported",
                    ));
                }
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
            | "isize"
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
            | "u64"
            | "union"
            | "unsafe"
            | "unsized"
            | "usize"
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
    U64,
    Bool,
    Unit,
}

impl fmt::Display for ScalarType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::I64 => formatter.write_str("i64"),
            Self::U64 => formatter.write_str("u64"),
            Self::Bool => formatter.write_str("bool"),
            Self::Unit => formatter.write_str("()"),
        }
    }
}

#[derive(Clone)]
struct FunctionSignature {
    parameter_types: Vec<ScalarType>,
    return_type: Option<ScalarType>,
}

struct FunctionHeader {
    name: String,
    name_span: Span,
    parameters: Vec<(String, bool, ScalarType)>,
    return_type: Option<ScalarType>,
}

#[derive(Clone, Copy)]
struct Constant {
    value: i64,
    ty: ScalarType,
}

#[derive(Clone)]
struct ConstantItem {
    name: String,
    name_span: Span,
    ty: ScalarType,
    expression_start: usize,
    expression_end: usize,
}

#[derive(Clone, Copy)]
struct ConstantValue {
    value: i64,
    ty: ScalarType,
    unsuffixed: bool,
}

#[derive(Clone, Copy)]
struct Binding {
    value: NodeId,
    mutable: bool,
    ty: ScalarType,
}

#[derive(Clone, Copy)]
enum MatchPattern {
    Literal {
        value: NodeId,
        bits: i64,
    },
    Range {
        start: NodeId,
        end: NodeId,
        start_bits: i64,
        end_bits: i64,
    },
    Wildcard,
}

struct LoopContext {
    header: NodeId,
    phis: Vec<(String, NodeId)>,
    breaks: Vec<(NodeId, HashMap<String, Binding>)>,
    induction: Option<(String, ScalarType)>,
    inclusive_end: Option<String>,
    shadow_depth: usize,
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
    graph: Graph,
    functions: HashSet<String>,
    function_indices: HashMap<String, usize>,
    signatures: HashMap<String, FunctionSignature>,
    constants: HashMap<String, Constant>,
    constant_items: HashMap<String, ConstantItem>,
    constant_order: Vec<String>,
    evaluating_constants: HashSet<String>,
    const_evaluation: bool,
    variables: HashMap<String, Binding>,
    shadowed_variables: Vec<(String, Binding)>,
    types: HashMap<NodeId, ScalarType>,
    unsuffixed_literals: HashSet<NodeId>,
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
            function_indices: HashMap::new(),
            signatures: HashMap::new(),
            constants: HashMap::new(),
            constant_items: HashMap::new(),
            constant_order: Vec::new(),
            evaluating_constants: HashSet::new(),
            const_evaluation: true,
            variables: HashMap::new(),
            shadowed_variables: Vec::new(),
            types: HashMap::new(),
            unsuffixed_literals: HashSet::new(),
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
            if !self.check(&TokenKind::Fn)
                && !self.peek_identifier("pub")
                && !self.peek_identifier("const")
            {
                return Err(self.error_here("expected `fn`; this item is not supported"));
            }
            if self.const_item_ahead() {
                let item = self.scan_const_item()?;
                let name = item.name.clone();
                if self.functions.contains(&name) || self.constant_items.contains_key(&name) {
                    return Err(ParseError::new(
                        item.name_span,
                        format!(
                            "duplicate item `{name}`; functions and constants must have unique names"
                        ),
                    ));
                }
                self.constant_order.push(name.clone());
                self.constant_items.insert(name, item);
                continue;
            }
            let header = self.parse_function_header()?;
            self.expect(TokenKind::LBrace, "`{` before function body")?;
            let mut depth = 1usize;
            while depth != 0 {
                match self.advance().kind {
                    TokenKind::LBrace => depth += 1,
                    TokenKind::RBrace => depth -= 1,
                    TokenKind::Eof => {
                        return Err(self.error_here(format!(
                            "unterminated body of function `{}`",
                            header.name
                        )));
                    }
                    _ => {}
                }
            }
            if self.constant_items.contains_key(&header.name) {
                return Err(ParseError::new(
                    header.name_span,
                    format!(
                        "duplicate item `{}`; functions and constants must have unique names",
                        header.name
                    ),
                ));
            }
            if !self.functions.insert(header.name.clone()) {
                return Err(ParseError::new(
                    header.name_span,
                    format!("duplicate function `{}`", header.name),
                ));
            }
            let index = self.function_indices.len();
            self.function_indices.insert(header.name.clone(), index);
            self.signatures.insert(
                header.name,
                FunctionSignature {
                    parameter_types: header.parameters.iter().map(|(_, _, ty)| *ty).collect(),
                    return_type: header.return_type,
                },
            );
        }
        if self.functions.is_empty() {
            return Err(self.error_here("expected at least one function"));
        }
        for name in self.constant_order.clone() {
            let name_span = self.constant_items[&name].name_span;
            self.evaluate_constant(&name, name_span)?;
        }
        self.position = 0;
        while !self.check(&TokenKind::Eof) {
            if self.const_item_ahead() {
                self.scan_const_item()?;
            } else {
                self.parse_function()?;
            }
        }
        debug_assert!(
            self.graph.validate().is_ok(),
            "frontend emitted an invalid graph"
        );
        Ok(self.graph)
    }

    fn const_item_ahead(&self) -> bool {
        let mut position = self.position;
        if matches!(
            self.tokens.get(position).map(|token| &token.kind),
            Some(TokenKind::Ident(identifier)) if identifier == "pub"
        ) {
            position += 1;
            if self
                .tokens
                .get(position)
                .is_some_and(|token| token.kind == TokenKind::LParen)
            {
                position += 1;
                if !matches!(
                    self.tokens.get(position).map(|token| &token.kind),
                    Some(TokenKind::Ident(path)) if matches!(path.as_str(), "crate" | "self" | "super")
                ) {
                    return false;
                }
                position += 1;
                if !self
                    .tokens
                    .get(position)
                    .is_some_and(|token| token.kind == TokenKind::RParen)
                {
                    return false;
                }
                position += 1;
            }
        }
        matches!(
            self.tokens.get(position).map(|token| &token.kind),
            Some(TokenKind::Ident(identifier)) if identifier == "const"
        ) && !self
            .tokens
            .get(position + 1)
            .is_some_and(|token| token.kind == TokenKind::Fn)
    }

    fn scan_const_item(&mut self) -> Result<ConstantItem, ParseError> {
        self.parse_function_visibility()?;
        let const_token = self.advance();
        debug_assert!(matches!(
            const_token.kind,
            TokenKind::Ident(identifier) if identifier == "const"
        ));
        let (name, name_span) = self.parse_identifier("constant name")?;
        self.expect(TokenKind::Colon, "`:` and explicit constant type")?;
        let ty = self.parse_type()?;
        self.expect(TokenKind::Eq, "`=` in constant item")?;
        let expression_start = self.position;
        let mut parentheses = 0usize;
        loop {
            if self.check(&TokenKind::Semicolon) {
                break;
            }
            if self.check(&TokenKind::Eof)
                || (parentheses == 0
                    && (self.check(&TokenKind::Fn)
                        || self.peek_identifier("pub")
                        || self.peek_identifier("const")))
            {
                return Err(self.error_here("expected `;` after constant item"));
            }
            match self.peek().kind {
                TokenKind::LParen => {
                    parentheses = parentheses.checked_add(1).ok_or_else(|| {
                        self.error_here("constant expression parentheses are nested too deeply")
                    })?;
                }
                TokenKind::RParen => parentheses = parentheses.saturating_sub(1),
                _ => {}
            }
            self.advance();
        }
        let expression_end = self.position;
        self.advance();
        Ok(ConstantItem {
            name,
            name_span,
            ty,
            expression_start,
            expression_end,
        })
    }

    fn evaluate_constant(
        &mut self,
        name: &str,
        reference_span: Span,
    ) -> Result<Constant, ParseError> {
        if let Some(constant) = self.constants.get(name).copied() {
            return Ok(constant);
        }
        let Some(item) = self.constant_items.get(name).cloned() else {
            return Err(ParseError::new(
                reference_span,
                format!("unknown constant `{name}`"),
            ));
        };
        if !self.evaluating_constants.insert(name.to_owned()) {
            return Err(ParseError::new(
                reference_span,
                format!("cyclic constant reference `{name}`"),
            ));
        }

        let saved_position = self.position;
        let saved_evaluation = self.const_evaluation;
        self.position = item.expression_start;
        self.const_evaluation = saved_evaluation;
        let result = (|| {
            let value = self.parse_const_logical_or(Some(item.ty))?;
            if self.position != item.expression_end {
                return Err(self.error_here("expected `;` after constant expression"));
            }
            let found = value.ty;
            let value = self
                .require_const_type(value, item.ty, self.peek().span)
                .map_err(|error| {
                    if error.message.starts_with("type mismatch") {
                        ParseError::new(
                            item.name_span,
                            format!(
                                "type mismatch in constant `{}`: expected `{}`, found `{found}`",
                                item.name, item.ty
                            ),
                        )
                    } else {
                        error
                    }
                })?;
            Ok(Constant {
                value: value.value,
                ty: value.ty,
            })
        })();
        self.position = saved_position;
        self.const_evaluation = saved_evaluation;
        self.evaluating_constants.remove(name);
        let constant = result?;
        if saved_evaluation {
            self.constants.insert(name.to_owned(), constant);
        }
        Ok(constant)
    }

    fn require_const_type(
        &self,
        mut value: ConstantValue,
        expected: ScalarType,
        span: Span,
    ) -> Result<ConstantValue, ParseError> {
        if expected == ScalarType::I64 && value.unsuffixed && value.value < 0 {
            return Err(ParseError::new(
                span,
                "integer literal is outside the positive i64 range",
            ));
        }
        if value.ty == expected {
            return Ok(value);
        }
        if value.ty == ScalarType::I64 && expected == ScalarType::U64 && value.unsuffixed {
            value.ty = ScalarType::U64;
            value.unsuffixed = false;
            return Ok(value);
        }
        Err(ParseError::new(
            span,
            format!("type mismatch: expected `{expected}`, found `{}`", value.ty),
        ))
    }

    fn const_numeric_pair(
        &self,
        mut left: ConstantValue,
        mut right: ConstantValue,
        span: Span,
    ) -> Result<(ConstantValue, ConstantValue, ScalarType), ParseError> {
        if left.unsuffixed && right.ty == ScalarType::U64 {
            left = self.require_const_type(left, ScalarType::U64, span)?;
        }
        if right.unsuffixed && left.ty == ScalarType::U64 {
            right = self.require_const_type(right, ScalarType::U64, span)?;
        }
        let ty = left.ty;
        if matches!(ty, ScalarType::Bool | ScalarType::Unit) {
            return Err(ParseError::new(
                span,
                format!("expected integer operands, found `{ty}`"),
            ));
        }
        left = self.require_const_type(left, ty, span)?;
        right = self.require_const_type(right, ty, span)?;
        Ok((left, right, ty))
    }

    fn const_integer_binary(
        &self,
        operation: BinaryOp,
        left: ConstantValue,
        right: ConstantValue,
        span: Span,
    ) -> Result<ConstantValue, ParseError> {
        let (left, right, ty) = self.const_numeric_pair(left, right, span)?;
        if !self.const_evaluation {
            return Ok(ConstantValue {
                value: 0,
                ty,
                unsuffixed: false,
            });
        }
        let overflow = || ParseError::new(span, "arithmetic overflow in constant expression");
        let divide_by_zero = || ParseError::new(span, "division by zero in constant expression");
        let invalid_shift = || {
            ParseError::new(
                span,
                "invalid shift count in constant expression; expected a value in 0..64",
            )
        };
        let value = match ty {
            ScalarType::I64 => match operation {
                BinaryOp::Add => left.value.checked_add(right.value).ok_or_else(overflow)?,
                BinaryOp::Sub => left.value.checked_sub(right.value).ok_or_else(overflow)?,
                BinaryOp::Mul => left.value.checked_mul(right.value).ok_or_else(overflow)?,
                BinaryOp::Div => {
                    if right.value == 0 {
                        return Err(divide_by_zero());
                    }
                    left.value.checked_div(right.value).ok_or_else(overflow)?
                }
                BinaryOp::Rem => {
                    if right.value == 0 {
                        return Err(divide_by_zero());
                    }
                    left.value.checked_rem(right.value).ok_or_else(overflow)?
                }
                BinaryOp::And => left.value & right.value,
                BinaryOp::Or => left.value | right.value,
                BinaryOp::Xor => left.value ^ right.value,
                BinaryOp::Shl | BinaryOp::Shr => {
                    if right.value < 0 || right.value >= 64 {
                        return Err(invalid_shift());
                    }
                    let shift = right.value as u32;
                    if operation == BinaryOp::Shl {
                        left.value.wrapping_shl(shift)
                    } else {
                        left.value >> shift
                    }
                }
                _ => return Err(ParseError::new(span, "invalid integer constant operation")),
            },
            ScalarType::U64 => {
                let left = left.value as u64;
                let right = right.value as u64;
                match operation {
                    BinaryOp::Add => left.checked_add(right).ok_or_else(overflow)? as i64,
                    BinaryOp::Sub => left.checked_sub(right).ok_or_else(overflow)? as i64,
                    BinaryOp::Mul => left.checked_mul(right).ok_or_else(overflow)? as i64,
                    BinaryOp::Div => {
                        if right == 0 {
                            return Err(divide_by_zero());
                        }
                        (left / right) as i64
                    }
                    BinaryOp::Rem => {
                        if right == 0 {
                            return Err(divide_by_zero());
                        }
                        (left % right) as i64
                    }
                    BinaryOp::And => (left & right) as i64,
                    BinaryOp::Or => (left | right) as i64,
                    BinaryOp::Xor => (left ^ right) as i64,
                    BinaryOp::Shl | BinaryOp::Shr => {
                        if right >= 64 {
                            return Err(invalid_shift());
                        }
                        let shift = right as u32;
                        if operation == BinaryOp::Shl {
                            left.wrapping_shl(shift) as i64
                        } else {
                            (left >> shift) as i64
                        }
                    }
                    _ => return Err(ParseError::new(span, "invalid integer constant operation")),
                }
            }
            _ => unreachable!("numeric pair excludes non-integer operands"),
        };
        Ok(ConstantValue {
            value,
            ty,
            unsuffixed: false,
        })
    }

    fn parse_const_logical_or(
        &mut self,
        hint: Option<ScalarType>,
    ) -> Result<ConstantValue, ParseError> {
        let mut left = self.parse_const_logical_and(hint)?;
        while self.eat(TokenKind::OrOr) {
            let span = self.peek().span;
            left = self.require_const_type(left, ScalarType::Bool, span)?;
            let evaluating = self.const_evaluation;
            self.const_evaluation = evaluating && left.value == 0;
            let parsed = self.parse_const_logical_and(Some(ScalarType::Bool));
            self.const_evaluation = evaluating;
            let right = self.require_const_type(parsed?, ScalarType::Bool, span)?;
            left = ConstantValue {
                value: if evaluating && left.value != 0 {
                    1
                } else if evaluating {
                    right.value
                } else {
                    0
                },
                ty: ScalarType::Bool,
                unsuffixed: false,
            };
        }
        Ok(left)
    }

    fn parse_const_logical_and(
        &mut self,
        hint: Option<ScalarType>,
    ) -> Result<ConstantValue, ParseError> {
        let mut left = self.parse_const_comparison(hint)?;
        while self.eat(TokenKind::AndAnd) {
            let span = self.peek().span;
            left = self.require_const_type(left, ScalarType::Bool, span)?;
            let evaluating = self.const_evaluation;
            self.const_evaluation = evaluating && left.value != 0;
            let parsed = self.parse_const_comparison(Some(ScalarType::Bool));
            self.const_evaluation = evaluating;
            let right = self.require_const_type(parsed?, ScalarType::Bool, span)?;
            left = ConstantValue {
                value: if evaluating && left.value == 0 {
                    0
                } else if evaluating {
                    right.value
                } else {
                    0
                },
                ty: ScalarType::Bool,
                unsuffixed: false,
            };
        }
        Ok(left)
    }

    fn parse_const_comparison(
        &mut self,
        hint: Option<ScalarType>,
    ) -> Result<ConstantValue, ParseError> {
        let operand_hint = hint.filter(|ty| matches!(ty, ScalarType::I64 | ScalarType::U64));
        let left = self.parse_const_bitwise_or(operand_hint)?;
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
        let span = self.advance().span;
        let right = self.parse_const_bitwise_or(Some(left.ty))?;
        if self.is_comparison(self.peek()) {
            return Err(self.error_here("chained comparisons are not supported"));
        }
        let ty = if matches!(operation, CompareOp::Eq | CompareOp::Ne)
            && (left.ty == ScalarType::Bool || right.ty == ScalarType::Bool)
        {
            self.require_const_type(left, ScalarType::Bool, span)?;
            self.require_const_type(right, ScalarType::Bool, span)?;
            ScalarType::Bool
        } else {
            self.const_numeric_pair(left, right, span)?.2
        };
        if !self.const_evaluation {
            return Ok(ConstantValue {
                value: 0,
                ty: ScalarType::Bool,
                unsuffixed: false,
            });
        }
        let value = match ty {
            ScalarType::Bool => match operation {
                CompareOp::Eq => left.value == right.value,
                CompareOp::Ne => left.value != right.value,
                _ => unreachable!("boolean ordering is rejected above"),
            },
            ScalarType::I64 => match operation {
                CompareOp::Eq => left.value == right.value,
                CompareOp::Ne => left.value != right.value,
                CompareOp::Lt => left.value < right.value,
                CompareOp::Le => left.value <= right.value,
                CompareOp::Gt => left.value > right.value,
                CompareOp::Ge => left.value >= right.value,
                _ => unreachable!("frontend only parses signed comparisons"),
            },
            ScalarType::U64 => {
                let left = left.value as u64;
                let right = right.value as u64;
                match operation {
                    CompareOp::Eq => left == right,
                    CompareOp::Ne => left != right,
                    CompareOp::Lt => left < right,
                    CompareOp::Le => left <= right,
                    CompareOp::Gt => left > right,
                    CompareOp::Ge => left >= right,
                    _ => unreachable!("frontend only parses unsigned comparisons"),
                }
            }
            ScalarType::Unit => unreachable!("unit constants are not supported"),
        };
        Ok(ConstantValue {
            value: i64::from(value),
            ty: ScalarType::Bool,
            unsuffixed: false,
        })
    }

    fn parse_const_bitwise_or(
        &mut self,
        hint: Option<ScalarType>,
    ) -> Result<ConstantValue, ParseError> {
        let mut value = self.parse_const_bitwise_xor(hint)?;
        while self.eat(TokenKind::Or) {
            let span = self.peek().span;
            let right = self.parse_const_bitwise_xor(Some(value.ty))?;
            value = self.const_integer_binary(BinaryOp::Or, value, right, span)?;
        }
        Ok(value)
    }

    fn parse_const_bitwise_xor(
        &mut self,
        hint: Option<ScalarType>,
    ) -> Result<ConstantValue, ParseError> {
        let mut value = self.parse_const_bitwise_and(hint)?;
        while self.eat(TokenKind::Xor) {
            let span = self.peek().span;
            let right = self.parse_const_bitwise_and(Some(value.ty))?;
            value = self.const_integer_binary(BinaryOp::Xor, value, right, span)?;
        }
        Ok(value)
    }

    fn parse_const_bitwise_and(
        &mut self,
        hint: Option<ScalarType>,
    ) -> Result<ConstantValue, ParseError> {
        let mut value = self.parse_const_shift(hint)?;
        while self.eat(TokenKind::And) {
            let span = self.peek().span;
            let right = self.parse_const_shift(Some(value.ty))?;
            value = self.const_integer_binary(BinaryOp::And, value, right, span)?;
        }
        Ok(value)
    }

    fn parse_const_shift(&mut self, hint: Option<ScalarType>) -> Result<ConstantValue, ParseError> {
        let mut value = self.parse_const_additive(hint)?;
        loop {
            let operation = match self.peek().kind {
                TokenKind::Shl => BinaryOp::Shl,
                TokenKind::Shr => BinaryOp::Shr,
                _ => return Ok(value),
            };
            self.position += 1;
            let span = self.peek().span;
            let right = self.parse_const_additive(Some(value.ty))?;
            value = self.const_integer_binary(operation, value, right, span)?;
        }
    }

    fn parse_const_additive(
        &mut self,
        hint: Option<ScalarType>,
    ) -> Result<ConstantValue, ParseError> {
        let mut value = self.parse_const_multiplicative(hint)?;
        loop {
            let operation = match self.peek().kind {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => return Ok(value),
            };
            self.position += 1;
            let span = self.peek().span;
            let right = self.parse_const_multiplicative(Some(value.ty))?;
            value = self.const_integer_binary(operation, value, right, span)?;
        }
    }

    fn parse_const_multiplicative(
        &mut self,
        hint: Option<ScalarType>,
    ) -> Result<ConstantValue, ParseError> {
        let mut value = self.parse_const_unary(hint)?;
        loop {
            let operation = match self.peek().kind {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Rem,
                _ => return Ok(value),
            };
            self.position += 1;
            let span = self.peek().span;
            let right = self.parse_const_unary(Some(value.ty))?;
            value = self.const_integer_binary(operation, value, right, span)?;
        }
    }

    fn parse_const_unary(&mut self, hint: Option<ScalarType>) -> Result<ConstantValue, ParseError> {
        if self.eat(TokenKind::Not) {
            let span = self.peek().span;
            let operand = self.parse_const_unary(hint)?;
            let operand = self.require_const_type(operand, operand.ty, span)?;
            if !self.const_evaluation {
                return Ok(ConstantValue {
                    value: 0,
                    ty: operand.ty,
                    unsuffixed: false,
                });
            }
            return Ok(ConstantValue {
                value: if operand.ty == ScalarType::Bool {
                    i64::from(operand.value == 0)
                } else {
                    !operand.value
                },
                ty: operand.ty,
                unsuffixed: false,
            });
        }
        if !self.eat(TokenKind::Minus) {
            return self.parse_const_primary(hint);
        }
        if let TokenKind::Integer(magnitude) | TokenKind::I64Integer(magnitude) = self.peek().kind {
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
            return Ok(ConstantValue {
                value,
                ty: ScalarType::I64,
                unsuffixed: false,
            });
        }
        let span = self.peek().span;
        let operand = self.parse_const_unary(Some(ScalarType::I64))?;
        let operand = self.require_const_type(operand, ScalarType::I64, span)?;
        if !self.const_evaluation {
            return Ok(ConstantValue {
                value: 0,
                ty: ScalarType::I64,
                unsuffixed: false,
            });
        }
        Ok(ConstantValue {
            value: operand.value.checked_neg().ok_or_else(|| {
                ParseError::new(span, "arithmetic overflow in constant expression")
            })?,
            ty: ScalarType::I64,
            unsuffixed: false,
        })
    }

    fn parse_const_primary(
        &mut self,
        hint: Option<ScalarType>,
    ) -> Result<ConstantValue, ParseError> {
        let token = self.advance();
        match token.kind {
            TokenKind::Integer(value) => {
                let unsigned = hint == Some(ScalarType::U64);
                Ok(ConstantValue {
                    value: value as i64,
                    ty: if unsigned {
                        ScalarType::U64
                    } else {
                        ScalarType::I64
                    },
                    unsuffixed: !unsigned,
                })
            }
            TokenKind::I64Integer(value) => {
                if value > i64::MAX as u64 {
                    return Err(ParseError::new(
                        token.span,
                        "integer literal is outside the positive i64 range",
                    ));
                }
                Ok(ConstantValue {
                    value: value as i64,
                    ty: ScalarType::I64,
                    unsuffixed: false,
                })
            }
            TokenKind::U64Integer(value) => Ok(ConstantValue {
                value: value as i64,
                ty: ScalarType::U64,
                unsuffixed: false,
            }),
            TokenKind::Ident(identifier) if identifier == "true" => Ok(ConstantValue {
                value: 1,
                ty: ScalarType::Bool,
                unsuffixed: false,
            }),
            TokenKind::Ident(identifier) if identifier == "false" => Ok(ConstantValue {
                value: 0,
                ty: ScalarType::Bool,
                unsuffixed: false,
            }),
            TokenKind::Ident(identifier) => {
                if self.check(&TokenKind::LParen) {
                    return Err(ParseError::new(
                        token.span,
                        format!(
                            "function call `{identifier}` is not supported in a constant expression"
                        ),
                    ));
                }
                let constant = self.evaluate_constant(&identifier, token.span)?;
                Ok(ConstantValue {
                    value: constant.value,
                    ty: constant.ty,
                    unsuffixed: false,
                })
            }
            TokenKind::LParen => {
                let value = self.parse_const_logical_or(hint)?;
                self.expect(
                    TokenKind::RParen,
                    "`)` after parenthesized constant expression",
                )?;
                Ok(value)
            }
            TokenKind::Eof => Err(ParseError::new(
                token.span,
                "expected constant expression, found end of file",
            )),
            _ => Err(ParseError::new(token.span, "expected constant expression")),
        }
    }

    fn parse_function_header(&mut self) -> Result<FunctionHeader, ParseError> {
        self.parse_function_modifiers()?;
        self.expect(TokenKind::Fn, "`fn`")?;
        let (name, name_span) = self.parse_identifier("function name")?;
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
        Ok(FunctionHeader {
            name,
            name_span,
            parameters,
            return_type,
        })
    }

    fn parse_function_modifiers(&mut self) -> Result<(), ParseError> {
        self.parse_function_visibility()?;
        if !self.peek_identifier("const") {
            return Ok(());
        }

        let const_token = self.advance();
        if self.check(&TokenKind::Fn) {
            return Ok(());
        }
        if self.peek_identifier("pub") || self.peek_identifier("const") {
            return Err(ParseError::new(
                self.peek().span,
                "malformed function modifiers: expected visibility before a single `const`, followed by `fn`",
            ));
        }
        Err(ParseError::new(
            const_token.span,
            "malformed function modifiers: expected `fn` after `const`",
        ))
    }

    fn parse_function_visibility(&mut self) -> Result<(), ParseError> {
        if !self.peek_identifier("pub") {
            return Ok(());
        }
        self.advance();
        if !self.eat(TokenKind::LParen) {
            return Ok(());
        }

        let visibility = self.advance();
        if !matches!(
            &visibility.kind,
            TokenKind::Ident(path) if matches!(path.as_str(), "crate" | "self" | "super")
        ) {
            return Err(ParseError::new(
                visibility.span,
                "expected `crate`, `self`, or `super` as restricted function visibility",
            ));
        }
        self.expect(
            TokenKind::RParen,
            "`)` after restricted function visibility",
        )?;
        Ok(())
    }

    fn parse_function(&mut self) -> Result<(), ParseError> {
        let FunctionHeader {
            name,
            parameters,
            return_type,
            ..
        } = self.parse_function_header()?;
        self.return_type = return_type;
        self.expect(TokenKind::LBrace, "`{` before function body")?;
        let start = self.graph.start();
        let mut return_node = None;
        self.control = start;
        self.terminated = false;
        self.loops.clear();
        self.variables.clear();
        self.shadowed_variables.clear();
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
            if return_node.is_some() || self.terminated {
                return Err(self.error_here(
                    "statements after `return` are unsupported; `return` must end the body",
                ));
            }
            if self.is_tail_expression_candidate() {
                let value = self.parse_expression()?;
                if let Some(expected) = return_type {
                    self.require_type(value, expected)?;
                } else if self.unsuffixed_literals.contains(&value) {
                    self.require_type(value, ScalarType::I64)?;
                }
                return_node = Some(self.graph.return_(self.control, Some(value)));
                self.terminated = true;
                break;
            }
            if let Some(node) = self.parse_statement()? {
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
        if return_node.is_none() && return_type.is_some() && !self.terminated {
            return Err(ParseError::new(
                close,
                format!("function `{name}` must end with `return expr;`"),
            ));
        }
        let return_node = match return_node {
            Some(node) => node,
            None if self.terminated => self
                .graph
                .iter()
                .filter(|(_, node)| node.kind == NodeKind::Return)
                .map(|(id, _)| id)
                .last()
                .expect("terminated function contains a return"),
            None => self.graph.return_(self.control, None),
        };
        self.graph.functions.push(FunctionMetadata {
            name,
            start,
            return_node,
            parameter_count: parameters.len(),
        });
        Ok(())
    }

    fn is_tail_expression_candidate(&self) -> bool {
        if self.check(&TokenKind::Let)
            || self.check(&TokenKind::Return)
            || ["if", "for", "while", "loop", "break", "continue", "return"]
                .into_iter()
                .any(|keyword| self.peek_identifier(keyword))
        {
            return false;
        }
        let mut brace_depth = 0usize;
        for token in &self.tokens[self.position..] {
            match token.kind {
                TokenKind::LBrace => brace_depth += 1,
                TokenKind::RBrace if brace_depth == 0 => return true,
                TokenKind::RBrace => brace_depth -= 1,
                TokenKind::Semicolon => return false,
                TokenKind::Eof => return false,
                _ => {}
            }
        }
        false
    }

    fn parse_statement(&mut self) -> Result<Option<NodeId>, ParseError> {
        self.terminated = false;
        if self.eat(TokenKind::Let) {
            self.parse_let()?;
            return Ok(None);
        }
        if self.peek_identifier("if") {
            self.parse_if_statement()?;
            return Ok(None);
        }
        if self.peek_identifier("for") {
            self.parse_for()?;
            return Ok(None);
        }
        if self.peek_identifier("loop") {
            self.position += 1;
            let condition = self.graph.constant(1);
            self.mark_bool(condition);
            self.parse_loop(Some(condition), None, None)?;
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
            self.terminated = true;
            if self.eat(TokenKind::Semicolon) {
                if let Some(expected) = self.return_type {
                    return Err(self
                        .error_here(format!("expected a `{expected}` expression after `return`")));
                }
                return Ok(Some(self.graph.return_(self.control, None)));
            }
            let value = self.parse_expression()?;
            if let Some(expected) = self.return_type {
                self.require_type(value, expected)?;
            } else if self.unsuffixed_literals.contains(&value) {
                self.require_type(value, ScalarType::I64)?;
            }
            self.expect(TokenKind::Semicolon, "`;` after returned expression")?;
            return Ok(Some(self.graph.return_(self.control, Some(value))));
        }
        let is_assignment = matches!(self.peek().kind, TokenKind::Ident(_))
            && self.tokens.get(self.position + 1).is_some_and(|token| {
                matches!(
                    token.kind,
                    TokenKind::Eq
                        | TokenKind::PlusEq
                        | TokenKind::MinusEq
                        | TokenKind::StarEq
                        | TokenKind::SlashEq
                        | TokenKind::PercentEq
                        | TokenKind::AndEq
                        | TokenKind::OrEq
                        | TokenKind::XorEq
                        | TokenKind::ShlEq
                        | TokenKind::ShrEq
                )
            });
        if is_assignment {
            self.parse_assignment()?;
        } else {
            self.parse_expression()?;
            self.expect(TokenKind::Semicolon, "`;` after expression statement")?;
        }
        Ok(None)
    }

    fn parse_let(&mut self) -> Result<(), ParseError> {
        let mutable = if self.peek_identifier("mut") {
            self.position += 1;
            true
        } else {
            false
        };
        let (name, _) = self.parse_identifier("binding name")?;
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
        if let Some(binding) = self.variables.get(&name).copied() {
            self.shadowed_variables.push((name.clone(), binding));
        }
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
            TokenKind::AndEq => Some(BinaryOp::And),
            TokenKind::OrEq => Some(BinaryOp::Or),
            TokenKind::XorEq => Some(BinaryOp::Xor),
            TokenKind::ShlEq => Some(BinaryOp::Shl),
            TokenKind::ShrEq => Some(BinaryOp::Shr),
            _ => return Err(self.error_here("expected an assignment operator")),
        };
        let right = self.parse_expression()?;
        let value = match operation {
            Some(operation) => self.integer_binary(operation, binding.value, right)?,
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

    fn binding_before_shadows(&self, name: &str, shadow_depth: usize) -> Binding {
        self.shadowed_variables[shadow_depth..]
            .iter()
            .find(|(shadowed, _)| shadowed == name)
            .map(|(_, binding)| *binding)
            .unwrap_or(self.variables[name])
    }

    fn variables_before_shadows(
        &self,
        outer_variables: &HashMap<String, Binding>,
        shadow_depth: usize,
    ) -> HashMap<String, Binding> {
        let mut variables = self.variables.clone();
        for name in outer_variables.keys() {
            let binding = self.binding_before_shadows(name, shadow_depth);
            variables.insert(name.clone(), binding);
        }
        variables
    }

    fn parse_break(&mut self) -> Result<(), ParseError> {
        if self.loops.is_empty() {
            return Err(self.error_here("`break` outside a loop is not supported"));
        }
        let jump = self.graph.jump(self.control);
        let (names, shadow_depth) = {
            let context = self.loops.last().expect("loop context exists");
            (
                context
                    .phis
                    .iter()
                    .map(|(name, _)| name.clone())
                    .collect::<Vec<_>>(),
                context.shadow_depth,
            )
        };
        let mut variables = self.variables.clone();
        for name in names {
            let binding = self.binding_before_shadows(&name, shadow_depth);
            variables.insert(name, binding);
        }
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
        self.finish_iteration();
        let jump = self.graph.jump(self.control);
        self.append_loop_edge(jump);
        self.terminated = true;
        Ok(())
    }

    fn finish_iteration(&mut self) {
        let Some(end_name) = self
            .loops
            .last()
            .and_then(|context| context.inclusive_end.clone())
        else {
            self.increment_induction();
            return;
        };
        let (index_name, _) = self
            .loops
            .last()
            .and_then(|context| context.induction.clone())
            .expect("inclusive range has an induction variable");
        let current = self.variables[&index_name];
        let end = self.variables[&end_name];
        let condition = self.graph.compare(CompareOp::Eq, current.value, end.value);
        self.types.insert(condition, ScalarType::Bool);
        let branch = self.graph.if_(self.control, condition);

        let last_iteration = self.graph.projection(branch, 0);
        self.control = last_iteration;
        self.parse_break()
            .expect("inclusive range is inside a loop");

        let next_iteration = self.graph.projection(branch, 1);
        self.control = next_iteration;
        self.increment_induction();
    }

    fn increment_induction(&mut self) {
        let Some((name, ty)) = self
            .loops
            .last()
            .and_then(|context| context.induction.clone())
        else {
            return;
        };
        let current = self.variables[&name];
        let one = self.graph.constant(1);
        self.types.insert(one, ty);
        let next = self.graph.binary(BinaryOp::Add, current.value, one);
        self.types.insert(next, ty);
        self.variables.insert(
            name,
            Binding {
                value: next,
                mutable: true,
                ty,
            },
        );
    }

    fn append_loop_edge(&mut self, jump: NodeId) {
        let context = self.loops.last().expect("loop context exists");
        let header = context.header;
        let phis = context.phis.clone();
        let shadow_depth = context.shadow_depth;
        for (name, phi) in phis {
            let value = self.binding_before_shadows(&name, shadow_depth).value;
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
            self.parse_statement()?;
            if self.terminated {
                terminated = true;
                break;
            }
        }
        if terminated && !self.check(&TokenKind::RBrace) {
            return Err(self.error_here(
                "statements after `break`, `continue`, or `return` are not supported",
            ));
        }
        self.expect(TokenKind::RBrace, "`}` after statement body")?;
        Ok(terminated)
    }

    fn parse_if_statement(&mut self) -> Result<(), ParseError> {
        self.position += 1;
        let outer_variables = self.variables.clone();
        let shadow_depth = self.shadowed_variables.len();
        let condition = self.parse_expression()?;
        self.require_type(condition, ScalarType::Bool)?;
        let branch = self.graph.if_(self.control, condition);
        self.expect(TokenKind::LBrace, "`{` after if condition")?;

        let true_projection = self.graph.projection(branch, 0);
        self.control = true_projection;
        let true_terminated = self.parse_statement_body()?;
        let true_variables = self.variables_before_shadows(&outer_variables, shadow_depth);
        let true_jump = if true_terminated {
            None
        } else {
            Some(self.graph.jump(self.control))
        };

        self.variables = outer_variables.clone();
        self.shadowed_variables.truncate(shadow_depth);
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
        let false_variables = self.variables_before_shadows(&outer_variables, shadow_depth);
        let false_jump = if false_terminated {
            None
        } else {
            Some(self.graph.jump(self.control))
        };

        self.variables = outer_variables;
        self.shadowed_variables.truncate(shadow_depth);
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

    fn parse_for(&mut self) -> Result<(), ParseError> {
        self.position += 1;
        let (variable, _) = self.parse_identifier("range loop variable")?;
        if !self.peek_identifier("in") {
            return Err(self.error_here("expected `in` after range loop variable"));
        }
        self.position += 1;
        let start = self.parse_expression()?;
        let inclusive = self.eat(TokenKind::DotDotEq);
        if !inclusive {
            self.expect(TokenKind::DotDot, "`..` in range loop")?;
        }
        let end = self.parse_expression()?;
        let ty = self.numeric_pair(start, end)?;

        let depth = self.loops.len();
        let index_name = format!("\0for_index_{depth}");
        let end_name = format!("\0for_end_{depth}");
        self.variables.insert(
            index_name.clone(),
            Binding {
                value: start,
                mutable: true,
                ty,
            },
        );
        self.variables.insert(
            end_name.clone(),
            Binding {
                value: end,
                mutable: false,
                ty,
            },
        );
        self.parse_loop(
            None,
            Some((index_name, variable, ty, end)),
            inclusive.then_some(end_name),
        )
    }

    fn parse_while(&mut self) -> Result<(), ParseError> {
        self.position += 1;
        self.parse_loop(None, None, None)
    }

    fn parse_loop(
        &mut self,
        condition: Option<NodeId>,
        induction: Option<(String, String, ScalarType, NodeId)>,
        inclusive_end: Option<String>,
    ) -> Result<(), ParseError> {
        let outer_variables = self.variables.clone();
        let shadow_depth = self.shadowed_variables.len();
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
        let condition = match condition {
            Some(condition) => condition,
            None => match &induction {
                Some((index_name, _, ty, end)) => {
                    let current = self.variables[index_name].value;
                    let comparison = match (*ty, inclusive_end.is_some()) {
                        (ScalarType::U64, true) => CompareOp::ULe,
                        (ScalarType::U64, false) => CompareOp::ULt,
                        (_, true) => CompareOp::Le,
                        (_, false) => CompareOp::Lt,
                    };
                    let condition = self.graph.compare(comparison, current, *end);
                    self.mark_bool(condition)
                }
                None => self.parse_expression()?,
            },
        };
        self.require_type(condition, ScalarType::Bool)?;
        let branch = self.graph.if_(header, condition);
        self.expect(TokenKind::LBrace, "`{` after while condition")?;

        let body_projection = self.graph.projection(branch, 0);
        self.control = body_projection;
        if let Some((index_name, variable, ty, _)) = &induction {
            let binding = self.variables[index_name];
            if let Some(outer) = self.variables.get(variable).copied() {
                self.shadowed_variables.push((variable.clone(), outer));
            }
            self.variables.insert(
                variable.clone(),
                Binding {
                    value: binding.value,
                    mutable: false,
                    ty: *ty,
                },
            );
        }
        self.loops.push(LoopContext {
            header,
            phis: loop_variables
                .iter()
                .zip(&phis)
                .map(|((name, _), phi)| (name.clone(), *phi))
                .collect(),
            breaks: Vec::new(),
            induction: induction.map(|(name, _, ty, _)| (name, ty)),
            inclusive_end,
            shadow_depth,
        });
        let body_terminated = self.parse_statement_body()?;
        if !body_terminated {
            self.finish_iteration();
            let back_edge = self.graph.jump(self.control);
            self.append_loop_edge(back_edge);
        }
        let exit_projection = self.graph.projection(branch, 1);
        let context = self.loops.pop().expect("loop context exists");
        self.shadowed_variables.truncate(context.shadow_depth);

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
        let ty = if self.peek_identifier("i64") || self.peek_identifier("isize") {
            ScalarType::I64
        } else if self.peek_identifier("u64") || self.peek_identifier("usize") {
            ScalarType::U64
        } else if self.peek_identifier("bool") {
            ScalarType::Bool
        } else {
            return Err(self.error_here(
                "supported scalar types are `i64`/`isize`, `u64`/`usize`, and `bool`",
            ));
        };
        self.position += 1;
        Ok(ty)
    }

    fn type_of(&self, value: NodeId) -> ScalarType {
        self.types.get(&value).copied().unwrap_or(ScalarType::I64)
    }

    fn require_type(&mut self, value: NodeId, expected: ScalarType) -> Result<(), ParseError> {
        if expected == ScalarType::I64
            && self.unsuffixed_literals.contains(&value)
            && matches!(self.graph.node(value).map(|node| node.kind), Some(NodeKind::Const(bits)) if bits < 0)
        {
            return Err(self.error_here("integer literal is outside the positive i64 range"));
        }
        let actual = self.type_of(value);
        if actual == expected {
            return Ok(());
        }
        if actual == ScalarType::I64
            && expected == ScalarType::U64
            && self.unsuffixed_literals.contains(&value)
        {
            self.types.insert(value, ScalarType::U64);
            return Ok(());
        }
        Err(self.error_here(format!(
            "type mismatch: expected `{expected}`, found `{actual}`"
        )))
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
        } else if self.peek_identifier("match") {
            self.parse_match_expression()
        } else {
            self.parse_logical_or()
        }
    }

    fn parse_match_expression(&mut self) -> Result<NodeId, ParseError> {
        let match_span = self.advance().span;
        let scrutinee = self.parse_expression()?;
        let scrutinee_type = self.type_of(scrutinee);
        if scrutinee_type == ScalarType::Unit {
            return Err(ParseError::new(
                match_span,
                "match scrutinee must have type `i64`/`isize`, `u64`/`usize`, or `bool`",
            ));
        }
        self.expect(TokenKind::LBrace, "`{` after match scrutinee")?;

        let mut seen_literals = HashSet::new();
        let mut seen_ranges = Vec::new();
        let mut controls = Vec::new();
        let mut values = Vec::new();
        let mut has_wildcard = false;
        while !self.check(&TokenKind::RBrace) {
            if self.check(&TokenKind::Eof) {
                return Err(self.error_here("unterminated match expression"));
            }
            if has_wildcard {
                return Err(self.error_here("wildcard `_` arm must be last and unique"));
            }

            let pattern_span = self.peek().span;
            let pattern = self.parse_match_pattern(scrutinee_type)?;
            let bounds = match pattern {
                MatchPattern::Literal { bits, .. } => Some((bits, bits)),
                MatchPattern::Range {
                    start_bits,
                    end_bits,
                    ..
                } => Some((start_bits, end_bits)),
                MatchPattern::Wildcard => None,
            };
            if let Some((start, end)) = bounds {
                if matches!(pattern, MatchPattern::Literal { .. }) && !seen_literals.insert(start) {
                    return Err(ParseError::new(
                        pattern_span,
                        "duplicate literal in match patterns",
                    ));
                }
                let overlaps = seen_ranges.iter().any(|&(other_start, other_end)| {
                    let starts_before_other = if scrutinee_type == ScalarType::U64 {
                        (start as u64) <= other_end as u64
                    } else {
                        start <= other_end
                    };
                    let other_starts_before = if scrutinee_type == ScalarType::U64 {
                        (other_start as u64) <= end as u64
                    } else {
                        other_start <= end
                    };
                    starts_before_other && other_starts_before
                });
                if overlaps {
                    return Err(ParseError::new(
                        pattern_span,
                        "overlapping literal or range in match patterns",
                    ));
                }
                seen_ranges.push((start, end));
            }

            let mut next_branches = Vec::new();
            match pattern {
                MatchPattern::Literal { value, .. } => {
                    let condition = self.graph.compare(CompareOp::Eq, scrutinee, value);
                    self.mark_bool(condition);
                    let branch = self.graph.if_(self.control, condition);
                    self.control = self.graph.projection(branch, 0);
                    next_branches.push(branch);
                }
                MatchPattern::Range { start, end, .. } => {
                    let (lower_operation, upper_operation) = if scrutinee_type == ScalarType::U64 {
                        (CompareOp::UGe, CompareOp::ULe)
                    } else {
                        (CompareOp::Ge, CompareOp::Le)
                    };
                    let lower = self.graph.compare(lower_operation, scrutinee, start);
                    self.mark_bool(lower);
                    let lower_branch = self.graph.if_(self.control, lower);
                    self.control = self.graph.projection(lower_branch, 0);
                    next_branches.push(lower_branch);

                    let upper = self.graph.compare(upper_operation, scrutinee, end);
                    self.mark_bool(upper);
                    let upper_branch = self.graph.if_(self.control, upper);
                    self.control = self.graph.projection(upper_branch, 0);
                    next_branches.push(upper_branch);
                }
                MatchPattern::Wildcard => has_wildcard = true,
            }

            if self.peek_identifier("if") {
                if matches!(pattern, MatchPattern::Wildcard) {
                    return Err(self.error_here(
                        "a guarded wildcard arm is non-exhaustive; use a final unguarded `_` arm",
                    ));
                }
                self.position += 1;
                let guard = self.parse_expression()?;
                self.require_type(guard, ScalarType::Bool)?;
                let branch = self.graph.if_(self.control, guard);
                self.control = self.graph.projection(branch, 0);
                next_branches.push(branch);
            }
            self.expect(
                TokenKind::FatArrow,
                "`=>` after match pattern and optional guard",
            )?;

            let value = if self.eat(TokenKind::LBrace) {
                let value = self.parse_expression()?;
                self.expect(TokenKind::RBrace, "`}` after match arm block")?;
                value
            } else {
                self.parse_expression()?
            };
            values.push(value);
            controls.push(self.graph.jump(self.control));

            if !next_branches.is_empty() {
                let next_jumps = next_branches
                    .into_iter()
                    .map(|branch| {
                        let control = self.graph.projection(branch, 1);
                        self.graph.jump(control)
                    })
                    .collect();
                self.control = self.graph.region(next_jumps);
            }
            if !self.eat(TokenKind::Comma) && !self.check(&TokenKind::RBrace) {
                return Err(self.error_here("expected `,` between match arms"));
            }
        }
        if !has_wildcard {
            return Err(self.error_here("match expression requires a final `_` wildcard arm"));
        }
        self.expect(TokenKind::RBrace, "`}` after match arms")?;

        let mut result_type = self.type_of(values[0]);
        let mut unsuffixed_values = Vec::new();
        if self.unsuffixed_literals.contains(&values[0]) {
            unsuffixed_values.push(values[0]);
        }
        for (index, value) in values.iter().copied().enumerate().skip(1) {
            if result_type == ScalarType::I64
                && self.type_of(value) == ScalarType::U64
                && unsuffixed_values.len() == index
            {
                for previous in unsuffixed_values.drain(..) {
                    self.require_type(previous, ScalarType::U64)?;
                }
                result_type = ScalarType::U64;
            }
            self.require_type(value, result_type)?;
            if self.unsuffixed_literals.contains(&value) {
                unsuffixed_values.push(value);
            }
        }

        self.control = self.graph.region(controls.clone());
        let phi = self.graph.phi(controls, values);
        self.types.insert(phi, result_type);
        Ok(phi)
    }

    fn parse_match_pattern(
        &mut self,
        scrutinee_type: ScalarType,
    ) -> Result<MatchPattern, ParseError> {
        let pattern_span = self.peek().span;
        let Some(start) = self.parse_match_literal(scrutinee_type)? else {
            if matches!(self.peek().kind, TokenKind::DotDot | TokenKind::DotDotEq) {
                return Err(self.error_here("range endpoints must be integer literals"));
            }
            return Ok(MatchPattern::Wildcard);
        };
        if self.eat(TokenKind::DotDot) {
            return Err(self.error_here("only inclusive `..=` range patterns are supported"));
        }
        if !self.eat(TokenKind::DotDotEq) {
            let NodeKind::Const(bits) = self
                .graph
                .node(start)
                .expect("match pattern is a constant")
                .kind
            else {
                unreachable!("match patterns lower to constants")
            };
            return Ok(MatchPattern::Literal { value: start, bits });
        }
        if scrutinee_type == ScalarType::Bool {
            return Err(ParseError::new(
                pattern_span,
                "range patterns require an integer scrutinee",
            ));
        }
        let end = self
            .parse_match_literal(scrutinee_type)?
            .ok_or_else(|| self.error_here("range endpoints must be integer literals"))?;
        let NodeKind::Const(start_bits) = self
            .graph
            .node(start)
            .expect("match range start is a constant")
            .kind
        else {
            unreachable!("match patterns lower to constants")
        };
        let NodeKind::Const(end_bits) = self
            .graph
            .node(end)
            .expect("match range end is a constant")
            .kind
        else {
            unreachable!("match patterns lower to constants")
        };
        let ordered = if scrutinee_type == ScalarType::U64 {
            (start_bits as u64) <= end_bits as u64
        } else {
            start_bits <= end_bits
        };
        if !ordered {
            return Err(ParseError::new(
                pattern_span,
                "range pattern start must be less than or equal to its end",
            ));
        }
        Ok(MatchPattern::Range {
            start,
            end,
            start_bits,
            end_bits,
        })
    }

    fn parse_match_literal(
        &mut self,
        scrutinee_type: ScalarType,
    ) -> Result<Option<NodeId>, ParseError> {
        let negative = self.eat(TokenKind::Minus);
        let token = self.advance();
        let value = match token.kind {
            TokenKind::Integer(magnitude) => {
                if negative {
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
                    self.graph.constant(value)
                } else {
                    let value = self.graph.constant(magnitude as i64);
                    self.unsuffixed_literals.insert(value);
                    value
                }
            }
            TokenKind::I64Integer(magnitude) => {
                if negative {
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
                    self.graph.constant(value)
                } else {
                    if magnitude > i64::MAX as u64 {
                        return Err(ParseError::new(
                            token.span,
                            "integer literal is outside the positive i64 range",
                        ));
                    }
                    self.graph.constant(magnitude as i64)
                }
            }
            TokenKind::U64Integer(magnitude) => {
                if negative {
                    return Err(ParseError::new(
                        token.span,
                        "negative integer literal cannot have an unsigned suffix",
                    ));
                }
                let value = self.graph.constant(magnitude as i64);
                self.types.insert(value, ScalarType::U64);
                value
            }
            TokenKind::Ident(identifier) if !negative && identifier == "_" => return Ok(None),
            TokenKind::Ident(identifier) if !negative && identifier == "true" => {
                let value = self.graph.constant(1);
                self.mark_bool(value)
            }
            TokenKind::Ident(identifier) if !negative && identifier == "false" => {
                let value = self.graph.constant(0);
                self.mark_bool(value)
            }
            _ => {
                return Err(ParseError::new(
                    token.span,
                    "expected an integer, `true`, `false`, or `_` match pattern; tuple and enum patterns are not supported",
                ));
            }
        };
        self.require_type(value, scrutinee_type)?;
        Ok(Some(value))
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

        let mut ty = self.type_of(true_value);
        if self.unsuffixed_literals.contains(&true_value)
            && self.type_of(false_value) == ScalarType::U64
        {
            self.require_type(true_value, ScalarType::U64)?;
            ty = ScalarType::U64;
        }
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
        let ty = if matches!(operation, CompareOp::Eq | CompareOp::Ne)
            && (self.type_of(left) == ScalarType::Bool || self.type_of(right) == ScalarType::Bool)
        {
            self.require_type(left, ScalarType::Bool)?;
            self.require_type(right, ScalarType::Bool)?;
            ScalarType::Bool
        } else {
            self.numeric_pair(left, right)?
        };
        let operation = match (operation, ty) {
            (CompareOp::Lt, ScalarType::U64) => CompareOp::ULt,
            (CompareOp::Le, ScalarType::U64) => CompareOp::ULe,
            (CompareOp::Gt, ScalarType::U64) => CompareOp::UGt,
            (CompareOp::Ge, ScalarType::U64) => CompareOp::UGe,
            (operation, _) => operation,
        };
        let comparison = self.graph.compare(operation, left, right);
        Ok(self.mark_bool(comparison))
    }

    fn numeric_pair(&mut self, left: NodeId, right: NodeId) -> Result<ScalarType, ParseError> {
        if self.unsuffixed_literals.contains(&left) && self.type_of(right) == ScalarType::U64 {
            self.require_type(left, ScalarType::U64)?;
        }
        if self.unsuffixed_literals.contains(&right) && self.type_of(left) == ScalarType::U64 {
            self.require_type(right, ScalarType::U64)?;
        }
        let ty = self.type_of(left);
        if matches!(ty, ScalarType::Bool | ScalarType::Unit) {
            return Err(self.error_here(format!("expected integer operands, found `{ty}`")));
        }
        self.require_type(left, ty)?;
        self.require_type(right, ty)?;
        Ok(ty)
    }

    fn integer_binary(
        &mut self,
        operation: BinaryOp,
        left: NodeId,
        right: NodeId,
    ) -> Result<NodeId, ParseError> {
        let ty = self.numeric_pair(left, right)?;
        let operation = match (operation, ty) {
            (BinaryOp::Div, ScalarType::U64) => BinaryOp::UDiv,
            (BinaryOp::Rem, ScalarType::U64) => BinaryOp::URem,
            (BinaryOp::Shr, ScalarType::U64) => BinaryOp::LShr,
            (operation, _) => operation,
        };
        let value = self.graph.binary(operation, left, right);
        self.types.insert(value, ty);
        Ok(value)
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
        let mut value = self.parse_cast()?;
        loop {
            let operation = match self.peek().kind {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Rem,
                _ => return Ok(value),
            };
            self.position += 1;
            let right = self.parse_cast()?;
            value = self.integer_binary(operation, value, right)?;
        }
    }

    fn parse_cast(&mut self) -> Result<NodeId, ParseError> {
        let value = self.parse_unary()?;
        while self.peek_identifier("as") {
            self.position += 1;
            let target = self.parse_type()?;
            let source = self.type_of(value);
            if source == ScalarType::Unit {
                return Err(self.error_here("unit values cannot be cast to scalar types"));
            }
            if target == ScalarType::Bool && source != ScalarType::Bool {
                return Err(self.error_here("numeric values cannot be cast to `bool`"));
            }
            self.types.insert(value, target);
        }
        Ok(value)
    }

    fn parse_unary(&mut self) -> Result<NodeId, ParseError> {
        if self.eat(TokenKind::Not) {
            let operand = self.parse_unary()?;
            let ty = self.type_of(operand);
            if ty == ScalarType::Unit {
                return Err(self.error_here("operator `!` does not support unit values"));
            }
            self.require_type(operand, ty)?;
            if ty == ScalarType::Bool {
                let zero = self.graph.constant(0);
                let inversion = self.graph.compare(CompareOp::Eq, operand, zero);
                return Ok(self.mark_bool(inversion));
            }
            let inversion = self.graph.unary(UnaryOp::Not, operand);
            self.types.insert(inversion, ty);
            return Ok(inversion);
        }
        if !self.eat(TokenKind::Minus) {
            return self.parse_primary();
        }
        if let TokenKind::Integer(magnitude) | TokenKind::I64Integer(magnitude) = self.peek().kind {
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
                let node = self.graph.constant(value as i64);
                self.unsuffixed_literals.insert(node);
                Ok(node)
            }
            TokenKind::I64Integer(value) => {
                if value > i64::MAX as u64 {
                    return Err(ParseError::new(
                        token.span,
                        "integer literal is outside the positive i64 range",
                    ));
                }
                Ok(self.graph.constant(value as i64))
            }
            TokenKind::U64Integer(value) => {
                let node = self.graph.constant(value as i64);
                self.types.insert(node, ScalarType::U64);
                Ok(node)
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
                    let return_type = signature.return_type.unwrap_or(ScalarType::Unit);
                    let target = self.function_indices[&identifier];
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
                if let Some(binding) = self.variables.get(&identifier) {
                    return Ok(binding.value);
                }
                if let Some(constant) = self.constants.get(&identifier).copied() {
                    let value = self.graph.constant(constant.value);
                    self.types.insert(value, constant.ty);
                    return Ok(value);
                }
                Err(ParseError::new(
                    token.span,
                    format!(
                        "unknown variable or constant `{identifier}`; local bindings must be declared before use"
                    ),
                ))
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
    fn rejects_duplicate_function_and_parameter_names_but_allows_shadowing() {
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
        graph("fn main() { let x = 1; let x = x + 1; return x; }");
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
            error("fn main() { while true { return; let x = 1; } }")
                .message
                .contains("after `break`, `continue`, or `return`")
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
                .contains("supported scalar types")
        );
        assert!(
            error("fn main() -> u32 { return 1; }")
                .message
                .contains("supported scalar types")
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
        graph("fn main() { 1 + 2; }");
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
            error("fn main() { return 0b102; }")
                .message
                .contains("suffix")
        );
        assert!(
            error("fn main() { return 0x_; }")
                .message
                .contains("no digits")
        );
        assert!(
            error("fn main() { return 0x10000000000000000u64; }")
                .message
                .contains("64-bit magnitude")
        );
        assert!(
            error("fn main() { return 1.0; }")
                .message
                .contains("floating-point")
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
        assert!(error("struct main;").message.contains("expected `fn`"));
        assert!(
            error("fn main() { @ }")
                .message
                .contains("unsupported token")
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
