//! A minimal boolean expression language for approval rules.
//!
//! An [`Expression`] is parsed once, when the workflow is built, and evaluated
//! against a JSON context when an approval gate opens. The language is small on
//! purpose: dotted paths, literals, comparisons, `&&`, `||`, `!` and
//! parentheses.
//!
//! # Grammar
//!
//! ```text
//! expr    := or
//! or      := and ("||" and)*
//! and     := unary ("&&" unary)*
//! unary   := "!" unary | primary
//! primary := "(" expr ")" | operand (cmp operand)?
//! cmp     := == | != | > | >= | < | <=
//! operand := path | literal
//! path    := ident ( "." ident | "[" string "]" | "[" integer "]" )*
//! ident   := [A-Za-z_][A-Za-z0-9_-]*
//! literal := number | "str" | 'str' | true | false | null
//! ```
//!
//! The root identifier of every path must be one of [`EXPRESSION_ROOTS`]:
//! `output`, `payload`, `labels`, `metadata` or `steps`.
//!
//! # Evaluation semantics
//!
//! Evaluation is total: it never fails.
//!
//! - A path that does not resolve (missing key, out-of-range index, indexing
//!   into a scalar) is `null`.
//! - `==` and `!=` use JSON equality, with numbers compared as `f64`. When one
//!   side is a number and the other a string that parses as a number, they are
//!   compared numerically, because labels are always strings
//!   (`labels.priority > 3` works with `priority = "5"`). `!=` is the negation
//!   of `==`.
//! - `>`, `>=`, `<` and `<=` compare numbers numerically (with the same
//!   coercion) and strings lexicographically. Any other pair of types is
//!   `false`.
//! - A bare operand is tested for truthiness: `null`, `false`, `0`, `""`, `[]`
//!   and `{}` are false, everything else is true.
//!
//! # Examples
//!
//! ```
//! use ironflow_engine::expression::Expression;
//! use serde_json::json;
//!
//! let expr = Expression::parse("payload.amount > 10000 && labels.env == 'production'")?;
//! let ctx = json!({
//!     "payload": {"amount": 15000},
//!     "labels": {"env": "production"},
//! });
//! assert!(expr.evaluate(&ctx));
//! # Ok::<(), ironflow_engine::expression::ExpressionError>(())
//! ```

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use thiserror::Error;

/// Maximum length of an expression source, in bytes.
///
/// # Examples
///
/// ```
/// use ironflow_engine::expression::{Expression, ExpressionError, MAX_EXPRESSION_LEN};
///
/// let long = format!("payload.a == \"{}\"", "x".repeat(MAX_EXPRESSION_LEN));
/// assert_eq!(Expression::parse(&long), Err(ExpressionError::TooLong));
/// ```
pub const MAX_EXPRESSION_LEN: usize = 4096;

/// Maximum nesting depth of parentheses and `!` operators.
///
/// # Examples
///
/// ```
/// use ironflow_engine::expression::{Expression, ExpressionError, MAX_EXPRESSION_DEPTH};
///
/// let deep = format!(
///     "{}payload.a{}",
///     "(".repeat(MAX_EXPRESSION_DEPTH + 1),
///     ")".repeat(MAX_EXPRESSION_DEPTH + 1)
/// );
/// assert_eq!(Expression::parse(&deep), Err(ExpressionError::TooDeep));
/// ```
pub const MAX_EXPRESSION_DEPTH: usize = 64;

/// Root identifiers a path may start with.
///
/// # Examples
///
/// ```
/// use ironflow_engine::expression::EXPRESSION_ROOTS;
///
/// assert!(EXPRESSION_ROOTS.contains(&"payload"));
/// ```
pub const EXPRESSION_ROOTS: [&str; 5] = ["output", "payload", "labels", "metadata", "steps"];

/// Error returned when an expression fails to parse.
///
/// Positions are byte offsets into the source.
///
/// # Examples
///
/// ```
/// use ironflow_engine::expression::{Expression, ExpressionError};
///
/// let err = Expression::parse("foo.bar == 1").unwrap_err();
/// assert_eq!(err, ExpressionError::UnknownRoot("foo".to_string()));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExpressionError {
    /// The source is empty or only whitespace.
    #[error("expression is empty")]
    Empty,
    /// The source exceeds [`MAX_EXPRESSION_LEN`] bytes.
    #[error("expression is longer than 4096 bytes")]
    TooLong,
    /// Parentheses or `!` are nested deeper than [`MAX_EXPRESSION_DEPTH`].
    #[error("expression is nested deeper than 64 levels")]
    TooDeep,
    /// A character that starts no token.
    #[error("unexpected character {ch:?} at position {pos}")]
    UnexpectedChar {
        /// Byte offset of the character.
        pos: usize,
        /// The offending character.
        ch: char,
    },
    /// A string literal without its closing quote.
    #[error("unterminated string starting at position {pos}")]
    UnterminatedString {
        /// Byte offset of the opening quote.
        pos: usize,
    },
    /// A malformed number literal.
    #[error("invalid number at position {pos}")]
    InvalidNumber {
        /// Byte offset of the number.
        pos: usize,
    },
    /// A token that does not fit the grammar at this point.
    #[error("unexpected token {found:?} at position {pos}")]
    UnexpectedToken {
        /// Byte offset of the token.
        pos: usize,
        /// Text of the token.
        found: String,
    },
    /// The source ended where more input was expected.
    #[error("unexpected end of expression")]
    UnexpectedEnd,
    /// A path starts with an identifier outside [`EXPRESSION_ROOTS`].
    #[error("unknown root {0:?}, expected one of output, payload, labels, metadata, steps")]
    UnknownRoot(String),
}

/// A parsed boolean expression over a JSON context.
///
/// See the [module documentation](crate::expression) for the grammar and the
/// evaluation semantics. Serializes as its source string; deserializing parses
/// the string and rejects invalid expressions. Two expressions are equal when
/// their sources are equal.
///
/// # Examples
///
/// ```
/// use ironflow_engine::expression::Expression;
/// use serde_json::json;
///
/// let expr: Expression = "steps[\"risk-assessment\"].output.level == \"high\"".parse()?;
/// let ctx = json!({"steps": {"risk-assessment": {"output": {"level": "high"}}}});
/// assert!(expr.evaluate(&ctx));
/// # Ok::<(), ironflow_engine::expression::ExpressionError>(())
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Expression {
    source: String,
    ast: Node,
}

impl Expression {
    /// Parse an expression.
    ///
    /// # Errors
    ///
    /// Returns an [`ExpressionError`] when the source is empty, too long, too
    /// deeply nested, lexically or syntactically invalid, or uses a path root
    /// outside [`EXPRESSION_ROOTS`].
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::expression::Expression;
    ///
    /// let expr = Expression::parse("output.amount > 10000")?;
    /// assert_eq!(expr.source(), "output.amount > 10000");
    /// assert!(Expression::parse("output.amount >").is_err());
    /// # Ok::<(), ironflow_engine::expression::ExpressionError>(())
    /// ```
    pub fn parse(src: &str) -> Result<Self, ExpressionError> {
        if src.trim().is_empty() {
            return Err(ExpressionError::Empty);
        }
        if src.len() > MAX_EXPRESSION_LEN {
            return Err(ExpressionError::TooLong);
        }
        let tokens = tokenize(src)?;
        let mut parser = Parser {
            tokens,
            pos: 0,
            depth: 0,
        };
        let ast = parser.parse_or()?;
        if let Some(token) = parser.peek() {
            return Err(token.unexpected());
        }
        Ok(Self {
            source: src.to_string(),
            ast,
        })
    }

    /// Evaluate the expression against a JSON context.
    ///
    /// Never fails: missing paths resolve to `null` and type mismatches
    /// evaluate to `false`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::expression::Expression;
    /// use serde_json::json;
    ///
    /// let expr = Expression::parse("labels.priority > 3")?;
    /// assert!(expr.evaluate(&json!({"labels": {"priority": "5"}})));
    /// assert!(!expr.evaluate(&json!({"labels": {}})));
    /// # Ok::<(), ironflow_engine::expression::ExpressionError>(())
    /// ```
    pub fn evaluate(&self, ctx: &Value) -> bool {
        self.ast.eval(ctx)
    }

    /// The source text the expression was parsed from.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::expression::Expression;
    ///
    /// let expr = Expression::parse("payload.urgent")?;
    /// assert_eq!(expr.source(), "payload.urgent");
    /// # Ok::<(), ironflow_engine::expression::ExpressionError>(())
    /// ```
    pub fn source(&self) -> &str {
        &self.source
    }
}

impl PartialEq for Expression {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl Eq for Expression {}

impl fmt::Display for Expression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.source)
    }
}

impl FromStr for Expression {
    type Err = ExpressionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<String> for Expression {
    type Error = ExpressionError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<Expression> for String {
    fn from(expr: Expression) -> Self {
        expr.source
    }
}

// ---------------------------------------------------------------------------
// AST and evaluation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Node {
    Or(Vec<Node>),
    And(Vec<Node>),
    Not(Box<Node>),
    Compare(Operand, CmpOp, Operand),
    Truthy(Operand),
}

#[derive(Debug, Clone, Copy)]
enum CmpOp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Debug, Clone)]
enum Operand {
    Path(Vec<Segment>),
    Literal(Value),
}

#[derive(Debug, Clone)]
enum Segment {
    Key(String),
    Index(usize),
}

static NULL: Value = Value::Null;

impl Node {
    fn eval(&self, ctx: &Value) -> bool {
        match self {
            Node::Or(nodes) => nodes.iter().any(|n| n.eval(ctx)),
            Node::And(nodes) => nodes.iter().all(|n| n.eval(ctx)),
            Node::Not(node) => !node.eval(ctx),
            Node::Compare(left, op, right) => {
                let (a, b) = (left.resolve(ctx), right.resolve(ctx));
                match op {
                    CmpOp::Eq => loose_eq(a, b),
                    CmpOp::Ne => !loose_eq(a, b),
                    CmpOp::Gt => loose_cmp(a, b).is_some_and(Ordering::is_gt),
                    CmpOp::Ge => loose_cmp(a, b).is_some_and(Ordering::is_ge),
                    CmpOp::Lt => loose_cmp(a, b).is_some_and(Ordering::is_lt),
                    CmpOp::Le => loose_cmp(a, b).is_some_and(Ordering::is_le),
                }
            }
            Node::Truthy(operand) => truthy(operand.resolve(ctx)),
        }
    }
}

impl Operand {
    fn resolve<'a>(&'a self, ctx: &'a Value) -> &'a Value {
        match self {
            Operand::Literal(value) => value,
            Operand::Path(segments) => {
                let mut current = ctx;
                for segment in segments {
                    let next = match segment {
                        Segment::Key(key) => current.as_object().and_then(|o| o.get(key)),
                        Segment::Index(idx) => current.as_array().and_then(|a| a.get(*idx)),
                    };
                    match next {
                        Some(value) => current = value,
                        None => return &NULL,
                    }
                }
                current
            }
        }
    }
}

/// The numeric value of `value`, coercing numeric strings.
fn as_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Both sides as numbers, when at least one is a JSON number and the other is
/// a number or a numeric string.
fn numeric_pair(a: &Value, b: &Value) -> Option<(f64, f64)> {
    if !a.is_number() && !b.is_number() {
        return None;
    }
    Some((as_number(a)?, as_number(b)?))
}

fn loose_eq(a: &Value, b: &Value) -> bool {
    if a.is_number() || b.is_number() {
        return numeric_pair(a, b).is_some_and(|(x, y)| x == y);
    }
    a == b
}

fn loose_cmp(a: &Value, b: &Value) -> Option<Ordering> {
    if let Some((x, y)) = numeric_pair(a, b) {
        return x.partial_cmp(&y);
    }
    match (a, b) {
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Num(Number),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dot,
    AndAnd,
    OrOr,
    Bang,
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tok::Ident(s) => f.write_str(s),
            Tok::Str(s) => write!(f, "{s:?}"),
            Tok::Num(n) => write!(f, "{n}"),
            Tok::LParen => f.write_str("("),
            Tok::RParen => f.write_str(")"),
            Tok::LBracket => f.write_str("["),
            Tok::RBracket => f.write_str("]"),
            Tok::Dot => f.write_str("."),
            Tok::AndAnd => f.write_str("&&"),
            Tok::OrOr => f.write_str("||"),
            Tok::Bang => f.write_str("!"),
            Tok::Eq => f.write_str("=="),
            Tok::Ne => f.write_str("!="),
            Tok::Gt => f.write_str(">"),
            Tok::Ge => f.write_str(">="),
            Tok::Lt => f.write_str("<"),
            Tok::Le => f.write_str("<="),
        }
    }
}

#[derive(Debug, Clone)]
struct Token {
    kind: Tok,
    pos: usize,
}

impl Token {
    fn unexpected(&self) -> ExpressionError {
        ExpressionError::UnexpectedToken {
            pos: self.pos,
            found: self.kind.to_string(),
        }
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn tokenize(src: &str) -> Result<Vec<Token>, ExpressionError> {
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let mut tokens = Vec::new();
    let mut i = 0;

    while let Some(&(pos, c)) = chars.get(i) {
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        let next = chars.get(i + 1).map(|&(_, n)| n);
        let (kind, len) = match (c, next) {
            ('(', _) => (Tok::LParen, 1),
            (')', _) => (Tok::RParen, 1),
            ('[', _) => (Tok::LBracket, 1),
            (']', _) => (Tok::RBracket, 1),
            ('.', _) => (Tok::Dot, 1),
            ('&', Some('&')) => (Tok::AndAnd, 2),
            ('|', Some('|')) => (Tok::OrOr, 2),
            ('=', Some('=')) => (Tok::Eq, 2),
            ('!', Some('=')) => (Tok::Ne, 2),
            ('!', _) => (Tok::Bang, 1),
            ('>', Some('=')) => (Tok::Ge, 2),
            ('>', _) => (Tok::Gt, 1),
            ('<', Some('=')) => (Tok::Le, 2),
            ('<', _) => (Tok::Lt, 1),
            ('"' | '\'', _) => {
                let (value, consumed) = lex_string(&chars, i)?;
                (Tok::Str(value), consumed)
            }
            (c, _) if c.is_ascii_digit() || c == '-' => {
                let (number, consumed) = lex_number(src, &chars, i)?;
                (Tok::Num(number), consumed)
            }
            (c, _) if is_ident_start(c) => {
                let mut j = i + 1;
                while chars.get(j).is_some_and(|&(_, n)| is_ident_continue(n)) {
                    j += 1;
                }
                let end = chars.get(j).map_or(src.len(), |&(p, _)| p);
                (Tok::Ident(src[pos..end].to_string()), j - i)
            }
            (ch, _) => return Err(ExpressionError::UnexpectedChar { pos, ch }),
        };

        tokens.push(Token { kind, pos });
        i += len;
    }

    Ok(tokens)
}

/// Lex a quoted string starting at `chars[start]`. Returns the unescaped value
/// and the number of chars consumed, quotes included.
fn lex_string(chars: &[(usize, char)], start: usize) -> Result<(String, usize), ExpressionError> {
    let (open_pos, quote) = chars[start];
    let mut value = String::new();
    let mut j = start + 1;

    while let Some(&(pos, c)) = chars.get(j) {
        if c == quote {
            return Ok((value, j - start + 1));
        }
        if c != '\\' {
            value.push(c);
            j += 1;
            continue;
        }
        let Some(&(_, escaped)) = chars.get(j + 1) else {
            break;
        };
        match escaped {
            '"' | '\'' | '\\' => value.push(escaped),
            'n' => value.push('\n'),
            other => return Err(ExpressionError::UnexpectedChar { pos, ch: other }),
        }
        j += 2;
    }

    Err(ExpressionError::UnterminatedString { pos: open_pos })
}

/// Lex a number literal starting at `chars[start]`, with an optional leading
/// `-` and an optional fractional part. Returns the number and the number of
/// chars consumed.
fn lex_number(
    src: &str,
    chars: &[(usize, char)],
    start: usize,
) -> Result<(Number, usize), ExpressionError> {
    let pos = chars[start].0;
    let invalid = ExpressionError::InvalidNumber { pos };
    let is_digit = |j: usize| chars.get(j).is_some_and(|&(_, c)| c.is_ascii_digit());

    let mut j = start;
    if chars[j].1 == '-' {
        j += 1;
    }
    if !is_digit(j) {
        return Err(invalid);
    }
    while is_digit(j) {
        j += 1;
    }
    let mut is_float = false;
    if chars.get(j).is_some_and(|&(_, c)| c == '.') && is_digit(j + 1) {
        is_float = true;
        j += 1;
        while is_digit(j) {
            j += 1;
        }
    }
    // A number glued to an identifier character (`12ab`) is malformed.
    if chars.get(j).is_some_and(|&(_, c)| is_ident_start(c)) {
        return Err(invalid);
    }

    let end = chars.get(j).map_or(src.len(), |&(p, _)| p);
    let text = &src[pos..end];
    let parsed = if is_float {
        text.parse::<f64>().ok().and_then(Number::from_f64)
    } else {
        text.parse::<i64>().ok().map(Number::from)
    };
    parsed.map(|n| (n, j - start)).ok_or(invalid)
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_kind(&self) -> Option<&Tok> {
        self.peek().map(|t| &t.kind)
    }

    fn advance(&mut self) -> Result<Token, ExpressionError> {
        let token = self
            .tokens
            .get(self.pos)
            .cloned()
            .ok_or(ExpressionError::UnexpectedEnd)?;
        self.pos += 1;
        Ok(token)
    }

    fn expect(&mut self, kind: &Tok) -> Result<(), ExpressionError> {
        let token = self.advance()?;
        if &token.kind == kind {
            Ok(())
        } else {
            Err(token.unexpected())
        }
    }

    fn enter(&mut self) -> Result<(), ExpressionError> {
        self.depth += 1;
        if self.depth > MAX_EXPRESSION_DEPTH {
            return Err(ExpressionError::TooDeep);
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn parse_or(&mut self) -> Result<Node, ExpressionError> {
        let mut nodes = vec![self.parse_and()?];
        while self.peek_kind() == Some(&Tok::OrOr) {
            self.pos += 1;
            nodes.push(self.parse_and()?);
        }
        Ok(if nodes.len() == 1 {
            nodes.remove(0)
        } else {
            Node::Or(nodes)
        })
    }

    fn parse_and(&mut self) -> Result<Node, ExpressionError> {
        let mut nodes = vec![self.parse_unary()?];
        while self.peek_kind() == Some(&Tok::AndAnd) {
            self.pos += 1;
            nodes.push(self.parse_unary()?);
        }
        Ok(if nodes.len() == 1 {
            nodes.remove(0)
        } else {
            Node::And(nodes)
        })
    }

    fn parse_unary(&mut self) -> Result<Node, ExpressionError> {
        if self.peek_kind() == Some(&Tok::Bang) {
            self.pos += 1;
            self.enter()?;
            let inner = self.parse_unary()?;
            self.leave();
            return Ok(Node::Not(Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Node, ExpressionError> {
        if self.peek_kind() == Some(&Tok::LParen) {
            self.pos += 1;
            self.enter()?;
            let inner = self.parse_or()?;
            self.expect(&Tok::RParen)?;
            self.leave();
            return Ok(inner);
        }

        let left = self.parse_operand()?;
        let op = match self.peek_kind() {
            Some(Tok::Eq) => CmpOp::Eq,
            Some(Tok::Ne) => CmpOp::Ne,
            Some(Tok::Gt) => CmpOp::Gt,
            Some(Tok::Ge) => CmpOp::Ge,
            Some(Tok::Lt) => CmpOp::Lt,
            Some(Tok::Le) => CmpOp::Le,
            _ => return Ok(Node::Truthy(left)),
        };
        self.pos += 1;
        let right = self.parse_operand()?;
        Ok(Node::Compare(left, op, right))
    }

    fn parse_operand(&mut self) -> Result<Operand, ExpressionError> {
        let token = self.advance()?;
        match &token.kind {
            Tok::Str(s) => Ok(Operand::Literal(Value::String(s.clone()))),
            Tok::Num(n) => Ok(Operand::Literal(Value::Number(n.clone()))),
            Tok::Ident(name) => match name.as_str() {
                "true" => Ok(Operand::Literal(Value::Bool(true))),
                "false" => Ok(Operand::Literal(Value::Bool(false))),
                "null" => Ok(Operand::Literal(Value::Null)),
                root if EXPRESSION_ROOTS.contains(&root) => {
                    let mut segments = vec![Segment::Key(root.to_string())];
                    self.parse_path_tail(&mut segments)?;
                    Ok(Operand::Path(segments))
                }
                other => Err(ExpressionError::UnknownRoot(other.to_string())),
            },
            _ => Err(token.unexpected()),
        }
    }

    fn parse_path_tail(&mut self, segments: &mut Vec<Segment>) -> Result<(), ExpressionError> {
        loop {
            match self.peek_kind() {
                Some(Tok::Dot) => {
                    self.pos += 1;
                    let token = self.advance()?;
                    match &token.kind {
                        Tok::Ident(name) => segments.push(Segment::Key(name.clone())),
                        _ => return Err(token.unexpected()),
                    }
                }
                Some(Tok::LBracket) => {
                    self.pos += 1;
                    let token = self.advance()?;
                    match &token.kind {
                        Tok::Str(key) => segments.push(Segment::Key(key.clone())),
                        Tok::Num(n) => match n.as_u64() {
                            Some(idx) => segments.push(Segment::Index(idx as usize)),
                            None => return Err(token.unexpected()),
                        },
                        _ => return Err(token.unexpected()),
                    }
                    self.expect(&Tok::RBracket)?;
                }
                _ => return Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value, json, to_value};

    use super::*;

    fn eval(src: &str, ctx: &Value) -> bool {
        Expression::parse(src)
            .unwrap_or_else(|e| panic!("{src:?} should parse: {e}"))
            .evaluate(ctx)
    }

    fn ctx() -> Value {
        json!({
            "output": {"amount": 15000, "ok": true},
            "payload": {
                "amount": 15000,
                "ratio": 0.75,
                "delta": -3,
                "name": "café",
                "items": ["first", "second"],
                "empty": "",
                "zero": 0,
                "list": [],
                "obj": {},
                "flag": false,
            },
            "labels": {"env": "production", "priority": "5", "team": "ops"},
            "metadata": {"attempt": 1, "workflow_name": "deploy"},
            "steps": {
                "risk_assessment": {"output": {"level": "high"}},
                "risk-assessment": {"output": {"level": "high"}},
            },
        })
    }

    // ---- comparisons ----

    #[test]
    fn every_comparison_operator() {
        let c = ctx();
        assert!(eval("payload.amount == 15000", &c));
        assert!(eval("payload.amount != 1", &c));
        assert!(eval("payload.amount > 10000", &c));
        assert!(eval("payload.amount >= 15000", &c));
        assert!(eval("payload.amount < 20000", &c));
        assert!(eval("payload.amount <= 15000", &c));
        assert!(!eval("payload.amount < 15000", &c));
        assert!(!eval("payload.amount >= 15001", &c));
        assert!(!eval("payload.amount <= 14999", &c));
    }

    #[test]
    fn output_amount_threshold() {
        assert!(eval("output.amount > 10000", &ctx()));
        let small = json!({"output": {"amount": 10}});
        assert!(!eval("output.amount > 10000", &small));
    }

    #[test]
    fn label_equality() {
        assert!(eval("labels.env == \"production\"", &ctx()));
        assert!(!eval("labels.env == \"staging\"", &ctx()));
    }

    #[test]
    fn step_output_path() {
        let c = ctx();
        assert!(eval("steps.risk_assessment.output.level == \"high\"", &c));
    }

    #[test]
    fn bracket_path_with_dash() {
        let src = "steps[\"risk-assessment\"].output.level == 'high'";
        assert!(eval(src, &ctx()));
    }

    #[test]
    fn bracket_path_with_unicode_and_spaces() {
        let c = json!({"steps": {"évaluation des risques": {"output": {"level": "élevé"}}}});
        let src = "steps[\"évaluation des risques\"].output.level == \"élevé\"";
        assert!(eval(src, &c));
    }

    #[test]
    fn array_index() {
        assert!(eval("payload.items[0] == \"first\"", &ctx()));
        assert!(eval("payload.items[1] == \"second\"", &ctx()));
        assert!(!eval("payload.items[5]", &ctx()));
    }

    // ---- boolean operators ----

    #[test]
    fn and_or_not() {
        let c = ctx();
        assert!(eval("payload.amount > 1 && labels.env == 'production'", &c));
        assert!(!eval("payload.amount > 1 && labels.env == 'staging'", &c));
        assert!(eval("payload.amount < 1 || labels.env == 'production'", &c));
        assert!(!eval("payload.amount < 1 || labels.env == 'staging'", &c));
        assert!(eval("!(labels.env == 'staging')", &c));
        assert!(eval("!!payload.amount", &c));
    }

    #[test]
    fn and_binds_tighter_than_or() {
        // true || (false && false) == true; (true || false) && false == false
        let c = ctx();
        assert!(eval(
            "labels.env == 'production' || labels.env == 'x' && labels.env == 'y'",
            &c
        ));
        assert!(!eval(
            "(labels.env == 'production' || labels.env == 'x') && labels.env == 'y'",
            &c
        ));
    }

    #[test]
    fn not_applies_to_the_following_primary_only() {
        let c = ctx();
        // (!false) && true
        assert!(eval("!payload.flag && payload.amount", &c));
    }

    // ---- truthiness ----

    #[test]
    fn bare_path_truthiness() {
        let c = ctx();
        assert!(eval("output.ok", &c));
        assert!(eval("payload.amount", &c));
        assert!(eval("payload.items", &c));
        assert!(eval("labels", &c));
        assert!(!eval("payload.flag", &c));
        assert!(!eval("payload.zero", &c));
        assert!(!eval("payload.empty", &c));
        assert!(!eval("payload.list", &c));
        assert!(!eval("payload.obj", &c));
        assert!(!eval("payload.missing", &c));
    }

    #[test]
    fn literal_truthiness() {
        let c = ctx();
        assert!(eval("true", &c));
        assert!(!eval("false", &c));
        assert!(!eval("null", &c));
        assert!(eval("1", &c));
        assert!(!eval("''", &c));
    }

    // ---- missing data and coercion ----

    #[test]
    fn missing_path_is_null() {
        let c = ctx();
        assert!(!eval("payload.nope.deeper > 1", &c));
        assert!(!eval("payload.nope == 1", &c));
        assert!(eval("payload.nope == null", &c));
        assert!(!eval("payload.amount.inner", &c));
        assert!(!eval("output.amount > 1", &json!({"output": null})));
        assert!(!eval("output.amount > 1", &json!({})));
    }

    #[test]
    fn not_equal_on_missing_path_is_true() {
        assert!(eval("payload.nope != 'x'", &ctx()));
    }

    #[test]
    fn label_strings_coerce_to_numbers() {
        let c = ctx();
        assert!(eval("labels.priority > 3", &c));
        assert!(eval("labels.priority == 5", &c));
        assert!(eval("5.0 == labels.priority", &c));
        assert!(!eval("labels.priority < 3", &c));
    }

    #[test]
    fn type_mismatch_is_false() {
        let c = ctx();
        assert!(!eval("labels.env > 3", &c));
        assert!(!eval("labels.env == 3", &c));
        assert!(!eval("payload.items > 1", &c));
        assert!(!eval("payload.flag < 1", &c));
        assert!(eval("labels.env != 3", &c));
    }

    #[test]
    fn strings_compare_lexicographically() {
        let c = ctx();
        assert!(eval("labels.team < 'zzz'", &c));
        assert!(eval("labels.team >= 'ops'", &c));
        assert!(!eval("labels.team > 'ops'", &c));
    }

    // ---- literals ----

    #[test]
    fn floats_and_negatives() {
        let c = ctx();
        assert!(eval("payload.ratio == 0.75", &c));
        assert!(eval("payload.ratio > 0.5", &c));
        assert!(eval("payload.delta == -3", &c));
        assert!(eval("payload.delta < -2.5", &c));
        assert!(eval("payload.amount == 15000.0", &c));
    }

    #[test]
    fn single_quotes_escapes_and_unicode() {
        let c = json!({"payload": {"q": "it's \"quoted\"\\\n", "name": "café ☕"}});
        assert!(eval(r#"payload.q == 'it\'s "quoted"\\\n'"#, &c));
        assert!(eval(r#"payload.q == "it's \"quoted\"\\\n""#, &c));
        assert!(eval("payload.name == 'café ☕'", &c));
    }

    #[test]
    fn whitespace_is_insignificant() {
        let src = "  payload.amount>10000&&labels.env=='production'  ";
        assert!(eval(src, &ctx()));
    }

    // ---- errors ----

    #[test]
    fn empty_source_is_rejected() {
        assert_eq!(Expression::parse(""), Err(ExpressionError::Empty));
        assert_eq!(Expression::parse("   \n"), Err(ExpressionError::Empty));
    }

    #[test]
    fn unknown_root_is_rejected() {
        assert_eq!(
            Expression::parse("foo.bar == 1"),
            Err(ExpressionError::UnknownRoot("foo".to_string()))
        );
        assert_eq!(
            Expression::parse("payload.a == other"),
            Err(ExpressionError::UnknownRoot("other".to_string()))
        );
    }

    #[test]
    fn unterminated_string_is_rejected() {
        assert_eq!(
            Expression::parse("labels.env == \"prod"),
            Err(ExpressionError::UnterminatedString { pos: 14 })
        );
        assert_eq!(
            Expression::parse("labels.env == 'prod\\"),
            Err(ExpressionError::UnterminatedString { pos: 14 })
        );
    }

    #[test]
    fn trailing_token_is_rejected() {
        assert_eq!(
            Expression::parse("payload.a == 1 2"),
            Err(ExpressionError::UnexpectedToken {
                pos: 15,
                found: "2".to_string()
            })
        );
    }

    #[test]
    fn unexpected_char_is_rejected() {
        assert_eq!(
            Expression::parse("payload.a # 1"),
            Err(ExpressionError::UnexpectedChar { pos: 10, ch: '#' })
        );
        assert_eq!(
            Expression::parse("payload.a = 1"),
            Err(ExpressionError::UnexpectedChar { pos: 10, ch: '=' })
        );
        assert_eq!(
            Expression::parse("payload.a & payload.b"),
            Err(ExpressionError::UnexpectedChar { pos: 10, ch: '&' })
        );
    }

    #[test]
    fn invalid_number_is_rejected() {
        assert_eq!(
            Expression::parse("payload.a > -"),
            Err(ExpressionError::InvalidNumber { pos: 12 })
        );
        assert_eq!(
            Expression::parse("payload.a > 12ab"),
            Err(ExpressionError::InvalidNumber { pos: 12 })
        );
    }

    #[test]
    fn unexpected_end_is_rejected() {
        assert_eq!(
            Expression::parse("payload.a >"),
            Err(ExpressionError::UnexpectedEnd)
        );
        assert_eq!(
            Expression::parse("(payload.a"),
            Err(ExpressionError::UnexpectedEnd)
        );
        assert_eq!(
            Expression::parse("payload."),
            Err(ExpressionError::UnexpectedEnd)
        );
    }

    #[test]
    fn misplaced_tokens_are_rejected() {
        assert!(matches!(
            Expression::parse("payload.a == == 1"),
            Err(ExpressionError::UnexpectedToken { .. })
        ));
        assert!(matches!(
            Expression::parse("payload[-1]"),
            Err(ExpressionError::UnexpectedToken { .. })
        ));
        assert!(matches!(
            Expression::parse("payload[1.5]"),
            Err(ExpressionError::UnexpectedToken { .. })
        ));
        assert!(matches!(
            Expression::parse(")"),
            Err(ExpressionError::UnexpectedToken { .. })
        ));
        assert!(matches!(
            Expression::parse("payload.a > 1 > 2"),
            Err(ExpressionError::UnexpectedToken { .. })
        ));
    }

    #[test]
    fn nesting_up_to_the_limit_is_accepted() {
        let src = format!(
            "{}payload.a{}",
            "(".repeat(MAX_EXPRESSION_DEPTH),
            ")".repeat(MAX_EXPRESSION_DEPTH)
        );
        assert!(Expression::parse(&src).is_ok());
        let bangs = format!("{}payload.a", "!".repeat(MAX_EXPRESSION_DEPTH));
        assert!(Expression::parse(&bangs).is_ok());
    }

    #[test]
    fn nesting_beyond_the_limit_is_rejected() {
        let src = format!(
            "{}payload.a{}",
            "(".repeat(MAX_EXPRESSION_DEPTH + 1),
            ")".repeat(MAX_EXPRESSION_DEPTH + 1)
        );
        assert_eq!(Expression::parse(&src), Err(ExpressionError::TooDeep));
        let bangs = format!("{}payload.a", "!".repeat(MAX_EXPRESSION_DEPTH + 1));
        assert_eq!(Expression::parse(&bangs), Err(ExpressionError::TooDeep));
    }

    #[test]
    fn source_longer_than_the_limit_is_rejected() {
        let src = format!("payload.a == '{}'", "x".repeat(MAX_EXPRESSION_LEN));
        assert_eq!(Expression::parse(&src), Err(ExpressionError::TooLong));
    }

    // ---- traits and serde ----

    #[test]
    fn display_from_str_and_conversions() {
        let expr: Expression = "payload.a > 1".parse().expect("parse");
        assert_eq!(expr.to_string(), "payload.a > 1");
        assert_eq!(expr.source(), "payload.a > 1");

        let from_string = Expression::try_from(expr.to_string()).expect("parse");
        assert_eq!(from_string, expr);
        assert_ne!(
            from_string,
            Expression::parse("payload.a >= 1").expect("parse")
        );

        let back: String = expr.into();
        assert_eq!(back, "payload.a > 1");
    }

    #[test]
    fn serde_roundtrip_as_a_plain_string() {
        let expr = Expression::parse("labels.env == 'production'").expect("parse");
        let json = to_value(&expr).expect("serialize");
        assert_eq!(json, json!("labels.env == 'production'"));

        let back: Expression = from_value(json).expect("deserialize");
        assert_eq!(back, expr);
        assert!(back.evaluate(&ctx()));
    }

    #[test]
    fn serde_rejects_an_invalid_expression() {
        let err = from_value::<Expression>(json!("foo.bar == 1")).expect_err("invalid expression");
        assert!(err.to_string().contains("unknown root"));
        assert!(from_value::<Expression>(json!("")).is_err());
    }
}
