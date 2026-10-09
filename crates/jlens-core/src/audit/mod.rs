//! Evaluation logging and auditing (see ADR 0004).
//!
//! The evaluator reports what it does to a [`Tracer`]: it enters and leaves nodes, compares
//! operands, short-circuits `and`/`or`, takes conditional branches, binds variables and filters
//! sequences. Tracers turn these events into something useful:
//!
//! * [`AuditCollector`] builds an [`AuditTree`], which records how the result was reached with
//!   business labels from [annotations](crate::annotation). [`explain`] summarizes it.
//! * [`JsonlTracer`] writes one JSON object per event, for offline analysis.
//! * `TracingTracer` (feature `tracing`) forwards events to the `tracing` crate.
//!
//! ```
//! use jlens_core::{Bindings, Expression, Options};
//! use jlens_core::audit::{AuditOptions, explain};
//! use serde_json::json;
//!
//! let expr = Expression::compile(
//!     r#"/** @label "Big order" */ total > 100 and /** @label "Known customer" */ customer != null"#,
//! ).unwrap();
//! let input = json!({"total": 250, "customer": null});
//! let (result, audit) =
//!     expr.evaluate_audited(Some(&input), &Bindings::new(), &Options::default(), &AuditOptions::decisions());
//! assert_eq!(result.unwrap(), Some(json!(false)));
//! let text = explain(&audit).to_string();
//! assert!(text.contains("✘ Known customer"));
//! ```

mod collector;
mod explain;
mod jsonl;
#[cfg(feature = "tracing")]
mod tracing_tracer;

pub use collector::AuditCollector;
pub use explain::{Explanation, ExplanationLine, explain};
pub use jsonl::JsonlTracer;
#[cfg(feature = "tracing")]
pub use tracing_tracer::TracingTracer;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::ast::{Node, NodeId, Span};
use crate::error::Error;
use crate::value::Value;

/// Receives evaluation events. Every method has an empty default implementation.
///
/// Events nest: between `enter(node)` and the matching `exit(node)`, all events concern the
/// evaluation of `node` and of its descendants. The same node is entered many times when it is
/// evaluated for several items or inside a function called repeatedly.
pub trait Tracer {
    /// The evaluation starts.
    fn begin(&mut self, info: &EvalInfo) {
        let _ = info;
    }
    /// Evaluation of `node` against `input` starts.
    fn enter(&mut self, node: &Node, input: ValueRef<'_>) {
        let _ = (node, input);
    }
    /// Evaluation of `node` ends with a value or an error.
    fn exit(&mut self, node: &Node, outcome: Outcome<'_>) {
        let _ = (node, outcome);
    }
    /// A comparison (`=`, `!=`, `<`, `<=`, `>`, `>=`, `in`) is about to compare these operands.
    fn operands(&mut self, node: &Node, lhs: ValueRef<'_>, rhs: ValueRef<'_>) {
        let _ = (node, lhs, rhs);
    }
    /// The right side of `and`/`or` was not evaluated because the left side decided.
    fn short_circuit(&mut self, node: &Node) {
        let _ = node;
    }
    /// A conditional took a branch.
    fn branch(&mut self, node: &Node, taken: Branch) {
        let _ = (node, taken);
    }
    /// `$name := value` bound a variable.
    fn bind(&mut self, node: &Node, name: &str, value: ValueRef<'_>) {
        let _ = (node, name, value);
    }
    /// A predicate filter starts testing the items of a sequence.
    fn filter_begin(&mut self, predicate: &Node) {
        let _ = predicate;
    }
    /// A predicate filter tested one item.
    fn filter_item(&mut self, predicate: &Node, index: usize, item: ValueRef<'_>, passed: bool) {
        let _ = (predicate, index, item, passed);
    }
    /// A predicate filter finished.
    fn filter_end(&mut self, predicate: &Node) {
        let _ = predicate;
    }
    /// The evaluation ended.
    fn end(&mut self, outcome: Outcome<'_>) {
        let _ = outcome;
    }
}

/// Facts about an evaluation, given to [`Tracer::begin`].
#[derive(Debug, Clone)]
pub struct EvalInfo<'a> {
    /// Source text of the expression.
    pub source: &'a str,
    /// The evaluation's timestamp (`$millis()`), in milliseconds since the epoch.
    pub timestamp: f64,
}

/// A read-only view of a run-time value.
#[derive(Clone, Copy)]
pub struct ValueRef<'a>(pub(crate) &'a Value);

impl ValueRef<'_> {
    /// The value as JSON (`None` for undefined and functions).
    pub fn to_json(&self) -> Option<JsonValue> {
        self.0.to_json()
    }

    /// The value as JSON, truncated according to `limits`.
    pub fn capture(&self, limits: &CaptureLimits) -> Option<JsonValue> {
        capture(self.0, limits, 0)
    }

    /// Whether the value is undefined.
    pub fn is_undefined(&self) -> bool {
        self.0.is_undefined()
    }

    /// The value if it is a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self.0 {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The JSONata type name (`string`, `number`, `boolean`, `null`, `array`, `object`,
    /// `function`, or `undefined`).
    pub fn type_name(&self) -> &'static str {
        match self.0 {
            Value::Undefined => "undefined",
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
            Value::Function(_) => "function",
        }
    }
}

impl std::fmt::Debug for ValueRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// How the evaluation of a node ended.
#[derive(Debug, Clone, Copy)]
pub enum Outcome<'a> {
    /// It produced a value (possibly undefined).
    Value(ValueRef<'a>),
    /// It raised an error.
    Error(&'a Error),
}

/// The branch taken by a conditional.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Branch {
    /// The condition held.
    Then,
    /// The condition did not hold and there is an `else` part.
    Else,
    /// The condition did not hold and there is no `else` part.
    None,
}

/// How much an [`AuditCollector`] records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditLevel {
    /// Nothing; the tree only has a header.
    Off,
    /// Only nodes that decide something: comparisons, `and`/`or`, conditionals, variable
    /// bindings, nodes producing booleans, filters, and annotated nodes.
    #[default]
    Decisions,
    /// Every evaluated node, with its input.
    Full,
}

/// Bounds on what an audit captures, so that large inputs produce bounded audits.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureLimits {
    /// Items kept from an array (and fields from an object) in captured values.
    pub max_items: usize,
    /// Characters kept from a string in captured values.
    pub max_string: usize,
    /// Nesting depth kept in captured values.
    pub max_depth: usize,
    /// Rejected items kept as samples per filter.
    pub max_rejected: usize,
    /// Nodes recorded in the tree; further nodes are dropped and `truncated` is set.
    pub max_nodes: usize,
    /// Characters of source text kept per node.
    pub max_source: usize,
}

impl Default for CaptureLimits {
    fn default() -> Self {
        CaptureLimits {
            max_items: 10,
            max_string: 200,
            max_depth: 4,
            max_rejected: 5,
            max_nodes: 10_000,
            max_source: 200,
        }
    }
}

/// Settings for [`crate::Expression::evaluate_audited`].
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct AuditOptions {
    /// What to record.
    pub level: AuditLevel,
    /// Bounds on captured values and tree size.
    pub limits: CaptureLimits,
}

impl AuditOptions {
    /// Records decisions only (the default).
    pub fn decisions() -> Self {
        AuditOptions::default()
    }

    /// Records every evaluated node.
    pub fn full() -> Self {
        AuditOptions {
            level: AuditLevel::Full,
            ..AuditOptions::default()
        }
    }
}

/// The audit of one evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditTree {
    /// What was evaluated, when, and how it ended.
    pub header: AuditHeader,
    /// The root of the recorded tree (absent with [`AuditLevel::Off`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<AuditNode>,
    /// Number of recorded nodes.
    pub node_count: usize,
    /// Whether nodes were dropped because of [`CaptureLimits::max_nodes`].
    pub truncated: bool,
}

/// The header of an [`AuditTree`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditHeader {
    /// `jlens-core` and its version.
    pub engine: String,
    /// FNV-1a 64-bit hash of the expression source, in hexadecimal.
    pub expression_hash: String,
    /// FNV-1a 64-bit hash of the input document's JSON text, in hexadecimal.
    pub input_hash: String,
    /// The evaluation timestamp (`$now()`), in ISO 8601.
    pub timestamp: String,
    /// Wall-clock duration of the evaluation, in milliseconds.
    pub duration_ms: f64,
    /// The level the audit was recorded at.
    pub level: AuditLevel,
    /// The result of the evaluation (absent when undefined or on error).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<JsonValue>,
    /// The error, if the evaluation failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AuditError>,
}

/// An error recorded in an audit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditError {
    /// The JSONata error code.
    pub code: String,
    /// The message.
    pub message: String,
}

impl From<&Error> for AuditError {
    fn from(e: &Error) -> Self {
        AuditError {
            code: e.code().to_owned(),
            message: e.message(),
        }
    }
}

/// The result kind of an audited node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutcomeKind {
    /// It produced `true`.
    True,
    /// It produced `false`.
    False,
    /// It produced another value.
    Value,
    /// It produced undefined.
    Undefined,
    /// It raised an error.
    Error,
}

/// One evaluation of one node.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditNode {
    /// The syntax-tree node (`Node::id`).
    pub node: NodeId,
    /// The node type (`binary`, `path`, `function`, ...).
    pub kind: String,
    /// The operator, function or variable name, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    /// Byte range of the node in the expression source.
    pub span: Span,
    /// The node's source text (possibly shortened).
    pub source: String,
    /// `@label` of the node's annotation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// `@message` of the node's annotation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// `@id` of the node's annotation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_id: Option<String>,
    /// The context value (recorded with [`AuditLevel::Full`] only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<JsonValue>,
    /// The value produced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<JsonValue>,
    /// How the evaluation ended.
    pub outcome: OutcomeKind,
    /// The error, when `outcome` is `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AuditError>,
    /// The compared values, for comparisons.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operands: Option<[Option<JsonValue>; 2]>,
    /// For `and`/`or`: the right side was not evaluated.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub short_circuited: bool,
    /// For conditionals: the branch taken.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<Branch>,
    /// For `$name := value`: the variable bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<String>,
    /// Predicate filters applied while evaluating this node.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub filters: Vec<FilterAudit>,
    /// Recorded evaluations nested inside this one.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub children: Vec<AuditNode>,
}

impl AuditNode {
    /// Whether the node evaluated to `true` (`Some(true)`), `false` (`Some(false)`), or
    /// something else (`None`).
    pub fn passed(&self) -> Option<bool> {
        match self.outcome {
            OutcomeKind::True => Some(true),
            OutcomeKind::False => Some(false),
            _ => None,
        }
    }
}

/// Statistics of a predicate filter (`items[price > 10]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterAudit {
    /// The predicate node.
    pub node: NodeId,
    /// The predicate's source text.
    pub source: String,
    /// `@label` of the predicate's annotation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Items tested.
    pub tested: usize,
    /// Items that passed.
    pub passed: usize,
    /// A sample of the items that did not pass.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub rejected: Vec<JsonValue>,
}

/// Captures a value as JSON within `limits`. Truncated arrays end with a `"… N more"` string,
/// truncated objects with a `"…": "N more"` field, and strings with `…`.
fn capture(v: &Value, limits: &CaptureLimits, depth: usize) -> Option<JsonValue> {
    Some(match v {
        Value::Undefined => return None,
        Value::Function(_) => JsonValue::String("<function>".into()),
        Value::String(s) => {
            let count = s.chars().count();
            if count > limits.max_string {
                let mut t: String = s.chars().take(limits.max_string).collect();
                t.push('…');
                JsonValue::String(t)
            } else {
                JsonValue::String(s.to_string())
            }
        }
        Value::Array(_) | Value::Object(_) if depth >= limits.max_depth => {
            JsonValue::String("…".into())
        }
        Value::Array(a) => {
            let mut items: Vec<JsonValue> = a
                .items
                .iter()
                .take(limits.max_items)
                .map(|i| capture(i, limits, depth + 1).unwrap_or(JsonValue::Null))
                .collect();
            if a.items.len() > limits.max_items {
                items.push(JsonValue::String(format!(
                    "… {} more",
                    a.items.len() - limits.max_items
                )));
            }
            JsonValue::Array(items)
        }
        Value::Object(o) => {
            let o = o.borrow();
            let mut map = serde_json::Map::new();
            for (k, v) in o.iter().take(limits.max_items) {
                if let Some(j) = capture(v, limits, depth + 1) {
                    map.insert(k.clone(), j);
                }
            }
            if o.len() > limits.max_items {
                map.insert(
                    "…".into(),
                    JsonValue::String(format!("{} more", o.len() - limits.max_items)),
                );
            }
            JsonValue::Object(map)
        }
        other => other.to_json()?,
    })
}

/// FNV-1a, 64 bits, as 16 hexadecimal digits.
pub(crate) fn fnv1a(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// A short description of a node for audits: its kind and operator/name.
pub(crate) fn describe(node: &Node) -> (&'static str, Option<String>) {
    use crate::ast::NodeKind;
    let op = match &node.kind {
        NodeKind::Binary { op, .. } => Some(op.symbol().to_owned()),
        NodeKind::Function { procedure, .. } | NodeKind::Partial { procedure, .. } => {
            match &procedure.kind {
                NodeKind::Variable { name } => Some(format!("${name}")),
                NodeKind::Path { steps, .. } => match &steps[0].kind {
                    NodeKind::Variable { name } => Some(format!("${name}")),
                    NodeKind::Name { name } => Some(name.clone()),
                    _ => None,
                },
                _ => None,
            }
        }
        NodeKind::Variable { name } => Some(format!("${name}")),
        NodeKind::Name { name } => Some(name.clone()),
        NodeKind::Bind { lhs, .. } => match &lhs.kind {
            NodeKind::Variable { name } => Some(format!("${name}")),
            _ => None,
        },
        _ => None,
    };
    (node.type_name(), op)
}

/// The source text of `span`, whitespace-normalized and shortened to `max` characters.
pub(crate) fn snippet(source: &str, span: Span, max: usize) -> String {
    let text = source.get(span.start..span.end).unwrap_or_default();
    // Drop comments (annotations show up as labels instead).
    let mut without_comments = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("/*") {
        without_comments.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => rest = "",
        }
    }
    without_comments.push_str(rest);
    let collapsed = without_comments
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.chars().count() > max {
        let mut s: String = collapsed.chars().take(max).collect();
        s.push('…');
        s
    } else {
        collapsed
    }
}
