//! The processed abstract syntax tree that the evaluator walks.
//!
//! The shape follows the output of `processAST` in jsonata-js (`src/parser.js`, MIT):
//! `.` chains become [`NodeKind::Path`] nodes with a list of steps, predicates become
//! [`Stage`]s, and so on. Unlike the reference implementation, every node is typed, carries a
//! byte [`Span`] covering its whole source text, and has a stable [`NodeId`].

use std::sync::Arc;

use serde::Serialize;

use crate::annotation::Annotation;
use crate::error::{Diagnostic, Error, Severity};
use crate::signature::Signature;

/// A byte range in the expression source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, serde::Deserialize)]
pub struct Span {
    /// Offset of the first byte.
    pub start: usize,
    /// Offset one past the last byte.
    pub end: usize,
}

impl Span {
    /// Creates a span.
    pub const fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }

    /// The smallest span covering both spans.
    pub fn to(self, other: Span) -> Span {
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }

    /// Whether `offset` lies within the span (the end is inclusive, which suits cursors).
    pub fn contains(self, offset: usize) -> bool {
        self.start <= offset && offset <= self.end
    }
}

/// Identifier of a node, unique within one [`Ast`] (assigned in pre-order).
pub type NodeId = u32;

/// Index of an ancestor slot in [`Ast::slots`], used by the parent operator `%`.
pub type SlotId = usize;

/// A parsed and processed JSONata expression.
#[derive(Debug, Clone, Serialize)]
pub struct Ast {
    /// The root node.
    pub root: Node,
    /// Ancestor slots created for parent operators (`%`). Nodes refer to them by index.
    pub slots: Vec<Slot>,
    /// Problems found while parsing. Strict parsing only reports warnings here; recovering
    /// parsing also reports the syntax errors it recovered from.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
    /// The source text.
    #[serde(skip)]
    pub source: String,
}

impl Ast {
    /// Whether any diagnostic is an error (only possible with recovering parsing).
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    /// The label of an ancestor slot.
    pub fn slot_label(&self, slot: SlotId) -> &str {
        &self.slots[slot].label
    }
}

/// An ancestor slot: the binding through which `%` reaches a parent value.
#[derive(Debug, Clone, Serialize)]
pub struct Slot {
    /// The variable name under which the ancestor is bound in the tuple stream (`!0`, `!1`, …).
    pub label: String,
    #[serde(skip)]
    pub(crate) level: i32,
}

/// A node of the processed syntax tree.
#[derive(Debug, Clone, Serialize)]
pub struct Node {
    /// Unique identifier within the tree.
    pub id: NodeId,
    /// What kind of node this is.
    #[serde(flatten)]
    pub kind: NodeKind,
    /// Byte range of the whole expression this node represents.
    pub span: Span,
    /// Byte offset just after the node's main token (operator or name). Errors raised for this
    /// node report this point, matching jsonata-js positions.
    #[serde(skip)]
    pub pos: usize,
    /// The same point as `pos`, as a UTF-16 offset: the `position` reported by errors.
    #[serde(skip)]
    pub position: usize,
    /// Step and sequence attributes set by `processAST`.
    #[serde(flatten)]
    pub attrs: Attrs,
    /// Audit annotation attached through a `/** ... */` comment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation: Option<Box<Annotation>>,
}

/// The kinds of nodes.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum NodeKind {
    /// A location path: a sequence of steps joined by `.`.
    Path {
        /// The steps, evaluated left to right.
        steps: Vec<Node>,
        /// Whether any step used the `[]` modifier.
        #[serde(skip_serializing_if = "is_false")]
        keep_singleton_array: bool,
    },
    /// A binary operator.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// Left operand.
        lhs: Box<Node>,
        /// Right operand.
        rhs: Box<Node>,
    },
    /// Numeric negation (`-x`).
    Negate {
        /// The operand.
        expr: Box<Node>,
    },
    /// Array constructor (`[a, b, 1..3]`).
    Array {
        /// The items.
        items: Vec<Node>,
    },
    /// Object constructor (`{"key": value}`).
    Object {
        /// Key/value expression pairs.
        pairs: Vec<(Node, Node)>,
    },
    /// A field name.
    Name {
        /// The field name.
        name: String,
    },
    /// A variable reference (`$name`, `$` for the context, `$$` for the root).
    Variable {
        /// The name without the leading `$`.
        name: String,
    },
    /// A string literal.
    String {
        /// The value.
        value: String,
    },
    /// A number literal.
    Number {
        /// The value.
        value: f64,
    },
    /// `true` or `false`.
    Bool {
        /// The value.
        value: bool,
    },
    /// `null`.
    Null,
    /// A regular expression literal (`/pattern/flags`).
    Regex {
        /// The pattern source.
        pattern: String,
        /// Flags (`i`, `m`).
        flags: String,
    },
    /// `*`: all fields of the context object.
    Wildcard,
    /// `**`: all descendants.
    Descendant,
    /// `%`: the parent of the context.
    Parent {
        /// The ancestor slot this operator reads.
        slot: SlotId,
        /// The variable under which the ancestor is bound (the slot's final label).
        label: String,
    },
    /// `cond ? then : else`.
    Condition {
        /// The condition.
        condition: Box<Node>,
        /// Evaluated when the condition is truthy.
        then: Box<Node>,
        /// Evaluated otherwise.
        #[serde(rename = "else", skip_serializing_if = "Option::is_none")]
        otherwise: Option<Box<Node>>,
    },
    /// `lhs ?: rhs` and `lhs ?? rhs`: the value of `lhs` when it is true as a Boolean (`?:`) or
    /// exists (`??`), else the value of `rhs`. `lhs` is evaluated once.
    Fallback {
        /// The operator.
        op: FallbackOp,
        /// The value, kept if it passes the test.
        lhs: Box<Node>,
        /// The fallback, evaluated when the value does not pass the test.
        rhs: Box<Node>,
    },
    /// `(e1; e2; ...)`.
    Block {
        /// The expressions, evaluated in order.
        exprs: Vec<Node>,
        /// The block is evaluated in a frame of its own, for the variables its expressions may
        /// bind. Set by post-processing; a block that binds none shares its enclosing frame.
        #[serde(skip)]
        frame: bool,
    },
    /// `$var := value`.
    Bind {
        /// The variable being bound.
        lhs: Box<Node>,
        /// The value.
        rhs: Box<Node>,
    },
    /// `value ~> $function`.
    Apply {
        /// The value (or function) on the left.
        lhs: Box<Node>,
        /// The function on the right.
        rhs: Box<Node>,
    },
    /// A function call.
    Function {
        /// The function being called.
        procedure: Box<Node>,
        /// The arguments.
        args: Vec<Node>,
    },
    /// A partial application (some arguments are `?`).
    Partial {
        /// The function being applied.
        procedure: Box<Node>,
        /// The arguments, with [`NodeKind::Placeholder`] for the missing ones.
        args: Vec<Node>,
    },
    /// The `?` placeholder in a partial application.
    Placeholder,
    /// A function definition, or a thunk created for tail-call optimisation.
    ///
    /// The definition is shared (`Arc`) so that function values created at run time can keep
    /// their body alive independently of the tree.
    Lambda(Arc<LambdaDef>),
    /// `| pattern | update [, delete] |`, shared like [`NodeKind::Lambda`].
    Transform(Arc<TransformDef>),
    /// An order-by step (`^(...)`), only found inside a path.
    Sort {
        /// The sort keys.
        terms: Vec<SortTerm>,
    },
    /// A syntax error that the recovering parser replaced with this node.
    Error {
        /// The error.
        error: Box<Error>,
    },
}

/// A function definition.
#[derive(Debug, Clone, Serialize)]
pub struct LambdaDef {
    /// Parameter names, without the leading `$`.
    pub params: Vec<String>,
    /// The optional signature (`<n-n:n>`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
    /// The body.
    pub body: Node,
    /// Whether this lambda is a tail-call thunk rather than a user-written function.
    #[serde(skip_serializing_if = "is_false")]
    pub thunk: bool,
}

/// A transform expression (`| pattern | update [, delete] |`).
#[derive(Debug, Clone, Serialize)]
pub struct TransformDef {
    /// Selects the objects to modify.
    pub pattern: Node,
    /// The object merged into each match.
    pub update: Node,
    /// Field names to remove.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete: Option<Node>,
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BinaryOp {
    /// `+`
    #[serde(rename = "+")]
    Add,
    /// `-`
    #[serde(rename = "-")]
    Sub,
    /// `*`
    #[serde(rename = "*")]
    Mul,
    /// `/`
    #[serde(rename = "/")]
    Div,
    /// `%`
    #[serde(rename = "%")]
    Mod,
    /// `=`
    #[serde(rename = "=")]
    Eq,
    /// `!=`
    #[serde(rename = "!=")]
    Ne,
    /// `<`
    #[serde(rename = "<")]
    Lt,
    /// `<=`
    #[serde(rename = "<=")]
    Le,
    /// `>`
    #[serde(rename = ">")]
    Gt,
    /// `>=`
    #[serde(rename = ">=")]
    Ge,
    /// `&` (string concatenation)
    #[serde(rename = "&")]
    Concat,
    /// `and`
    #[serde(rename = "and")]
    And,
    /// `or`
    #[serde(rename = "or")]
    Or,
    /// `in`
    #[serde(rename = "in")]
    In,
    /// `..` (range, inside array constructors)
    #[serde(rename = "..")]
    Range,
}

impl BinaryOp {
    pub(crate) fn from_symbol(symbol: &str) -> Option<Self> {
        Some(match symbol {
            "+" => BinaryOp::Add,
            "-" => BinaryOp::Sub,
            "*" => BinaryOp::Mul,
            "/" => BinaryOp::Div,
            "%" => BinaryOp::Mod,
            "=" => BinaryOp::Eq,
            "!=" => BinaryOp::Ne,
            "<" => BinaryOp::Lt,
            "<=" => BinaryOp::Le,
            ">" => BinaryOp::Gt,
            ">=" => BinaryOp::Ge,
            "&" => BinaryOp::Concat,
            "and" => BinaryOp::And,
            "or" => BinaryOp::Or,
            "in" => BinaryOp::In,
            ".." => BinaryOp::Range,
            _ => return None,
        })
    }

    /// The operator as written in source.
    pub fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Mod => "%",
            BinaryOp::Eq => "=",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::Concat => "&",
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
            BinaryOp::In => "in",
            BinaryOp::Range => "..",
        }
    }
}

/// The operators of [`NodeKind::Fallback`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FallbackOp {
    /// `?:`: keep the value when it is true as a Boolean.
    #[serde(rename = "?:")]
    Default,
    /// `??`: keep the value when it exists.
    #[serde(rename = "??")]
    Coalesce,
}

impl FallbackOp {
    /// The operator as written in source.
    pub fn symbol(self) -> &'static str {
        match self {
            FallbackOp::Default => "?:",
            FallbackOp::Coalesce => "??",
        }
    }
}

/// Attributes that `processAST` attaches to steps and other nodes.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attrs {
    /// The `[]` modifier: keep singleton results as arrays.
    #[serde(skip_serializing_if = "is_false")]
    pub keep_array: bool,
    /// An array constructor whose result must not be flattened into the path.
    #[serde(skip_serializing_if = "is_false")]
    pub cons_array: bool,
    /// The step is evaluated as a tuple stream (it binds `@`, `#` or ancestors).
    #[serde(skip_serializing_if = "is_false")]
    pub tuple: bool,
    /// Context variable bound with `@$var`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    /// Position variable bound with `#$var`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<String>,
    /// Filters and index bindings applied to each item of a path step.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<Stage>,
    /// Filters applied to the whole result of a non-path expression.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub predicate: Vec<Stage>,
    /// Group-by object constructor (`expr{key: value}`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Box<Group>>,
    /// Ancestor slot this step binds for a `%` further along.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ancestor: Option<SlotId>,
    /// The label under which this step binds its input for a `%` further along.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ancestor_label: Option<String>,
    /// Ancestor slots that still need to be resolved by an enclosing path.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub seeking_parent: Vec<SlotId>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A stage applied to each item of a step.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Stage {
    /// A predicate filter (`[expr]`): a boolean condition or a numeric index.
    Filter {
        /// The predicate.
        expr: Box<Node>,
        /// Byte span of the predicate including brackets.
        span: Span,
        /// The predicate is built from literals alone, so it gives the same value for every
        /// item, and is evaluated once rather than once per item.
        #[serde(skip)]
        constant: bool,
    },
    /// A position variable binding (`#$i`) that follows a filter.
    Index {
        /// Variable name without `$`.
        var: String,
        /// Byte span of the binding.
        span: Span,
    },
}

/// A group-by clause (`expr{key: value}`).
#[derive(Debug, Clone, Serialize)]
pub struct Group {
    /// Key/value expression pairs.
    pub pairs: Vec<(Node, Node)>,
    /// Byte span of the clause.
    pub span: Span,
}

/// A sort key in an order-by clause.
#[derive(Debug, Clone, Serialize)]
pub struct SortTerm {
    /// Sort in descending order (`>`).
    pub descending: bool,
    /// The key expression.
    pub expr: Node,
}

impl Node {
    pub(crate) fn new(kind: NodeKind, span: Span, pos: usize) -> Self {
        Node {
            id: 0,
            kind,
            span,
            pos,
            position: 0,
            attrs: Attrs::default(),
            annotation: None,
        }
    }

    pub(crate) fn path(steps: Vec<Node>) -> Self {
        let span = match (steps.first(), steps.last()) {
            (Some(first), Some(last)) => first.span.to(last.span),
            _ => Span::default(),
        };
        let pos = steps.first().map_or(0, |s| s.pos);
        Node::new(
            NodeKind::Path {
                steps,
                keep_singleton_array: false,
            },
            span,
            pos,
        )
    }

    /// The node type name used by jsonata-js (`path`, `name`, `binary`, ...).
    pub fn type_name(&self) -> &'static str {
        match &self.kind {
            NodeKind::Path { .. } => "path",
            NodeKind::Binary { .. } => "binary",
            NodeKind::Negate { .. } | NodeKind::Array { .. } | NodeKind::Object { .. } => "unary",
            NodeKind::Name { .. } => "name",
            NodeKind::Variable { .. } => "variable",
            NodeKind::String { .. } => "string",
            NodeKind::Number { .. } => "number",
            NodeKind::Bool { .. } | NodeKind::Null => "value",
            NodeKind::Regex { .. } => "regex",
            NodeKind::Wildcard => "wildcard",
            NodeKind::Descendant => "descendant",
            NodeKind::Parent { .. } => "parent",
            NodeKind::Condition { .. } | NodeKind::Fallback { .. } => "condition",
            NodeKind::Block { .. } => "block",
            NodeKind::Bind { .. } => "bind",
            NodeKind::Apply { .. } => "apply",
            NodeKind::Function { .. } => "function",
            NodeKind::Partial { .. } => "partial",
            NodeKind::Placeholder => "operator",
            NodeKind::Lambda { .. } => "lambda",
            NodeKind::Transform { .. } => "transform",
            NodeKind::Sort { .. } => "sort",
            NodeKind::Error { .. } => "error",
        }
    }

    pub(crate) fn steps_mut(&mut self) -> Option<&mut Vec<Node>> {
        match &mut self.kind {
            NodeKind::Path { steps, .. } => Some(steps),
            _ => None,
        }
    }

    /// Visits this node and all its descendants in pre-order (children in source order).
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Node)) {
        f(self);
        self.for_each_child(|c| c.walk(f));
    }

    pub(crate) fn walk_mut(&mut self, f: &mut impl FnMut(&mut Node)) {
        f(self);
        self.for_each_child_mut(&mut |c| c.walk_mut(f));
    }

    /// Calls `f` for each direct child, including predicates, group-by pairs and sort keys.
    pub fn for_each_child<'a>(&'a self, mut f: impl FnMut(&'a Node)) {
        match &self.kind {
            NodeKind::Path { steps, .. } => steps.iter().for_each(&mut f),
            NodeKind::Binary { lhs, rhs, .. }
            | NodeKind::Fallback { lhs, rhs, .. }
            | NodeKind::Bind { lhs, rhs }
            | NodeKind::Apply { lhs, rhs } => {
                f(lhs);
                f(rhs);
            }
            NodeKind::Negate { expr } => f(expr),
            NodeKind::Array { items } => items.iter().for_each(&mut f),
            NodeKind::Object { pairs } => pairs.iter().for_each(|(k, v)| {
                f(k);
                f(v);
            }),
            NodeKind::Condition {
                condition,
                then,
                otherwise,
            } => {
                f(condition);
                f(then);
                if let Some(e) = otherwise {
                    f(e);
                }
            }
            NodeKind::Block { exprs, .. } => exprs.iter().for_each(&mut f),
            NodeKind::Function { procedure, args } | NodeKind::Partial { procedure, args } => {
                f(procedure);
                args.iter().for_each(&mut f);
            }
            NodeKind::Lambda(def) => f(&def.body),
            NodeKind::Transform(def) => {
                f(&def.pattern);
                f(&def.update);
                if let Some(d) = &def.delete {
                    f(d);
                }
            }
            NodeKind::Sort { terms } => terms.iter().for_each(|t| f(&t.expr)),
            _ => {}
        }
        for stage in self.attrs.stages.iter().chain(&self.attrs.predicate) {
            if let Stage::Filter { expr, .. } = stage {
                f(expr);
            }
        }
        if let Some(group) = &self.attrs.group {
            group.pairs.iter().for_each(|(k, v)| {
                f(k);
                f(v);
            });
        }
    }

    fn for_each_child_mut(&mut self, f: &mut impl FnMut(&mut Node)) {
        match &mut self.kind {
            NodeKind::Path { steps, .. } => steps.iter_mut().for_each(&mut *f),
            NodeKind::Binary { lhs, rhs, .. }
            | NodeKind::Fallback { lhs, rhs, .. }
            | NodeKind::Bind { lhs, rhs }
            | NodeKind::Apply { lhs, rhs } => {
                f(lhs);
                f(rhs);
            }
            NodeKind::Negate { expr } => f(expr),
            NodeKind::Array { items } => items.iter_mut().for_each(&mut *f),
            NodeKind::Object { pairs } => pairs.iter_mut().for_each(|(k, v)| {
                f(k);
                f(v);
            }),
            NodeKind::Condition {
                condition,
                then,
                otherwise,
            } => {
                f(condition);
                f(then);
                if let Some(e) = otherwise {
                    f(e);
                }
            }
            NodeKind::Block { exprs, .. } => exprs.iter_mut().for_each(&mut *f),
            NodeKind::Function { procedure, args } | NodeKind::Partial { procedure, args } => {
                f(procedure);
                args.iter_mut().for_each(&mut *f);
            }
            // Definitions are only mutated while the tree is being built, when they are unique.
            NodeKind::Lambda(def) => {
                if let Some(def) = Arc::get_mut(def) {
                    f(&mut def.body);
                }
            }
            NodeKind::Transform(def) => {
                if let Some(def) = Arc::get_mut(def) {
                    f(&mut def.pattern);
                    f(&mut def.update);
                    if let Some(d) = &mut def.delete {
                        f(d);
                    }
                }
            }
            NodeKind::Sort { terms } => terms.iter_mut().for_each(|t| f(&mut t.expr)),
            _ => {}
        }
        for stage in self
            .attrs
            .stages
            .iter_mut()
            .chain(&mut self.attrs.predicate)
        {
            if let Stage::Filter { expr, .. } = stage {
                f(expr);
            }
        }
        if let Some(group) = &mut self.attrs.group {
            group.pairs.iter_mut().for_each(|(k, v)| {
                f(k);
                f(v);
            });
        }
    }
}
