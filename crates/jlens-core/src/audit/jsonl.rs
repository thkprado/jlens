//! An event log in JSON Lines format.

use std::io::Write;

use serde_json::{Value as JsonValue, json};

use super::{Branch, CaptureLimits, EvalInfo, Outcome, Tracer, ValueRef};
use crate::ast::Node;

/// A [`Tracer`] that writes one JSON object per event to a writer.
///
/// Each line has an `event` field (`begin`, `enter`, `exit`, `operands`, `shortCircuit`,
/// `branch`, `bind`, `filter`, `end`) and the node id and span where relevant. Values are
/// captured within [`CaptureLimits`]. Write errors stop the log; check [`JsonlTracer::error`].
pub struct JsonlTracer<W: Write> {
    out: W,
    limits: CaptureLimits,
    error: Option<std::io::Error>,
}

impl<W: Write> JsonlTracer<W> {
    /// A tracer writing to `out`.
    pub fn new(out: W, limits: CaptureLimits) -> Self {
        JsonlTracer {
            out,
            limits,
            error: None,
        }
    }

    /// The first write error, if any.
    pub fn error(&self) -> Option<&std::io::Error> {
        self.error.as_ref()
    }

    /// Returns the writer.
    pub fn into_inner(self) -> W {
        self.out
    }

    fn write(&mut self, line: JsonValue) {
        if self.error.is_some() {
            return;
        }
        if let Err(e) = writeln!(self.out, "{line}") {
            self.error = Some(e);
        }
    }

    fn node(node: &Node) -> JsonValue {
        json!({"node": node.id, "kind": node.type_name(), "span": [node.span.start, node.span.end]})
    }
}

fn outcome_json(outcome: Outcome<'_>, limits: &CaptureLimits) -> JsonValue {
    match outcome {
        Outcome::Value(v) => json!({"value": v.capture(limits)}),
        Outcome::Error(e) => json!({"error": {"code": e.code(), "message": e.message()}}),
    }
}

impl<W: Write> Tracer for JsonlTracer<W> {
    fn begin(&mut self, info: &EvalInfo) {
        self.write(json!({"event": "begin", "timestamp": info.timestamp as i64}));
    }

    fn enter(&mut self, node: &Node, _input: ValueRef<'_>) {
        let mut line = Self::node(node);
        line["event"] = "enter".into();
        self.write(line);
    }

    fn exit(&mut self, node: &Node, outcome: Outcome<'_>) {
        let mut line = Self::node(node);
        line["event"] = "exit".into();
        line["outcome"] = outcome_json(outcome, &self.limits);
        self.write(line);
    }

    fn operands(&mut self, node: &Node, lhs: ValueRef<'_>, rhs: ValueRef<'_>) {
        let mut line = Self::node(node);
        line["event"] = "operands".into();
        line["lhs"] = lhs.capture(&self.limits).into();
        line["rhs"] = rhs.capture(&self.limits).into();
        self.write(line);
    }

    fn short_circuit(&mut self, node: &Node) {
        let mut line = Self::node(node);
        line["event"] = "shortCircuit".into();
        self.write(line);
    }

    fn branch(&mut self, node: &Node, taken: Branch) {
        let mut line = Self::node(node);
        line["event"] = "branch".into();
        line["taken"] = serde_json::to_value(taken).unwrap_or_default();
        self.write(line);
    }

    fn bind(&mut self, node: &Node, name: &str, value: ValueRef<'_>) {
        let mut line = Self::node(node);
        line["event"] = "bind".into();
        line["name"] = name.into();
        line["value"] = value.capture(&self.limits).into();
        self.write(line);
    }

    fn filter_item(&mut self, predicate: &Node, index: usize, _item: ValueRef<'_>, passed: bool) {
        let mut line = Self::node(predicate);
        line["event"] = "filter".into();
        line["index"] = index.into();
        line["passed"] = passed.into();
        self.write(line);
    }

    fn end(&mut self, outcome: Outcome<'_>) {
        let mut line = json!({"event": "end"});
        line["outcome"] = outcome_json(outcome, &self.limits);
        self.write(line);
    }
}
