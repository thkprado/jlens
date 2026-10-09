//! # jlens-core
//!
//! The JSONata language engine behind jlens: lexer, parser, evaluator, function library and
//! the evaluation audit subsystem.
//!
//! The crate is being built in phases (see `docs/src/architecture/roadmap.md`). The public
//! API below is the stable entry point; until the parser and evaluator land, compiling an
//! expression returns an [`Error`] with code [`Error::UNIMPLEMENTED`].
//!
//! ```
//! use jlens_core::{Bindings, Expression};
//!
//! match Expression::compile("Account.Order.Product.Price") {
//!     Ok(expr) => {
//!         let input = serde_json::json!({"Account": {"Order": []}});
//!         let _ = expr.evaluate(Some(&input), &Bindings::new());
//!     }
//!     Err(err) => assert_eq!(err.code(), "X0000"),
//! }
//! ```

mod error;

pub use error::{Error, Result};

use std::time::Duration;

use indexmap::IndexMap;
use serde_json::Value as JsonValue;

/// A compiled JSONata expression.
///
/// Compile once with [`Expression::compile`] and evaluate many times. A compiled expression is
/// immutable and can be shared between threads.
#[derive(Debug, Clone)]
pub struct Expression {
    source: String,
}

impl Expression {
    /// Parses and compiles a JSONata expression.
    pub fn compile(source: &str) -> Result<Self> {
        let _ = source;
        Err(Error::unimplemented("the JSONata parser"))
    }

    /// Returns the source text this expression was compiled from.
    pub fn source(&self) -> &str {
        &self.source
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
        _input: Option<&JsonValue>,
        _bindings: &Bindings,
        _options: &Options,
    ) -> Result<Option<JsonValue>> {
        Err(Error::unimplemented("the JSONata evaluator"))
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
