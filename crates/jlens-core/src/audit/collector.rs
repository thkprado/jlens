//! Builds an [`AuditTree`] from evaluation events.

use web_time::Instant;

use super::{
    AuditError, AuditHeader, AuditLevel, AuditNode, AuditOptions, AuditTree, Branch, EvalInfo,
    FilterAudit, Outcome, OutcomeKind, Tracer, ValueRef, describe, fnv1a, snippet,
};
use crate::ast::{BinaryOp, Node, NodeKind};

/// A [`Tracer`] that records an [`AuditTree`].
///
/// Usually created by [`crate::Expression::evaluate_audited`]; use it directly to combine
/// auditing with your own tracer.
pub struct AuditCollector {
    options: AuditOptions,
    source: String,
    input_hash: String,
    timestamp: f64,
    started: Option<Instant>,
    stack: Vec<Frame>,
    root: Option<AuditNode>,
    node_count: usize,
    truncated: bool,
    /// Depth of nested predicate filters; in `Decisions` mode nothing is recorded inside them,
    /// only the filter statistics.
    filter_depth: usize,
    result: Option<serde_json::Value>,
    error: Option<AuditError>,
}

struct Frame {
    node: AuditNode,
    suppressed: bool,
    decision: bool,
}

impl AuditCollector {
    /// A collector for an evaluation of `input` (used for the header's input hash).
    pub fn new(options: AuditOptions, input: Option<&serde_json::Value>) -> Self {
        let input_text = input.map(|i| i.to_string()).unwrap_or_default();
        AuditCollector {
            options,
            source: String::new(),
            input_hash: fnv1a(&input_text),
            timestamp: 0.0,
            started: None,
            stack: Vec::new(),
            root: None,
            node_count: 0,
            truncated: false,
            filter_depth: 0,
            result: None,
            error: None,
        }
    }

    /// The finished audit.
    pub fn finish(self) -> AuditTree {
        AuditTree {
            header: AuditHeader {
                engine: concat!("jlens-core ", env!("CARGO_PKG_VERSION")).into(),
                expression_hash: fnv1a(&self.source),
                input_hash: self.input_hash,
                timestamp: crate::datetime::from_millis(self.timestamp, None, None, usize::MAX)
                    .unwrap_or_default(),
                duration_ms: self
                    .started
                    .map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0),
                level: self.options.level,
                result: self.result,
                error: self.error,
            },
            root: self.root,
            node_count: self.node_count,
            truncated: self.truncated,
        }
    }

    fn level(&self) -> AuditLevel {
        self.options.level
    }

    fn top(&mut self) -> Option<&mut AuditNode> {
        self.stack.last_mut().map(|f| &mut f.node)
    }

    /// Adds a finished node to its parent, or makes it the root.
    fn attach(&mut self, node: AuditNode) {
        match self.stack.last_mut() {
            Some(parent) => parent.node.children.push(node),
            None => self.root = Some(node),
        }
    }
}

/// Nodes that decide: comparisons, `and`/`or`, conditionals and bindings.
fn is_decision(node: &Node) -> bool {
    match &node.kind {
        NodeKind::Binary { op, .. } => matches!(
            op,
            BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge
                | BinaryOp::In
                | BinaryOp::And
                | BinaryOp::Or
        ),
        NodeKind::Condition { .. } | NodeKind::Fallback { .. } | NodeKind::Bind { .. } => true,
        _ => false,
    }
}

impl Tracer for AuditCollector {
    fn begin(&mut self, info: &EvalInfo) {
        self.source = info.source.to_owned();
        self.timestamp = info.timestamp;
        self.started = Some(Instant::now());
    }

    fn enter(&mut self, node: &Node, input: ValueRef<'_>) {
        if self.level() == AuditLevel::Off {
            return;
        }
        let suppressed = self.filter_depth > 0 && self.level() == AuditLevel::Decisions;
        let (kind, op) = describe(node);
        let annotation = node.annotation.as_deref();
        let limits = &self.options.limits;
        self.stack.push(Frame {
            node: AuditNode {
                node: node.id,
                kind: kind.to_owned(),
                op,
                span: node.span,
                source: if suppressed {
                    String::new()
                } else {
                    snippet(&self.source, node.span, limits.max_source)
                },
                label: annotation.and_then(|a| a.label.clone()),
                message: annotation.and_then(|a| a.message.clone()),
                check_id: annotation.and_then(|a| a.id.clone()),
                input: (self.level() == AuditLevel::Full)
                    .then(|| input.capture(limits))
                    .flatten(),
                output: None,
                outcome: OutcomeKind::Undefined,
                error: None,
                operands: None,
                short_circuited: false,
                branch: None,
                binding: None,
                filters: Vec::new(),
                children: Vec::new(),
            },
            suppressed,
            decision: is_decision(node) || annotation.is_some(),
        });
    }

    fn exit(&mut self, _node: &Node, outcome: Outcome<'_>) {
        let Some(mut frame) = self.stack.pop() else {
            return;
        };
        let limits = self.options.limits;
        let is_bool = match outcome {
            Outcome::Value(v) => {
                frame.node.output = v.capture(&limits);
                frame.node.outcome = match v.as_bool() {
                    Some(true) => OutcomeKind::True,
                    Some(false) => OutcomeKind::False,
                    None if v.is_undefined() => OutcomeKind::Undefined,
                    None => OutcomeKind::Value,
                };
                v.as_bool().is_some()
            }
            Outcome::Error(e) => {
                frame.node.outcome = OutcomeKind::Error;
                frame.node.error = Some(e.into());
                false
            }
        };
        if frame.suppressed {
            return;
        }
        let is_root = self.stack.is_empty();
        let record = match self.level() {
            AuditLevel::Off => false,
            AuditLevel::Full => true,
            AuditLevel::Decisions => {
                is_root
                    || frame.decision
                    || is_bool
                    || frame.node.outcome == OutcomeKind::Error
                    || !frame.node.filters.is_empty()
            }
        };
        // Keep one slot for the root, which is always recorded.
        if record && !is_root && self.node_count + 1 >= limits.max_nodes {
            self.truncated = true;
            return;
        }
        if record {
            self.node_count += 1;
            self.attach(frame.node);
        } else if let Some(parent) = self.stack.last_mut() {
            // Collapse: the parent adopts this node's recorded descendants and filters.
            parent.node.children.append(&mut frame.node.children);
            parent.node.filters.append(&mut frame.node.filters);
        }
    }

    fn operands(&mut self, _node: &Node, lhs: ValueRef<'_>, rhs: ValueRef<'_>) {
        let limits = self.options.limits;
        if let Some(top) = self.top() {
            top.operands = Some([lhs.capture(&limits), rhs.capture(&limits)]);
        }
    }

    fn short_circuit(&mut self, _node: &Node) {
        if let Some(top) = self.top() {
            top.short_circuited = true;
        }
    }

    fn branch(&mut self, _node: &Node, taken: Branch) {
        if let Some(top) = self.top() {
            top.branch = Some(taken);
        }
    }

    fn bind(&mut self, _node: &Node, name: &str, _value: ValueRef<'_>) {
        if let Some(top) = self.top() {
            top.binding = Some(format!("${name}"));
        }
    }

    fn filter_begin(&mut self, predicate: &Node) {
        self.filter_depth += 1;
        if self.level() == AuditLevel::Off || self.filter_depth > 1 {
            return;
        }
        let source = snippet(&self.source, predicate.span, self.options.limits.max_source);
        let label = predicate
            .annotation
            .as_deref()
            .and_then(|a| a.label.clone());
        if let Some(top) = self.top()
            && !top.filters.iter().any(|f| f.node == predicate.id)
        {
            top.filters.push(FilterAudit {
                node: predicate.id,
                source,
                label,
                tested: 0,
                passed: 0,
                rejected: Vec::new(),
            });
        }
    }

    fn filter_item(&mut self, predicate: &Node, _index: usize, item: ValueRef<'_>, passed: bool) {
        if self.filter_depth != 1 {
            return;
        }
        let limits = self.options.limits;
        if let Some(top) = self.top()
            && let Some(f) = top.filters.iter_mut().find(|f| f.node == predicate.id)
        {
            f.tested += 1;
            if passed {
                f.passed += 1;
            } else if f.rejected.len() < limits.max_rejected
                && let Some(v) = item.capture(&limits)
            {
                f.rejected.push(v);
            }
        }
    }

    fn filter_end(&mut self, _predicate: &Node) {
        self.filter_depth = self.filter_depth.saturating_sub(1);
    }

    fn end(&mut self, outcome: Outcome<'_>) {
        match outcome {
            Outcome::Value(v) => self.result = v.to_json(),
            Outcome::Error(e) => self.error = Some(e.into()),
        }
    }
}
