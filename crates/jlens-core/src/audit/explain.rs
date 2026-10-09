//! Human-oriented summaries of audit trees.

use std::fmt;

use serde::Serialize;
use serde_json::Value as JsonValue;

use super::{AuditNode, AuditTree, Branch, OutcomeKind};
use crate::ast::NodeId;

/// Why an evaluation produced its result, derived from an [`AuditTree`].
///
/// The explanation follows the decisions that mattered:
/// - for a false `and`, the operands that failed;
/// - for a true `or`, the operands that held;
/// - for a conditional, the condition and the branch taken;
/// - for filters, how many items passed and a sample of those rejected.
#[derive(Debug, Clone, Serialize)]
pub struct Explanation {
    /// The result of the evaluation, or the error.
    pub result: String,
    /// The lines of the explanation, in order, with their nesting depth.
    pub lines: Vec<ExplanationLine>,
}

/// One line of an [`Explanation`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplanationLine {
    /// Nesting depth (0 for top-level reasons).
    pub depth: usize,
    /// `Some(true)` for a check that held, `Some(false)` for one that failed.
    pub passed: Option<bool>,
    /// The text of the line.
    pub text: String,
    /// The annotation `@message`, shown for failed checks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The syntax-tree node the line is about.
    pub node: NodeId,
}

/// Explains an audit.
pub fn explain(tree: &AuditTree) -> Explanation {
    let result = match (&tree.header.error, &tree.header.result) {
        (Some(e), _) => format!("error {}: {}", e.code, e.message),
        (None, Some(v)) => v.to_string(),
        (None, None) => "undefined".into(),
    };
    let mut lines = Vec::new();
    if let Some(root) = &tree.root {
        explain_node(root, 0, &mut lines);
    }
    Explanation { result, lines }
}

/// Whether a node is shown as a line of its own: labelled checks, comparisons, variable
/// bindings, conditionals, errors and calls returning a boolean. Unlabelled `and`/`or` chains
/// and blocks are flattened into their operands.
fn is_check(n: &AuditNode) -> bool {
    n.label.is_some()
        || n.operands.is_some()
        || n.binding.is_some()
        || n.branch.is_some()
        || n.outcome == OutcomeKind::Error
        || (n.kind == "function" && n.passed().is_some())
}

fn value_text(v: &Option<JsonValue>) -> String {
    v.as_ref()
        .map_or_else(|| "undefined".into(), JsonValue::to_string)
}

fn describe(n: &AuditNode) -> String {
    let mut text = String::new();
    if let Some(label) = &n.label {
        text.push_str(label);
        text.push_str(" — ");
    }
    text.push_str(&n.source);
    match n.outcome {
        OutcomeKind::Error => {
            if let Some(e) = &n.error {
                text.push_str(&format!(" → error {}: {}", e.code, e.message));
            }
        }
        _ if n.binding.is_some() => text.push_str(&format!(" = {}", value_text(&n.output))),
        _ => text.push_str(&format!(" → {}", value_text(&n.output))),
    }
    if let (Some([l, r]), Some(op)) = (&n.operands, &n.op) {
        text.push_str(&format!(" ({} {op} {})", value_text(l), value_text(r)));
    }
    if n.short_circuited {
        text.push_str(" (right side not evaluated)");
    }
    if let Some(branch) = n.branch {
        text.push_str(match branch {
            Branch::Then => " [then]",
            Branch::Else => " [else]",
            Branch::None => " [no branch]",
        });
    }
    text
}

fn explain_node(n: &AuditNode, depth: usize, out: &mut Vec<ExplanationLine>) {
    let shown = is_check(n);
    let child_depth = if shown { depth + 1 } else { depth };
    if shown {
        out.push(ExplanationLine {
            depth,
            passed: n.passed(),
            text: describe(n),
            message: n.message.clone().filter(|_| n.passed() == Some(false)),
            node: n.node,
        });
    }
    let op = n.op.as_deref();
    let relevant: Vec<&AuditNode> = match (op, n.passed()) {
        (Some("and"), Some(false)) => n
            .children
            .iter()
            .filter(|c| c.passed() == Some(false))
            .collect(),
        (Some("or"), Some(true)) => n
            .children
            .iter()
            .filter(|c| c.passed() == Some(true))
            .collect(),
        _ => n.children.iter().collect(),
    };
    for child in relevant {
        explain_node(child, child_depth, out);
    }
    filters(n, child_depth, out);
}

fn filters(n: &AuditNode, depth: usize, out: &mut Vec<ExplanationLine>) {
    for f in &n.filters {
        let mut text = String::new();
        if let Some(label) = &f.label {
            text.push_str(label);
            text.push_str(" — ");
        }
        text.push_str(&format!(
            "[{}]: {} of {} items passed",
            f.source, f.passed, f.tested
        ));
        if !f.rejected.is_empty() {
            let sample: Vec<String> = f.rejected.iter().map(JsonValue::to_string).collect();
            text.push_str(&format!("; rejected: {}", sample.join(", ")));
        }
        out.push(ExplanationLine {
            depth,
            passed: None,
            text,
            message: None,
            node: f.node,
        });
    }
}

impl fmt::Display for Explanation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "result: {}", self.result)?;
        for line in &self.lines {
            let mark = match line.passed {
                Some(true) => "✔",
                Some(false) => "✘",
                None => "•",
            };
            writeln!(f, "{}{mark} {}", "  ".repeat(line.depth), line.text)?;
            if let Some(message) = &line.message {
                writeln!(f, "{}    {message}", "  ".repeat(line.depth))?;
            }
        }
        Ok(())
    }
}
