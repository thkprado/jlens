//! # jlens-core
//!
//! The JSONata language engine behind jlens: lexer, parser, evaluator, function library and
//! the evaluation audit subsystem.
//!
//! Compile an expression once, then evaluate it against any number of JSON documents:
//!
//! ```
//! use jlens_core::{Bindings, Expression};
//! use serde_json::json;
//!
//! let expr = Expression::compile("$sum(items[type = $kind].(price * qty))").unwrap();
//! let input = json!({"items": [
//!     {"type": "a", "price": 2.5, "qty": 4},
//!     {"type": "b", "price": 1.0, "qty": 1},
//! ]});
//! let bindings = Bindings::new().var("kind", "a");
//! assert_eq!(expr.evaluate(Some(&input), &bindings).unwrap(), Some(json!(10)));
//!
//! let err = Expression::compile("Account.Order[").unwrap_err();
//! assert_eq!(err.code(), "S0203");
//! ```
//!
//! For editors, [`parse_recover`] keeps going after syntax errors and returns a partial tree
//! together with [`Diagnostic`]s:
//!
//! ```
//! let ast = jlens_core::parse_recover("Account.");
//! assert!(ast.has_errors());
//! assert_eq!(ast.diagnostics[0].code, "S0207");
//! ```
//!
//! Parts of this crate are ported from [jsonata-js](https://github.com/jsonata-js/jsonata)
//! (© IBM Corp. and JSONata contributors, MIT license); the ported modules say so in their
//! documentation.

pub mod annotation;
pub mod ast;
pub mod audit;
mod datetime;
pub mod debug;
pub mod error;
mod eval;
pub mod functions;
mod json;
mod lexer;
mod parser;
mod process;
mod regex;
pub mod signature;
mod value;

pub use annotation::Annotation;
pub use ast::{Ast, Node, NodeKind, Span};
pub use error::{Diagnostic, Error, Result, Severity};
pub use functions::{Category, FunctionInfo, catalog as function_catalog};
pub use json::format_json;

use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexMap;
use serde_json::Value as JsonValue;

/// Parses an expression, failing on the first syntax error.
///
/// Warnings (such as misplaced annotations) are returned in [`Ast::diagnostics`].
pub fn parse(source: &str) -> Result<Ast> {
    let (raw, errors, warnings) = parser::parse(source, false);
    debug_assert!(errors.is_empty());
    let mut processor = process::Processor::new(source, false);
    let root = processor.process_root(raw?)?;
    Ok(Ast {
        root,
        slots: processor.slots,
        diagnostics: warnings,
        source: source.to_owned(),
    })
}

/// Parses an expression, recovering from syntax errors.
///
/// The returned tree contains [`NodeKind::Error`] nodes where the input could not be parsed,
/// and [`Ast::diagnostics`] lists every error and warning, ordered by position. This is the
/// entry point for editors and language tooling.
pub fn parse_recover(source: &str) -> Ast {
    let (raw, parse_errors, warnings) = parser::parse(source, true);
    let mut processor = process::Processor::new(source, true);
    let root = raw
        .and_then(|raw| processor.process_root(raw))
        .unwrap_or_else(|err| {
            let span = err.span().unwrap_or_default();
            processor.errors.push(err.clone());
            Node::new(
                NodeKind::Error {
                    error: Box::new(err),
                },
                span,
                span.end,
            )
        });
    let mut diagnostics: Vec<Diagnostic> = parse_errors
        .iter()
        .chain(&processor.errors)
        .map(Diagnostic::from)
        .collect();
    diagnostics.extend(warnings);
    diagnostics.sort_by_key(|d| d.span.start);
    Ast {
        root,
        slots: processor.slots,
        diagnostics,
        source: source.to_owned(),
    }
}

/// A compiled JSONata expression.
///
/// Compile once with [`Expression::compile`] and evaluate many times. A compiled expression is
/// immutable and can be shared between threads.
#[derive(Debug, Clone)]
pub struct Expression {
    ast: Arc<Ast>,
}

impl Expression {
    /// Parses and compiles a JSONata expression.
    pub fn compile(source: &str) -> Result<Self> {
        Ok(Expression {
            ast: Arc::new(parse(source)?),
        })
    }

    /// Returns the source text this expression was compiled from.
    pub fn source(&self) -> &str {
        &self.ast.source
    }

    /// The syntax tree of the expression.
    pub fn ast(&self) -> &Ast {
        &self.ast
    }

    /// Evaluates the expression against `input` with default [`Options`].
    ///
    /// `None` as input means *undefined* (no input document). A result of `Ok(None)` means
    /// the expression evaluated to *undefined*.
    pub fn evaluate(
        &self,
        input: Option<&JsonValue>,
        bindings: &Bindings,
    ) -> Result<Option<JsonValue>> {
        self.evaluate_with(input, bindings, &Options::default())
    }

    /// Evaluates the expression with explicit [`Options`] (limits and guardrails).
    pub fn evaluate_with(
        &self,
        input: Option<&JsonValue>,
        bindings: &Bindings,
        options: &Options,
    ) -> Result<Option<JsonValue>> {
        eval::evaluate(&self.ast, input, bindings, limits(options), None)
    }

    /// Evaluates the expression, reporting every evaluation event to `tracer`.
    ///
    /// See the [`audit`] module for the events and the tracers jlens provides.
    pub fn evaluate_traced(
        &self,
        input: Option<&JsonValue>,
        bindings: &Bindings,
        options: &Options,
        tracer: &mut dyn audit::Tracer,
    ) -> Result<Option<JsonValue>> {
        eval::evaluate(&self.ast, input, bindings, limits(options), Some(tracer))
    }

    /// Evaluates the expression and records an [`audit::AuditTree`] of how the result was
    /// reached. The audit is returned even when the evaluation fails.
    ///
    /// ```
    /// use jlens_core::{Bindings, Expression, Options};
    /// use jlens_core::audit::{AuditOptions, OutcomeKind};
    ///
    /// let expr = Expression::compile("$x > 10").unwrap();
    /// let bindings = Bindings::new().var("x", 3);
    /// let (result, audit) =
    ///     expr.evaluate_audited(None, &bindings, &Options::default(), &AuditOptions::decisions());
    /// assert_eq!(result.unwrap(), Some(serde_json::json!(false)));
    /// let root = audit.root.unwrap();
    /// assert_eq!(root.outcome, OutcomeKind::False);
    /// assert_eq!(root.operands, Some([Some(3.into()), Some(10.into())]));
    /// ```
    pub fn evaluate_audited(
        &self,
        input: Option<&JsonValue>,
        bindings: &Bindings,
        options: &Options,
        audit: &audit::AuditOptions,
    ) -> (Result<Option<JsonValue>>, audit::AuditTree) {
        let mut collector = audit::AuditCollector::new(*audit, input);
        let result = self.evaluate_traced(input, bindings, options, &mut collector);
        (result, collector.finish())
    }
}

fn limits(options: &Options) -> eval::Limits {
    eval::Limits {
        max_depth: options.max_depth.unwrap_or(eval::DEFAULT_MAX_DEPTH),
        timeout: options.timeout,
    }
}

/// Variable bindings made available to an expression as `$name`.
#[derive(Debug, Clone, Default)]
pub struct Bindings {
    vars: IndexMap<String, JsonValue>,
}

impl Bindings {
    /// Creates an empty set of bindings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds (or replaces) a variable binding. The name is given without the leading `$`.
    pub fn var(mut self, name: impl Into<String>, value: impl Into<JsonValue>) -> Self {
        self.vars.insert(name.into(), value.into());
        self
    }

    /// Looks up a variable binding by name (without the leading `$`).
    pub fn get(&self, name: &str) -> Option<&JsonValue> {
        self.vars.get(name)
    }

    /// Iterates over all bindings in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &JsonValue)> {
        self.vars.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl FromIterator<(String, JsonValue)> for Bindings {
    fn from_iter<T: IntoIterator<Item = (String, JsonValue)>>(iter: T) -> Self {
        Bindings {
            vars: iter.into_iter().collect(),
        }
    }
}

/// Evaluation guardrails.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Maximum wall-clock time for one evaluation. Exceeding it raises `U1001`.
    pub timeout: Option<Duration>,
    /// Maximum evaluation depth. Exceeding it raises `U1001`.
    pub max_depth: Option<usize>,
}
