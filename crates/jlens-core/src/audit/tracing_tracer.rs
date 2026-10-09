//! Forwards evaluation events to the `tracing` crate (feature `tracing`).

use super::{Branch, CaptureLimits, Outcome, Tracer, ValueRef};
use crate::ast::Node;

/// A [`Tracer`] that emits `tracing` events under the target `jlens::eval`.
///
/// Node entries and exits are `TRACE` events; decisions (comparisons, short-circuits, branches,
/// bindings) are `DEBUG` events. Install any `tracing` subscriber to collect them.
#[derive(Debug, Default)]
pub struct TracingTracer {
    limits: CaptureLimits,
}

impl TracingTracer {
    /// A tracer capturing values within `limits`.
    pub fn new(limits: CaptureLimits) -> Self {
        TracingTracer { limits }
    }
}

impl Tracer for TracingTracer {
    fn enter(&mut self, node: &Node, _input: ValueRef<'_>) {
        tracing::trace!(target: "jlens::eval", node = node.id, kind = node.type_name(), start = node.span.start, end = node.span.end, "enter");
    }

    fn exit(&mut self, node: &Node, outcome: Outcome<'_>) {
        match outcome {
            Outcome::Value(v) => {
                let value = v
                    .capture(&self.limits)
                    .map(|j| j.to_string())
                    .unwrap_or_default();
                tracing::trace!(target: "jlens::eval", node = node.id, kind = node.type_name(), value = %value, "exit");
            }
            Outcome::Error(e) => {
                tracing::trace!(target: "jlens::eval", node = node.id, code = e.code(), "error");
            }
        }
    }

    fn operands(&mut self, node: &Node, lhs: ValueRef<'_>, rhs: ValueRef<'_>) {
        let l = lhs
            .capture(&self.limits)
            .map(|j| j.to_string())
            .unwrap_or_default();
        let r = rhs
            .capture(&self.limits)
            .map(|j| j.to_string())
            .unwrap_or_default();
        tracing::debug!(target: "jlens::eval", node = node.id, lhs = %l, rhs = %r, "compare");
    }

    fn short_circuit(&mut self, node: &Node) {
        tracing::debug!(target: "jlens::eval", node = node.id, "short-circuit");
    }

    fn branch(&mut self, node: &Node, taken: Branch) {
        tracing::debug!(target: "jlens::eval", node = node.id, taken = ?taken, "branch");
    }

    fn bind(&mut self, node: &Node, name: &str, value: ValueRef<'_>) {
        let v = value
            .capture(&self.limits)
            .map(|j| j.to_string())
            .unwrap_or_default();
        tracing::debug!(target: "jlens::eval", node = node.id, name, value = %v, "bind");
    }
}
