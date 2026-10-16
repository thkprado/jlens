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
//! (MIT license, reproduced in `LICENSE`); the ported modules say so in their documentation.

pub mod annotation;
pub mod ast;
pub mod audit;
mod datetime;
pub mod debug;
mod document;
pub mod error;
mod eval;
pub mod functions;
mod host;
pub mod input;
mod json;
mod lexer;
mod parser;
mod process;
pub mod reads;
mod regex;
pub mod signature;
pub mod stack;
mod value;

pub use annotation::Annotation;
pub use ast::{Ast, Node, NodeKind, Span};
pub use document::Document;
pub use error::{Diagnostic, Error, Result, Severity};
pub use functions::{Category, FunctionInfo, catalog as function_catalog};
pub use host::{Callable, HostCall, HostFunction};
pub use json::format_json;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

    /// What the expression may read of its input and of its bindings, found without
    /// evaluating it ([ADR 0016]). Evaluated on its input and bindings cut by the result
    /// ([`reads::Read::cut`]), the expression gives the same result or error code. See the
    /// [`reads`] module.
    ///
    /// The analysis runs on each call, in time linear in the size of the expression.
    ///
    /// [ADR 0016]: https://github.com/thkprado/jlens/blob/main/docs/src/adr/0016-fields-read.md
    pub fn reads(&self) -> reads::Reads {
        reads::analyse(&self.ast)
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
        let shared = value::Shared::new();
        eval::evaluate(
            &self.ast,
            input.map_or(value::Value::Undefined, |json| {
                value::Value::lazy(json, &shared)
            }),
            bindings
                .iter()
                .map(|(name, json)| (name, value::Value::lazy(json, &shared))),
            bindings,
            limits(options),
            None,
            None,
        )
    }

    /// Evaluates the expression against an input read from a [`Source`](input::Source), one
    /// level at a time, as the evaluation reaches it. Use it for an input that is not a
    /// `serde_json::Value`, without converting it first.
    ///
    /// `input` writes the input document as the only value of its level, or nothing for
    /// *undefined*. A level that the source fails to read stops the evaluation with `U1004`.
    pub fn evaluate_source<'a>(
        &self,
        input: &dyn input::Source<'a>,
        bindings: &'a Bindings,
        options: &Options,
    ) -> Result<Option<JsonValue>> {
        let shared = value::Shared::new();
        eval::evaluate(
            &self.ast,
            value::Value::from_source(input, &shared)?,
            bindings
                .iter()
                .map(|(name, json)| (name, value::Value::lazy(json, &shared))),
            bindings,
            limits(options),
            None,
            Some(shared.clone()),
        )
    }

    /// Evaluates the expression against a [`Document`] converted beforehand. Use it to
    /// evaluate several expressions, or the same one several times, against one large document
    /// without converting it each time.
    pub fn evaluate_document(
        &self,
        document: &Document,
        bindings: &Bindings,
        options: &Options,
    ) -> Result<Option<JsonValue>> {
        eval::evaluate(
            &self.ast,
            document.value().clone(),
            bindings
                .iter()
                .map(|(name, json)| (name, value::Value::from_json(json))),
            bindings,
            limits(options),
            None,
            None,
        )
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
        let shared = value::Shared::new();
        eval::evaluate(
            &self.ast,
            input.map_or(value::Value::Undefined, |json| {
                value::Value::lazy(json, &shared)
            }),
            bindings
                .iter()
                .map(|(name, json)| (name, value::Value::lazy(json, &shared))),
            bindings,
            limits(options),
            Some(tracer),
            None,
        )
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
        max_string_length: options
            .max_string_length
            .unwrap_or(eval::DEFAULT_MAX_STRING_LENGTH),
        max_array_length: options
            .max_array_length
            .unwrap_or(eval::DEFAULT_MAX_ARRAY_LENGTH),
        max_result_size: options
            .max_result_size
            .unwrap_or(eval::DEFAULT_MAX_RESULT_SIZE),
        cancel_token: options.cancel_token.clone(),
        now: options.now,
        seed: options.seed,
    }
}

/// Variables and host functions made available to an expression as `$name`.
///
/// A name holds either a value or a [`HostFunction`]: binding one replaces the other. Bindings
/// take precedence over the built-in functions of the same name, as in jsonata-js.
#[derive(Debug, Clone, Default)]
pub struct Bindings {
    vars: IndexMap<String, JsonValue>,
    functions: IndexMap<String, HostFunction>,
}

impl Bindings {
    /// Creates an empty set of bindings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds (or replaces) a variable binding. The name is given without the leading `$`.
    pub fn var(mut self, name: impl Into<String>, value: impl Into<JsonValue>) -> Self {
        let name = name.into();
        self.functions.shift_remove(&name);
        self.vars.insert(name, value.into());
        self
    }

    /// Adds (or replaces) a host function, called as `$name(...)`. The name is given without
    /// the leading `$`.
    pub fn function(mut self, name: impl Into<String>, function: HostFunction) -> Self {
        let name = name.into();
        self.vars.shift_remove(&name);
        self.functions.insert(name, function);
        self
    }

    /// Looks up a variable binding by name (without the leading `$`).
    pub fn get(&self, name: &str) -> Option<&JsonValue> {
        self.vars.get(name)
    }

    /// Looks up a host function by name (without the leading `$`).
    pub fn get_function(&self, name: &str) -> Option<&HostFunction> {
        self.functions.get(name)
    }

    /// Iterates over the variable bindings in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &JsonValue)> {
        self.vars.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Iterates over the host functions in insertion order.
    pub fn functions(&self) -> impl Iterator<Item = (&str, &HostFunction)> {
        self.functions.iter().map(|(k, f)| (k.as_str(), f))
    }
}

impl FromIterator<(String, JsonValue)> for Bindings {
    fn from_iter<T: IntoIterator<Item = (String, JsonValue)>>(iter: T) -> Self {
        Bindings {
            vars: iter.into_iter().collect(),
            functions: IndexMap::new(),
        }
    }
}

/// Evaluation guardrails, and the clock and seed that make an evaluation reproducible.
///
/// The struct is `#[non_exhaustive]`, so new options can be added without breaking callers:
/// start from [`Options::new`] (or `Options::default()`) and chain the builder methods.
///
/// ```
/// use jlens_core::Options;
/// use std::time::Duration;
///
/// let options = Options::new()
///     .timeout(Duration::from_millis(100))
///     .max_depth(500);
/// assert_eq!(options.max_depth, Some(500));
/// ```
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Options {
    /// Maximum wall-clock time for one evaluation. Exceeding it raises `D1012`.
    pub timeout: Option<Duration>,
    /// Maximum evaluation depth (default 50,000; 500 on WebAssembly). Exceeding it raises
    /// `D1011`.
    pub max_depth: Option<usize>,
    /// The evaluation's timestamp, in milliseconds since the epoch, for `$now()` and
    /// `$millis()`. Default: the system clock, read once, the first time the evaluation needs
    /// it.
    pub now: Option<i64>,
    /// Seed for `$random()` and `$shuffle()`. Default: one taken from the system clock.
    pub seed: Option<u64>,
    /// Stops the evaluation with `U1003` once the token is cancelled, from any thread.
    pub cancel_token: Option<CancelToken>,
    /// Maximum length, in bytes of UTF-8, of a string that the evaluation builds (default
    /// 268,435,456: 256 MiB). Exceeding it raises `U1001`.
    pub max_string_length: Option<usize>,
    /// Maximum number of items in an array or sequence that the evaluation builds (default
    /// 16,777,216). Exceeding it raises `U1001`.
    pub max_array_length: Option<usize>,
    /// Maximum size of the result as compact JSON text, in bytes (default 268,435,456:
    /// 256 MiB), which also limits the arguments of a host function. A value that appears
    /// several times counts each time, as it does in the text. Exceeding it raises `U1001`.
    pub max_result_size: Option<usize>,
}

impl Options {
    /// Creates options with no timeout, the default depth and size limits, the system clock
    /// and an unseeded generator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fixes the evaluation's timestamp (`$now()`, `$millis()`), in milliseconds since the
    /// epoch.
    pub fn now(mut self, millis: i64) -> Self {
        self.now = Some(millis);
        self
    }

    /// Seeds `$random()` and `$shuffle()`: the same seed gives the same sequence.
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Sets the maximum wall-clock time for one evaluation (`D1012` when exceeded).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets the maximum evaluation depth (`D1011` when exceeded).
    pub fn max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = Some(max_depth);
        self
    }

    /// Sets the maximum length, in bytes, of a string the evaluation builds (`U1001` when
    /// exceeded).
    pub fn max_string_length(mut self, bytes: usize) -> Self {
        self.max_string_length = Some(bytes);
        self
    }

    /// Sets the maximum number of items of an array the evaluation builds (`U1001` when
    /// exceeded).
    pub fn max_array_length(mut self, items: usize) -> Self {
        self.max_array_length = Some(items);
        self
    }

    /// Sets the maximum size, in bytes of compact JSON text, of the result and of the
    /// arguments of a host function (`U1001` when exceeded).
    pub fn max_result_size(mut self, bytes: usize) -> Self {
        self.max_result_size = Some(bytes);
        self
    }

    /// Lets `token` stop the evaluation (`U1003` once it is cancelled).
    pub fn cancel_token(mut self, token: CancelToken) -> Self {
        self.cancel_token = Some(token);
        self
    }
}

/// A handle that stops evaluations from another thread.
///
/// Clones share one flag: pass a clone in [`Options::cancel_token`], keep another, and call
/// [`CancelToken::cancel`] to stop every evaluation that uses it. The evaluation checks the
/// flag between steps, as it checks the timeout, and fails with `U1003`; a host function
/// runs to completion first. Cancelling cannot be undone: an evaluation started with a
/// cancelled token fails at once, so use a new token for each job.
///
/// ```
/// use jlens_core::{Bindings, CancelToken, Expression, Options};
///
/// let token = CancelToken::new();
/// let options = Options::new().cancel_token(token.clone());
/// token.cancel();
/// let err = Expression::compile("1 + 1")
///     .unwrap()
///     .evaluate_with(None, &Bindings::new(), &options)
///     .unwrap_err();
/// assert_eq!(err.code(), "U1003");
/// ```
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// Creates a token that is not cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancels the token: evaluations that use it stop with `U1003`.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Returns whether [`CancelToken::cancel`] has been called on this token or a clone.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}
