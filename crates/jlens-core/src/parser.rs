//! The Pratt (top-down operator precedence) parser. Ported from the `parser` function in
//! jsonata-js `src/parser.js` (MIT), which builds on Vaughan Pratt's algorithm
//! and Douglas Crockford's TDOP framework.
//!
//! The parser produces a *raw* tree that mirrors the reference implementation's parse tree;
//! [`crate::process`] then turns it into the [`crate::ast`] the evaluator uses.
//!
//! In *recover* mode, errors are collected instead of aborting and replaced with error nodes,
//! following the reference implementation. jlens additionally recovers from lexer errors (an
//! unterminated string, for instance), which editors hit constantly while the user is typing.

use serde::Serialize;

use crate::annotation::{self, Annotation, DocComment};
use crate::ast::Span;
use crate::error::{Diagnostic, Error, Result};
use crate::lexer::{Lexer, Lit, TokKind};
use crate::signature::Signature;

/// An annotation covers the expression that follows it, but never extends past `and`/`or`
/// (whose binding power is 30) or looser operators.
const ANNOTATION_BP: u8 = 30;

/// A node of the raw parse tree.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Raw {
    #[serde(flatten)]
    pub kind: RawKind,
    pub span: Span,
    /// Byte offset just after the node's main token.
    #[serde(skip)]
    pub pos: usize,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub keep_array: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation: Option<Box<Annotation>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum RawKind {
    Name {
        value: String,
    },
    Variable {
        value: String,
    },
    String {
        value: String,
    },
    Number {
        value: f64,
    },
    Value {
        value: Lit,
    },
    Regex {
        pattern: String,
        flags: String,
    },
    Wildcard,
    Descendant,
    Parent,
    Binary {
        op: &'static str,
        lhs: Box<Raw>,
        rhs: Box<Raw>,
    },
    Sort {
        lhs: Box<Raw>,
        terms: Vec<(bool, Raw)>,
    },
    Group {
        lhs: Box<Raw>,
        pairs: Vec<(Raw, Raw)>,
    },
    Array {
        items: Vec<Raw>,
    },
    Object {
        pairs: Vec<(Raw, Raw)>,
    },
    Negate {
        expr: Box<Raw>,
    },
    Function {
        procedure: Box<Raw>,
        args: Vec<Raw>,
        partial: bool,
    },
    Placeholder,
    Lambda {
        params: Vec<Raw>,
        signature: Option<Signature>,
        body: Box<Raw>,
    },
    Condition {
        condition: Box<Raw>,
        then: Box<Raw>,
        otherwise: Option<Box<Raw>>,
    },
    Transform {
        pattern: Box<Raw>,
        update: Box<Raw>,
        delete: Option<Box<Raw>>,
    },
    Block {
        exprs: Vec<Raw>,
    },
    /// `(end)` used as an operand; processing reports S0207.
    End,
    /// An error recorded by the recovering parser, possibly wrapping what was parsed before it.
    Error {
        error: Option<Box<Error>>,
        lhs: Option<Box<Raw>>,
    },
}

impl Raw {
    fn new(kind: RawKind, span: Span, pos: usize) -> Self {
        Raw {
            kind,
            span,
            pos,
            keep_array: false,
            annotation: None,
        }
    }

    /// The value jsonata-js would report as the `token` of an error about this node.
    fn token_value(&self) -> serde_json::Value {
        match &self.kind {
            RawKind::Name { value } | RawKind::Variable { value } | RawKind::String { value } => {
                value.clone().into()
            }
            RawKind::Number { value } => (*value).into(),
            RawKind::Value { value } => lit_json(*value),
            _ => serde_json::Value::Null,
        }
    }
}

fn lit_json(lit: Lit) -> serde_json::Value {
    match lit {
        Lit::Bool(b) => b.into(),
        Lit::Null => serde_json::Value::Null,
    }
}

/// The current symbol: a token, the end of input, or the error marker of recover mode.
#[derive(Debug, Clone)]
enum SymKind {
    Tok(TokKind),
    End,
    Error,
}

#[derive(Debug, Clone)]
struct Sym {
    kind: SymKind,
    span: Span,
    docs: Vec<DocComment>,
}

impl Sym {
    fn id(&self) -> &'static str {
        match &self.kind {
            SymKind::Tok(TokKind::Name(_) | TokKind::Variable(_)) => "(name)",
            SymKind::Tok(TokKind::Str(_) | TokKind::Number(_) | TokKind::Value(_)) => "(literal)",
            SymKind::Tok(TokKind::Regex { .. }) => "(regex)",
            SymKind::Tok(TokKind::Op(op)) => op,
            SymKind::End => "(end)",
            SymKind::Error => "(error)",
        }
    }

    /// Left binding power, from the reference implementation's symbol table.
    fn lbp(&self) -> u8 {
        match self.id() {
            "." => 75,
            "[" | "(" | "@" | "#" => 80,
            "{" => 70,
            "*" | "/" | "%" => 60,
            "+" | "-" | "&" => 50,
            "=" | "<" | ">" | "^" | "!=" | "<=" | ">=" | "~>" | "?:" | "??" | "in" => 40,
            "and" => 30,
            "or" => 25,
            "?" => 20,
            ":=" | "(error)" => 10,
            _ => 0,
        }
    }

    /// The value jsonata-js reports as the `token` of an error at this symbol.
    fn value(&self) -> Option<serde_json::Value> {
        Some(match &self.kind {
            SymKind::Tok(TokKind::Name(s) | TokKind::Variable(s) | TokKind::Str(s)) => {
                s.clone().into()
            }
            SymKind::Tok(TokKind::Number(n)) => (*n).into(),
            SymKind::Tok(TokKind::Value(lit)) => lit_json(*lit),
            SymKind::Tok(TokKind::Regex { pattern, .. }) => pattern.clone().into(),
            SymKind::Tok(TokKind::Op(op)) => (*op).into(),
            SymKind::End => "(end)".into(),
            SymKind::Error => return None,
        })
    }

    fn is_op(&self, op: &str) -> bool {
        matches!(&self.kind, SymKind::Tok(TokKind::Op(o)) if *o == op)
    }
}

/// Operators the lexer recognises but the grammar does not (`!` and `~` alone).
fn is_unknown_operator(op: &str) -> bool {
    matches!(op, "!" | "~")
}

type PResult<T> = Result<T>;

/// Maximum depth of the syntax tree: nested subexpressions plus the operands of operator chains
/// such as `1 + 2 + 3`. Deeper expressions fail with `U1001` instead of exhausting the stack in
/// the passes that walk, clone or drop the tree.
pub(crate) const MAX_NESTING: usize = 1000;

pub(crate) struct Parser<'a> {
    src: &'a str,
    lexer: Lexer<'a>,
    node: Sym,
    prev_end: usize,
    recover: bool,
    /// Depth of the tree being built (nested `expression` calls and the operators applied in
    /// each), bounded by [`MAX_NESTING`].
    depth: usize,
    pub errors: Vec<Error>,
    pub warnings: Vec<Diagnostic>,
}

/// Parses `src` into a raw tree.
pub(crate) fn parse(src: &str, recover: bool) -> (PResult<Raw>, Vec<Error>, Vec<Diagnostic>) {
    let mut parser = Parser {
        src,
        lexer: Lexer::new(src),
        node: Sym {
            kind: SymKind::End,
            span: Span::new(0, 0),
            docs: Vec::new(),
        },
        prev_end: 0,
        recover,
        depth: 0,
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    let result = parser.parse_root();
    for doc in parser.lexer.take_trailing_docs() {
        parser
            .warnings
            .push(Diagnostic::warning("W0101", doc.span, &[]));
    }
    (result, parser.errors, parser.warnings)
}

impl<'a> Parser<'a> {
    fn parse_root(&mut self) -> PResult<Raw> {
        self.advance(None, false)?;
        let expr = self.expression(0)?;
        if !matches!(self.node.kind, SymKind::End) {
            let err = self.error_at_node("S0201");
            self.handle_error(err)?;
        }
        Ok(expr)
    }

    fn error_at_node(&self, code: &str) -> Error {
        let mut err = Error::new(code);
        if let Some(value) = self.node.value() {
            err = err.with("token", value);
        }
        err.located(self.src, self.node.span, self.node.span.end)
    }

    fn end_span(&self) -> Span {
        Span::new(self.src.len(), self.src.len())
    }

    /// Records an error. In recover mode the rest of the input is skipped and parsing carries
    /// on from an error marker; otherwise the error aborts parsing.
    fn handle_error(&mut self, err: Error) -> PResult<Raw> {
        if !self.recover {
            return Err(err);
        }
        self.drain();
        let span = err.span().unwrap_or(self.end_span());
        self.errors.push(err.clone());
        self.node = Sym {
            kind: SymKind::Error,
            span,
            docs: Vec::new(),
        };
        Ok(Raw::new(
            RawKind::Error {
                error: Some(Box::new(err)),
                lhs: None,
            },
            span,
            span.end,
        ))
    }

    /// Consumes the remaining tokens (the reference implementation attaches them to errors).
    /// Lexer errors in the rest of the input are ignored: the input ends here either way.
    fn drain(&mut self) {
        self.lexer.finish();
    }

    fn advance(&mut self, expect: Option<&str>, infix: bool) -> PResult<()> {
        if let Some(id) = expect
            && self.node.id() != id
        {
            let code = if matches!(self.node.kind, SymKind::End) {
                "S0203"
            } else {
                "S0202"
            };
            let err = self.error_at_node(code).with("value", id);
            self.handle_error(err)?;
            return Ok(());
        }
        for doc in std::mem::take(&mut self.node.docs) {
            self.warnings
                .push(Diagnostic::warning("W0101", doc.span, &[]));
        }
        if !matches!(self.node.kind, SymKind::Error) {
            self.prev_end = self.prev_end.max(self.node.span.end);
        }
        match self.lexer.next(infix) {
            Ok(None) => {
                self.node = Sym {
                    kind: SymKind::End,
                    span: self.end_span(),
                    docs: Vec::new(),
                };
            }
            Ok(Some(tok)) => {
                if let TokKind::Op(op) = tok.kind
                    && is_unknown_operator(op)
                {
                    let err = Error::new("S0204").with("token", op).located(
                        self.src,
                        tok.span,
                        tok.span.end,
                    );
                    self.handle_error(err)?;
                    return Ok(());
                }
                self.node = Sym {
                    kind: SymKind::Tok(tok.kind),
                    span: tok.span,
                    docs: tok.docs,
                };
            }
            Err(err) => {
                // jlens extension: lexer errors are recoverable too.
                self.handle_error(err)?;
            }
        }
        Ok(())
    }

    fn annotation(&mut self, docs: &[DocComment]) -> Option<Box<Annotation>> {
        let mut merged: Option<Annotation> = None;
        for doc in docs {
            let a = annotation::parse_doc(doc, &mut self.warnings);
            match &mut merged {
                Some(m) => m.merge(a),
                None => merged = Some(a),
            }
        }
        merged.map(Box::new)
    }

    /// Pratt's algorithm.
    fn expression(&mut self, rbp: u8) -> PResult<Raw> {
        let depth = self.depth;
        self.deeper()?;
        let result = crate::stack::grow(|| self.expression_inner(rbp));
        // Also undoes the levels added by the operators of this expression.
        self.depth = depth;
        result
    }

    /// Adds a level to the tree being built, failing with `U1001` past [`MAX_NESTING`].
    fn deeper(&mut self) -> PResult<()> {
        if self.depth >= MAX_NESTING || crate::stack::exhausted() {
            return Err(Error::new("U1001")
                .with(
                    "message",
                    format!(
                        "The expression is nested too deeply (the limit is {MAX_NESTING} levels)"
                    ),
                )
                .located(self.src, self.node.span, self.node.span.end));
        }
        self.depth += 1;
        Ok(())
    }

    fn expression_inner(&mut self, rbp: u8) -> PResult<Raw> {
        let docs = std::mem::take(&mut self.node.docs);
        let t = self.node.clone();
        self.advance(None, true)?;
        let mut left = self.nud(t)?;
        let annotation = self.annotation(&docs);
        let inner = if annotation.is_some() {
            rbp.max(ANNOTATION_BP)
        } else {
            rbp
        };
        while inner < self.node.lbp() {
            left = self.led_step(left)?;
        }
        if let Some(a) = annotation {
            match &mut left.annotation {
                Some(existing) => {
                    let mut outer = *a;
                    outer.merge(*existing.clone());
                    **existing = outer;
                }
                None => left.annotation = Some(a),
            }
            while rbp < self.node.lbp() {
                left = self.led_step(left)?;
            }
        }
        Ok(left)
    }

    /// Applies the operator at the current symbol to `left`: one more level of the tree, so that
    /// a long chain such as `1 + 2 + … + 5000` cannot build a tree deeper than the limit.
    fn led_step(&mut self, left: Raw) -> PResult<Raw> {
        self.deeper()?;
        let t = self.node.clone();
        self.advance(None, false)?;
        self.led(t, left)
    }

    fn span_from(&self, start: usize) -> Span {
        Span::new(start, self.prev_end.max(start))
    }

    fn nud(&mut self, t: Sym) -> PResult<Raw> {
        let start = t.span.start;
        let pos = t.span.end;
        let kind = match t.kind {
            SymKind::Tok(TokKind::Name(value)) => RawKind::Name { value },
            SymKind::Tok(TokKind::Variable(value)) => RawKind::Variable { value },
            SymKind::Tok(TokKind::Str(value)) => RawKind::String { value },
            SymKind::Tok(TokKind::Number(value)) => RawKind::Number { value },
            SymKind::Tok(TokKind::Value(value)) => RawKind::Value { value },
            SymKind::Tok(TokKind::Regex { pattern, flags }) => RawKind::Regex { pattern, flags },
            SymKind::End => return Ok(Raw::new(RawKind::End, t.span, t.span.end)),
            SymKind::Tok(TokKind::Op(op)) => match op {
                // keywords used as field names
                "and" | "or" | "in" => RawKind::Name {
                    value: op.to_owned(),
                },
                "-" => RawKind::Negate {
                    expr: Box::new(self.expression(70)?),
                },
                "*" => RawKind::Wildcard,
                "**" => RawKind::Descendant,
                "%" => RawKind::Parent,
                "(" => {
                    let mut exprs = Vec::new();
                    while self.node.id() != ")" {
                        exprs.push(self.expression(0)?);
                        if self.node.id() != ";" {
                            break;
                        }
                        self.advance(Some(";"), false)?;
                    }
                    self.advance(Some(")"), true)?;
                    RawKind::Block { exprs }
                }
                "[" => {
                    let mut items = Vec::new();
                    if self.node.id() != "]" {
                        loop {
                            let mut item = self.expression(0)?;
                            if self.node.id() == ".." {
                                let range_pos = self.node.span.end;
                                self.advance(Some(".."), false)?;
                                let rhs = self.expression(0)?;
                                let span = item.span.to(rhs.span);
                                item = Raw::new(
                                    RawKind::Binary {
                                        op: "..",
                                        lhs: Box::new(item),
                                        rhs: Box::new(rhs),
                                    },
                                    span,
                                    range_pos,
                                );
                            }
                            items.push(item);
                            if self.node.id() != "," {
                                break;
                            }
                            self.advance(Some(","), false)?;
                        }
                    }
                    self.advance(Some("]"), true)?;
                    RawKind::Array { items }
                }
                "{" => RawKind::Object {
                    pairs: self.object_pairs()?,
                },
                "|" => {
                    let pattern = self.expression(0)?;
                    self.advance(Some("|"), false)?;
                    let update = self.expression(0)?;
                    let mut delete = None;
                    if self.node.id() == "," {
                        self.advance(Some(","), false)?;
                        delete = Some(Box::new(self.expression(0)?));
                    }
                    self.advance(Some("|"), false)?;
                    RawKind::Transform {
                        pattern: Box::new(pattern),
                        update: Box::new(update),
                        delete,
                    }
                }
                _ => return self.prefix_error(&t),
            },
            // An expression starting at the error marker: reuse the error already recorded
            // instead of reporting a spurious S0211 as jsonata-js does.
            SymKind::Error => RawKind::Error {
                error: self.errors.last().cloned().map(Box::new),
                lhs: None,
            },
        };
        Ok(Raw::new(kind, self.span_from(start), pos))
    }

    /// S0211: a symbol that cannot start an expression.
    fn prefix_error(&mut self, t: &Sym) -> PResult<Raw> {
        let mut err = Error::new("S0211");
        if let Some(value) = t.value() {
            err = err.with("token", value);
        }
        let err = err.located(self.src, t.span, t.span.end);
        if !self.recover {
            return Err(err);
        }
        self.drain();
        self.errors.push(err.clone());
        Ok(Raw::new(
            RawKind::Error {
                error: Some(Box::new(err)),
                lhs: None,
            },
            t.span,
            t.span.end,
        ))
    }

    fn object_pairs(&mut self) -> PResult<Vec<(Raw, Raw)>> {
        let mut pairs = Vec::new();
        if self.node.id() != "}" {
            loop {
                let name = self.expression(0)?;
                self.advance(Some(":"), false)?;
                let value = self.expression(0)?;
                pairs.push((name, value));
                if self.node.id() != "," {
                    break;
                }
                self.advance(Some(","), false)?;
            }
        }
        self.advance(Some("}"), true)?;
        Ok(pairs)
    }

    fn led(&mut self, t: Sym, left: Raw) -> PResult<Raw> {
        let start = left.span.start;
        let pos = t.span.end;
        let op = t.id();
        let kind = match op {
            "(error)" => {
                let (span, pos) = (left.span, left.pos);
                return Ok(Raw::new(
                    RawKind::Error {
                        error: None,
                        lhs: Some(Box::new(left)),
                    },
                    span,
                    pos,
                ));
            }
            "??" | "?:" => {
                // The right side extends as far as possible, like jsonata-js. The left side is
                // not copied into a condition, as jsonata-js does: nesting would double the tree.
                let rhs = self.expression(0)?;
                RawKind::Binary {
                    op,
                    lhs: Box::new(left),
                    rhs: Box::new(rhs),
                }
            }
            "(" => return self.call(t, left),
            "[" => {
                if self.node.id() == "]" {
                    // `[]`: keep singleton arrays in the output.
                    let mut left = left;
                    mark_keep_array(&mut left);
                    self.advance(Some("]"), false)?;
                    left.span = self.span_from(start);
                    return Ok(left);
                }
                let rhs = self.expression(0)?;
                self.advance(Some("]"), true)?;
                RawKind::Binary {
                    op: "[",
                    lhs: Box::new(left),
                    rhs: Box::new(rhs),
                }
            }
            "^" => {
                self.advance(Some("("), false)?;
                let mut terms = Vec::new();
                loop {
                    let mut descending = false;
                    if self.node.id() == "<" {
                        self.advance(Some("<"), false)?;
                    } else if self.node.id() == ">" {
                        descending = true;
                        self.advance(Some(">"), false)?;
                    }
                    terms.push((descending, self.expression(0)?));
                    if self.node.id() != "," {
                        break;
                    }
                    self.advance(Some(","), false)?;
                }
                self.advance(Some(")"), false)?;
                RawKind::Sort {
                    lhs: Box::new(left),
                    terms,
                }
            }
            "{" => RawKind::Group {
                lhs: Box::new(left),
                pairs: self.object_pairs()?,
            },
            ":=" => {
                if !matches!(left.kind, RawKind::Variable { .. }) {
                    let err = Error::new("S0212")
                        .with("token", left.token_value())
                        .located(self.src, left.span, left.pos);
                    return self.handle_error(err);
                }
                let rhs = self.expression(t.lbp() - 1)?;
                RawKind::Binary {
                    op: ":=",
                    lhs: Box::new(left),
                    rhs: Box::new(rhs),
                }
            }
            "@" | "#" => {
                let rhs = self.expression(80)?;
                if !matches!(rhs.kind, RawKind::Variable { .. }) {
                    let err = Error::new("S0214")
                        .with("token", op)
                        .located(self.src, rhs.span, rhs.pos);
                    return self.handle_error(err);
                }
                RawKind::Binary {
                    op,
                    lhs: Box::new(left),
                    rhs: Box::new(rhs),
                }
            }
            "?" => {
                let then = self.expression(0)?;
                let mut otherwise = None;
                if self.node.id() == ":" {
                    self.advance(Some(":"), false)?;
                    otherwise = Some(Box::new(self.expression(0)?));
                }
                RawKind::Condition {
                    condition: Box::new(left),
                    then: Box::new(then),
                    otherwise,
                }
            }
            _ => {
                // `.`, arithmetic, comparison, `and`/`or`/`in`, `&`, `~>`
                let rhs = self.expression(t.lbp())?;
                RawKind::Binary {
                    op,
                    lhs: Box::new(left),
                    rhs: Box::new(rhs),
                }
            }
        };
        Ok(Raw::new(kind, self.span_from(start), pos))
    }

    /// Function call, partial application or lambda definition: `left(args)`.
    fn call(&mut self, t: Sym, left: Raw) -> PResult<Raw> {
        let start = left.span.start;
        let mut args = Vec::new();
        let mut partial = false;
        if self.node.id() != ")" {
            loop {
                if self.node.is_op("?") {
                    partial = true;
                    args.push(Raw::new(
                        RawKind::Placeholder,
                        self.node.span,
                        self.node.span.end,
                    ));
                    self.advance(Some("?"), false)?;
                } else {
                    args.push(self.expression(0)?);
                }
                if self.node.id() != "," {
                    break;
                }
                self.advance(Some(","), false)?;
            }
        }
        self.advance(Some(")"), true)?;

        let is_lambda =
            matches!(&left.kind, RawKind::Name { value } if value == "function" || value == "λ");
        if !is_lambda {
            return Ok(Raw::new(
                RawKind::Function {
                    procedure: Box::new(left),
                    args,
                    partial,
                },
                self.span_from(start),
                t.span.end,
            ));
        }

        for (index, arg) in args.iter().enumerate() {
            if !matches!(arg.kind, RawKind::Variable { .. }) {
                let err = Error::new("S0208")
                    .with("token", arg.token_value())
                    .with("value", index + 1)
                    .located(self.src, arg.span, arg.pos);
                self.handle_error(err)?;
            }
        }
        let mut signature = None;
        if self.node.id() == "<" {
            let sig_start = self.node.span.start;
            let mut depth = 1;
            let mut text = String::from("<");
            while depth > 0 && self.node.id() != "{" && self.node.id() != "(end)" {
                self.advance(None, false)?;
                match self.node.id() {
                    ">" => depth -= 1,
                    "<" => depth += 1,
                    _ => {}
                }
                if !matches!(self.node.kind, SymKind::End | SymKind::Error) {
                    text.push_str(&self.src[self.node.span.start..self.node.span.end]);
                }
            }
            self.advance(Some(">"), false)?;
            match Signature::parse(&text) {
                Ok(sig) => signature = Some(sig),
                Err(err) => {
                    let offset = err
                        .detail("offset")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as usize;
                    let at = (sig_start + 1 + offset).min(self.src.len());
                    let err = err.located(self.src, Span::new(sig_start, self.prev_end), at);
                    return self.handle_error(err);
                }
            }
        }
        self.advance(Some("{"), false)?;
        let body = self.expression(0)?;
        self.advance(Some("}"), false)?;
        Ok(Raw::new(
            RawKind::Lambda {
                params: args,
                signature,
                body: Box::new(body),
            },
            self.span_from(start),
            t.span.end,
        ))
    }
}

/// Applies the `[]` modifier to the step that the predicates in `raw` filter.
fn mark_keep_array(raw: &mut Raw) {
    match &mut raw.kind {
        RawKind::Binary { op: "[", lhs, .. } => mark_keep_array(lhs),
        _ => raw.keep_array = true,
    }
}
