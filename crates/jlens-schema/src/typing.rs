//! Static typing of expressions against a schema.
//!
//! The walk mirrors the evaluator's context rules: each path step is evaluated against the
//! *elements* of the previous step's result (arrays are flattened), predicates see the elements
//! of their step, `@$var` and `:=` bind variables, and block scopes end with the block. The
//! types are sets of schema shapes; `partial` marks that something unknown contributed, in
//! which case no "missing field" warnings are produced. A type also tells whether its value may
//! be undefined, and which operand makes it so.

use std::collections::HashSet;
use std::sync::LazyLock;

use jlens_core::ast::{BinaryOp, FallbackOp, LambdaDef, Node, NodeKind, Span, Stage};

use crate::shape::{Definitions, Field, JsonType, NO_DEFINITIONS, Sh, Shape, members, members_of};

/// The type of a value: the shapes it may have.
#[derive(Debug, Clone, Default)]
pub(crate) struct Ty<'s> {
    /// The shapes, each read through its references already: one that refers to definitions
    /// is replaced by those it stands for.
    pub shapes: Vec<Sh<'s>>,
    /// Something unknown contributed (a user function, an object constructor, no schema...).
    pub partial: bool,
    /// The value may be undefined: the span of the operand that makes it so (a field the
    /// objects may lack, a filter, an array that may be empty, a function that may give
    /// nothing...). `None` when it is always there, as far as the shapes tell: a `partial`
    /// type may be anything, undefined included. The context a node is walked against is
    /// always there, as each item of a path is.
    pub undefined: Option<Span>,
    /// The value may be a sequence: several values gathered in an array, as a path over several
    /// items gives them, and one value alone. The span of the operand that makes it so (the step
    /// over an array, a filter...). The shapes are then those of the values, or arrays of them
    /// that the sequence flattens: [`Ty::elements`] gives the values.
    pub sequence: Option<Span>,
    /// Whether the value is the shapes themselves or an array of them.
    pub form: Form,
}

/// Whether a value is what its shapes describe or an array of such values: an array constructor
/// gives one, as do built-in functions such as `$sort`. Paths read the items of either.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) enum Form {
    /// The shapes themselves.
    #[default]
    Value,
    /// An array whose items have the shapes.
    Array,
    /// Either of the two.
    Either,
}

impl<'s> Ty<'s> {
    pub fn unknown() -> Self {
        Ty {
            shapes: Vec::new(),
            partial: true,
            ..Ty::default()
        }
    }

    /// A known value without fields (number, string, boolean...).
    pub fn scalar() -> Self {
        Ty::default()
    }

    /// A value of one JSON type without fields: a literal, the result of an operator.
    pub fn literal(ty: JsonType) -> Self {
        static SHAPES: LazyLock<[Shape; 4]> = LazyLock::new(|| {
            [
                JsonType::Null,
                JsonType::Boolean,
                JsonType::Number,
                JsonType::String,
            ]
            .map(|t| Shape {
                types: vec![t],
                ..Shape::default()
            })
        });
        match SHAPES.iter().find(|s| s.types[0] == ty) {
            Some(shape) => Ty::of(shape, &NO_DEFINITIONS),
            None => Ty::scalar(),
        }
    }

    /// The type of a value of `shape`, a shape of the schema whose definitions are `defs`.
    pub fn of(shape: &'s Shape, defs: &'s Definitions) -> Self {
        let (shapes, unknown) = members_of(shape, defs);
        Ty {
            partial: unknown || shapes.iter().any(|s| s.types.is_empty()),
            shapes,
            ..Ty::default()
        }
    }

    pub fn union(mut self, other: Ty<'s>) -> Self {
        for s in other.shapes {
            if !self.shapes.iter().any(|t| std::ptr::eq(t.shape, s.shape)) {
                self.shapes.push(s);
            }
        }
        self.partial |= other.partial;
        self.undefined = self.undefined.or(other.undefined);
        self.sequence = self.sequence.or(other.sequence);
        if self.form != other.form {
            self.form = Form::Either;
        }
        self
    }

    /// This type, undefined where `cause` is (when it is `Some`).
    pub fn or_undefined(mut self, cause: Option<Span>) -> Self {
        self.undefined = self.undefined.or(cause);
        self
    }

    /// This type as an array of its values, as an array constructor or `$sort` gives it.
    fn into_array(mut self) -> Self {
        self.form = Form::Array;
        self.sequence = None;
        self.undefined = None;
        self
    }

    /// The items a filter or `$filter` selects from this value, a sequence of them where `cause`
    /// may select several.
    fn selected(&self, cause: Option<Span>) -> Self {
        let mut out = self.items();
        out.undefined = self.undefined;
        out.sequence = cause;
        out
    }

    /// The items of this value, one level down, as a filter reads them: those of an array, a
    /// value that is not an array as it is. [`Ty::elements`] goes down every level, as a path
    /// step does.
    fn items(&self) -> Ty<'s> {
        let mut out = Ty {
            partial: self.partial,
            ..Ty::default()
        };
        let push = |shape: Sh<'s>, out: &mut Ty<'s>| {
            if !out
                .shapes
                .iter()
                .any(|s| std::ptr::eq(s.shape, shape.shape))
            {
                out.shapes.push(shape);
            }
        };
        if self.form != Form::Value {
            // The shapes are those of the items already.
            for shape in &self.shapes {
                push(*shape, &mut out);
            }
        }
        if self.form != Form::Array {
            for shape in &self.shapes {
                if shape.types.is_empty() {
                    out.partial = true;
                    continue;
                }
                if shape.types.contains(&JsonType::Array) {
                    match &shape.shape.items {
                        Some(items) => {
                            let (items, unknown) = members_of(items, shape.defs);
                            out.partial |= unknown;
                            for item in items {
                                out.partial |= item.types.is_empty();
                                push(item, &mut out);
                            }
                        }
                        None => out.partial = true,
                    }
                }
                if shape.types.iter().any(|t| *t != JsonType::Array) {
                    push(*shape, &mut out);
                }
            }
        }
        out
    }

    /// This type, not fully known where `unknown` is: a value computed from a value that may be
    /// anything may be undefined.
    fn unknown_if(mut self, unknown: bool) -> Self {
        self.partial |= unknown;
        self
    }

    /// Whether this value may hold several values for a filter to keep: an array, or a
    /// sequence. A filter of one value keeps it or nothing.
    fn may_hold_several(&self) -> bool {
        self.partial || self.form != Form::Value || self.sequence.is_some() || self.has_arrays()
    }

    /// Whether one of the shapes may be an array.
    fn has_arrays(&self) -> bool {
        self.shapes
            .iter()
            .any(|s| s.types.contains(&JsonType::Array))
    }

    /// Whether one of the shapes may be an empty array: one with no `minItems` of at least 1.
    fn may_be_empty(&self) -> bool {
        self.shapes
            .iter()
            .any(|s| s.types.contains(&JsonType::Array) && s.shape.min_items.is_none_or(|n| n == 0))
    }

    /// Whether `*` on this value may give nothing: an array may be empty, an item may not be an
    /// object, or an object may have none of its fields.
    fn may_have_no_fields(&self) -> bool {
        self.may_be_empty()
            || self.elements().shapes.iter().any(|s| {
                s.types.iter().any(|t| *t != JsonType::Object)
                    || !s.fields.values().any(|f| f.required)
            })
    }

    /// The items a path step iterates over: arrays are replaced by their items, recursively.
    pub fn elements(&self) -> Ty<'s> {
        // `arrays` holds the arrays already entered: a recursive schema leads back to them.
        fn push<'s>(shape: Sh<'s>, out: &mut Ty<'s>, arrays: &mut Vec<*const Shape>) {
            if shape.types.is_empty() {
                out.partial = true;
                return;
            }
            if shape.types.contains(&JsonType::Array)
                && !arrays.contains(&(shape.shape as *const Shape))
            {
                match &shape.shape.items {
                    Some(items) if arrays.len() < 32 => {
                        arrays.push(shape.shape);
                        let (items, unknown) = members_of(items, shape.defs);
                        out.partial |= unknown;
                        for item in items {
                            push(item, out, arrays);
                        }
                    }
                    _ => out.partial = true,
                }
            }
            if shape.types.iter().any(|t| *t != JsonType::Array)
                && !out
                    .shapes
                    .iter()
                    .any(|s| std::ptr::eq(s.shape, shape.shape))
            {
                out.shapes.push(shape);
            }
        }
        let mut out = Ty {
            shapes: Vec::new(),
            partial: self.partial,
            ..Ty::default()
        };
        let mut arrays = Vec::new();
        for shape in &self.shapes {
            push(*shape, &mut out, &mut arrays);
        }
        out
    }

    /// The fields available on the (element) shapes, merged by name in first-seen order, each
    /// with the definitions its shape is read in.
    pub fn fields(&self) -> Vec<(&'s str, &'s Field, &'s Definitions)> {
        let shapes = self.elements().shapes;
        let mut out: Vec<(&'s str, &'s Field, &'s Definitions)> = Vec::new();
        for (index, shape) in shapes.iter().enumerate() {
            for (name, field) in &shape.shape.fields {
                // A shape has each name once: only an earlier shape can have given it already.
                if !shapes[..index].iter().any(|s| s.fields.contains_key(name)) {
                    out.push((name, field, shape.defs));
                }
            }
        }
        out
    }

    /// Whether one of the (element) shapes has field `name`.
    pub fn has_field(&self, name: &str) -> bool {
        self.elements()
            .shapes
            .iter()
            .any(|shape| shape.fields.contains_key(name))
    }

    /// The value of field `name` on the element shapes, looked up by the step at `at`, which
    /// makes it undefined where an array may be empty, an item may not be an object, or an
    /// object may lack the field.
    pub fn field(&self, name: &str, at: Span) -> Ty<'s> {
        let elements = self.elements();
        let mut out = Ty {
            shapes: Vec::new(),
            partial: elements.partial,
            ..Ty::default()
        };
        let mut miss = self.may_be_empty();
        for shape in &elements.shapes {
            let field = shape.shape.fields.get(name);
            miss |= shape.types.iter().any(|t| *t != JsonType::Object)
                || !field.is_some_and(|f| f.required);
            if let Some(f) = field {
                out = out.union(Ty::of(&f.shape, shape.defs));
            }
        }
        out.undefined = miss.then_some(at);
        out
    }

    /// Whether this is known to be objects whose fields are all known.
    pub fn is_known_objects(&self) -> bool {
        let elements = self.elements();
        !elements.partial && elements.shapes.iter().any(|s| s.is_object())
    }

    /// The signature symbols of the values this type may have, and of the items of its arrays;
    /// `None` when the type is not fully known. Undefined (`m`) is left out.
    pub fn symbols(&self) -> Option<Symbols> {
        fn symbol(t: JsonType) -> char {
            match t {
                JsonType::Null => 'l',
                JsonType::Boolean => 'b',
                JsonType::Integer | JsonType::Number => 'n',
                JsonType::String => 's',
                JsonType::Array => 'a',
                JsonType::Object => 'o',
            }
        }
        fn add(set: &mut String, c: char) {
            if !set.contains(c) {
                set.push(c);
            }
        }
        if self.partial || self.shapes.is_empty() {
            return None;
        }
        let mut shapes = Symbols::default();
        for shape in &self.shapes {
            for &t in &shape.types {
                add(&mut shapes.values, symbol(t));
                if t == JsonType::Array {
                    let (items, unknown) = members_of(shape.shape.items.as_deref()?, shape.defs);
                    if unknown || items.iter().any(|i| i.types.is_empty()) {
                        return None;
                    }
                    for &i in items.iter().flat_map(|i| &i.types) {
                        add(&mut shapes.items, symbol(i));
                    }
                }
            }
        }
        // An array of the shapes, or a sequence of their values, is an array whose items are
        // those values.
        let mut out = match self.form {
            Form::Value => shapes.clone(),
            Form::Array => Symbols {
                values: String::new(),
                items: shapes.values.clone(),
            },
            Form::Either => Symbols {
                values: shapes.values.clone(),
                items: shapes.values.clone(),
            },
        };
        if self.form != Form::Value {
            add(&mut out.values, 'a');
            // An array of arrays: their items are counted with the items, which a path
            // flattens.
            for c in shapes.items.chars() {
                add(&mut out.items, c);
            }
        }
        if self.sequence.is_some() {
            add(&mut out.values, 'a');
            let values = shapes.values.chars().filter(|c| *c != 'a');
            for c in values.chain(shapes.items.chars()).collect::<Vec<_>>() {
                add(&mut out.items, c);
            }
        }
        Some(out)
    }

    fn all_field_values(&self) -> Ty<'s> {
        let mut out = Ty {
            shapes: Vec::new(),
            partial: self.elements().partial,
            ..Ty::default()
        };
        for (_, f, defs) in self.fields() {
            out = out.union(Ty::of(&f.shape, defs));
        }
        out
    }

    fn descendants(&self) -> Ty<'s> {
        let mut out = Ty {
            shapes: Vec::new(),
            partial: self.partial,
            ..Ty::default()
        };
        // Iterative, so that a deep schema cannot overflow the stack. `seen` holds the shapes
        // already collected: a recursive schema leads back to them.
        let mut seen: HashSet<*const Shape> = HashSet::new();
        let mut pending = self.elements().shapes;
        pending.reverse();
        while let Some(shape) = pending.pop() {
            if !seen.insert(shape.shape) {
                continue;
            }
            out.partial |= shape.types.is_empty();
            out.shapes.push(shape);
            let mut below = Vec::new();
            let children = shape.shape.fields.values().map(|f| &f.shape);
            for child in children.chain(shape.shape.items.as_deref()) {
                out.partial |= members(child, shape.defs, &mut below);
            }
            pending.extend(below.into_iter().rev());
        }
        out
    }
}

/// Signature symbols (`s`, `n`, `a`...), see [`Ty::symbols`].
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Symbols {
    /// The types of the value.
    pub values: String,
    /// The types of the items of its arrays.
    pub items: String,
}

/// Variables in scope, innermost last.
pub(crate) type Scope<'s> = Vec<(String, Ty<'s>)>;

/// What the walk reports to its visitor.
pub(crate) enum Event<'a, 's> {
    /// A field name used as a path step, with the context it is looked up in.
    Field {
        node: &'a Node,
        name: &'a str,
        ctx: &'a Ty<'s>,
        scope: &'a Scope<'s>,
    },
    /// A variable reference.
    Variable {
        node: &'a Node,
        name: &'a str,
        ty: &'a Ty<'s>,
        scope: &'a Scope<'s>,
    },
    /// A call of a function named by a variable, `$name(...)`, or its partial application.
    Call {
        /// The `$name` node.
        node: &'a Node,
        name: &'a str,
        /// The number of arguments, the value applied with `~>` included.
        args: usize,
        /// The span and type of each argument; `None` for a partial application.
        types: Option<&'a [(Span, Ty<'s>)]>,
    },
    /// A variable bound by `:=`, a function parameter, `@$name` or `#$name`.
    Bind { name: &'a str },
    /// A comparison, `=`, `!=` or `in`, reported after both sides are walked.
    Compare {
        /// The comparison.
        node: &'a Node,
        op: BinaryOp,
        /// The type of the left-hand side.
        lhs: &'a Ty<'s>,
        rhs: &'a Node,
        /// The left-hand side, the operand whose type is `lhs`.
        left: &'a Node,
        /// The type of the right-hand side.
        right: &'a Ty<'s>,
    },
    /// An order comparison (`<`, `<=`, `>`, `>=`) or arithmetic (`+`, `-`, `*`, `/`, `%`),
    /// reported after both sides are walked.
    Operands {
        /// The operation.
        node: &'a Node,
        op: BinaryOp,
        /// The left-hand side and its type.
        lhs: (&'a Node, &'a Ty<'s>),
        /// The right-hand side and its type.
        rhs: (&'a Node, &'a Ty<'s>),
    },
    /// A negation, `-x`, reported after its operand is walked.
    Negate {
        /// The operand.
        operand: &'a Node,
        /// The operand's type.
        ty: &'a Ty<'s>,
    },
}

/// Functions whose result has the type of their first argument.
const PASSTHROUGH: &[&str] = &[
    "sort", "filter", "reverse", "distinct", "shuffle", "single", "clone", "sift",
];
/// Functions whose result structure is not derived from their arguments.
const OPAQUE: &[&str] = &[
    "map", "each", "zip", "eval", "spread", "match", "lookup", "reduce", "error", "assert", "merge",
];

pub(crate) struct Walker<'s, 'v> {
    pub root: Ty<'s>,
    pub externals: &'v [(String, Ty<'s>)],
    pub visit: &'v mut dyn FnMut(Event<'_, 's>),
}

impl<'s> Walker<'s, '_> {
    fn lookup(&self, name: &str, scope: &Scope<'s>) -> Ty<'s> {
        if let Some((_, t)) = scope.iter().rev().find(|(n, _)| n == name) {
            return t.clone();
        }
        if let Some((_, t)) = self.externals.iter().find(|(n, _)| n == name) {
            return t.clone();
        }
        Ty::unknown()
    }

    /// Walks `node` evaluated against context `ctx`, returning the type of its result.
    pub fn walk(&mut self, node: &Node, ctx: &Ty<'s>, scope: &mut Scope<'s>) -> Ty<'s> {
        // Very deep expressions are typed only down to where the stack allows.
        if jlens_core::stack::exhausted() {
            return Ty::unknown();
        }
        jlens_core::stack::grow(|| self.walk_inner(node, ctx, scope))
    }

    fn walk_inner(&mut self, node: &Node, ctx: &Ty<'s>, scope: &mut Scope<'s>) -> Ty<'s> {
        let ty = match &node.kind {
            NodeKind::Path { steps, .. } => {
                let mark = scope.len();
                let mut cur = ctx.clone();
                let mut result = Ty::unknown();
                // Where the path may give nothing. Each step is walked against one item, and
                // gives nothing where its result for an item may be undefined, a filter may
                // drop every item, or the step before gave an empty array. An empty array of
                // the last step is the result as it is, unless the step ran over several items:
                // their arrays are then flattened into a sequence, which is undefined when
                // empty. The array constructor that starts a path is never flattened.
                //
                // `several` is where the steps may start to run over several items: the step
                // that gave arrays, or a sequence. A step that runs over several items gives a
                // sequence, as does a filter that may keep several.
                let mut undefined = None;
                let mut several = (ctx.has_arrays()
                    && !matches!(
                        steps.first().map(|s| &s.kind),
                        Some(NodeKind::Variable { .. })
                    ))
                .then(|| steps.first().map_or(node.span, |s| s.span));
                let mut sequence = None;
                let last = steps.len().saturating_sub(1);
                for (i, step) in steps.iter().enumerate() {
                    let res = self.step(step, &cur, scope);
                    let cons = i == 0 && step.attrs.cons_array;
                    let arrays = !cons && res.has_arrays();
                    undefined = undefined.or(res.undefined);
                    if !cons && res.may_be_empty() && (i < last || several.is_some()) {
                        undefined = undefined.or(Some(step.span));
                    }
                    sequence = several.or(res.sequence);
                    let index = Self::index_filter(step);
                    if (arrays || cons) && !index {
                        several = several.or(Some(step.span));
                    }
                    several = several.or(res.sequence);
                    let elements = res.elements();
                    let mut filtered = false;
                    let holds_several = res.may_hold_several();
                    for stage in &step.attrs.stages {
                        if let Stage::Filter { expr, span, .. } = stage {
                            self.walk(expr, &elements, scope);
                            undefined = undefined.or(Some(*span));
                            filtered = true;
                            if !index && holds_several {
                                sequence = sequence.or(Some(*span));
                            }
                        }
                    }
                    // A filter gives the items it keeps.
                    let res = if filtered { res.items() } else { res };
                    // Other steps are walked whole, group-by included.
                    if let Some(group) = &step.attrs.group
                        && Self::is_simple_step(step)
                    {
                        for (k, v) in &group.pairs {
                            self.walk(k, &elements, scope);
                            self.walk(v, &elements, scope);
                        }
                    }
                    if let Some(index) = &step.attrs.index {
                        (self.visit)(Event::Bind { name: index });
                        scope.push((index.clone(), Ty::scalar()));
                    }
                    match &step.attrs.focus {
                        Some(focus) => {
                            (self.visit)(Event::Bind { name: focus });
                            scope.push((focus.clone(), elements));
                        }
                        None => cur = elements,
                    }
                    result = res;
                }
                if let Some(group) = &node.attrs.group {
                    let elements = result.elements();
                    for (k, v) in &group.pairs {
                        self.walk(k, &elements, scope);
                        self.walk(v, &elements, scope);
                    }
                    result = Ty::unknown();
                } else {
                    result.undefined = undefined;
                    result.sequence = sequence;
                }
                // A tuple stream (`@$v`, `#$i`) gives what its steps are evaluated against,
                // which is not told here.
                let tuples = steps.iter().any(|s| {
                    s.attrs.focus.is_some()
                        || s.attrs.index.is_some()
                        || s.attrs
                            .stages
                            .iter()
                            .any(|t| matches!(t, Stage::Index { .. }))
                });
                scope.truncate(mark);
                return result.unknown_if(tuples);
            }
            NodeKind::Name { .. } | NodeKind::Wildcard | NodeKind::Descendant => {
                self.step(node, ctx, scope)
            }
            NodeKind::Variable { name } => {
                let ty = match name.as_str() {
                    "" => ctx.clone(),
                    "$" => self.root.clone(),
                    other => self.lookup(other, scope),
                };
                (self.visit)(Event::Variable {
                    node,
                    name,
                    ty: &ty,
                    scope,
                });
                ty
            }
            NodeKind::Binary { op, lhs, rhs } => {
                let left = self.walk(lhs, ctx, scope);
                let right = self.walk(rhs, ctx, scope);
                match op {
                    BinaryOp::Eq | BinaryOp::Ne | BinaryOp::In => (self.visit)(Event::Compare {
                        node,
                        op: *op,
                        lhs: &left,
                        rhs,
                        left: lhs,
                        right: &right,
                    }),
                    BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
                    | BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod => (self.visit)(Event::Operands {
                        node,
                        op: *op,
                        lhs: (lhs, &left),
                        rhs: (rhs, &right),
                    }),
                    _ => {}
                }
                // Arithmetic, an order comparison and a range are undefined where a side is;
                // `=`, `!=`, `in`, `and` and `or` are false there, and `&` reads it as "".
                let either = left.undefined.or(right.undefined);
                let unknown = left.partial || right.partial;
                match op {
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod => Ty::literal(JsonType::Number)
                        .or_undefined(either)
                        .unknown_if(unknown),
                    BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                        Ty::literal(JsonType::Boolean)
                            .or_undefined(either)
                            .unknown_if(unknown)
                    }
                    BinaryOp::Concat => Ty::literal(JsonType::String),
                    BinaryOp::Range => Ty::scalar().or_undefined(either).unknown_if(unknown),
                    _ => Ty::literal(JsonType::Boolean),
                }
            }
            NodeKind::Negate { expr } => {
                let ty = self.walk(expr, ctx, scope);
                (self.visit)(Event::Negate {
                    operand: expr,
                    ty: &ty,
                });
                Ty::literal(JsonType::Number)
                    .or_undefined(ty.undefined)
                    .unknown_if(ty.partial)
            }
            NodeKind::Array { items } => {
                let mut ty = Ty::scalar();
                for item in items {
                    ty = ty.union(self.walk(item, ctx, scope));
                }
                // An array is always there, empty where its items are undefined. An array of
                // arrays has items the shapes do not tell.
                let nested = ty.form != Form::Value;
                ty.into_array().unknown_if(nested)
            }
            NodeKind::Object { pairs } => {
                for (k, v) in pairs {
                    self.walk(k, ctx, scope);
                    self.walk(v, ctx, scope);
                }
                Ty::unknown()
            }
            NodeKind::Condition {
                condition,
                then,
                otherwise,
            } => {
                self.walk(condition, ctx, scope);
                let t = self.walk(then, ctx, scope);
                match otherwise {
                    Some(e) => t.union(self.walk(e, ctx, scope)),
                    // Undefined where the condition is false.
                    None => t.or_undefined(Some(condition.span)),
                }
            }
            NodeKind::Fallback { op, lhs, rhs } => {
                let left = self.walk(lhs, ctx, scope);
                let right = self.walk(rhs, ctx, scope);
                // The fallback is taken where the value is undefined (`??`), or where it may
                // also be false (`?:`): the result is undefined only where the fallback is.
                let undefined = match op {
                    FallbackOp::Default => right.undefined,
                    FallbackOp::Coalesce => left.undefined.and(right.undefined),
                };
                let mut t = left.union(right);
                t.undefined = undefined;
                t
            }
            NodeKind::Block { exprs, .. } => {
                let mark = scope.len();
                // `()` is undefined.
                let mut last = Ty::unknown().or_undefined(Some(node.span));
                for e in exprs {
                    last = self.walk(e, ctx, scope);
                }
                scope.truncate(mark);
                last
            }
            NodeKind::Bind { lhs, rhs } => {
                let ty = self.walk(rhs, ctx, scope);
                if let NodeKind::Variable { name } = &lhs.kind {
                    (self.visit)(Event::Bind { name });
                    scope.push((name.clone(), ty.clone()));
                }
                ty
            }
            NodeKind::Function { procedure, args } => {
                self.call(node.span, procedure, args, None, ctx, scope)
            }
            NodeKind::Partial { procedure, args } => {
                self.walk(procedure, ctx, scope);
                if let NodeKind::Variable { name } = &procedure.kind {
                    (self.visit)(Event::Call {
                        node: procedure,
                        name,
                        args: args.len(),
                        types: None,
                    });
                }
                for a in args {
                    self.walk(a, ctx, scope);
                }
                Ty::unknown()
            }
            NodeKind::Apply { lhs, rhs } => {
                let value = self.walk(lhs, ctx, scope);
                match &rhs.kind {
                    NodeKind::Function { procedure, args } => self.call(
                        rhs.span,
                        procedure,
                        args,
                        Some((lhs.span, value)),
                        ctx,
                        scope,
                    ),
                    _ => {
                        self.walk(rhs, ctx, scope);
                        Ty::unknown()
                    }
                }
            }
            NodeKind::Lambda(def) => {
                self.lambda(def, &[], ctx, scope);
                Ty::unknown()
            }
            NodeKind::Transform(def) => {
                let matches = self.walk(&def.pattern, ctx, scope).elements();
                self.walk(&def.update, &matches, scope);
                if let Some(d) = &def.delete {
                    self.walk(d, &matches, scope);
                }
                Ty::unknown()
            }
            NodeKind::Sort { terms } => {
                for t in terms {
                    self.walk(&t.expr, ctx, scope);
                }
                ctx.clone()
            }
            NodeKind::String { .. } => Ty::literal(JsonType::String),
            NodeKind::Number { .. } => Ty::literal(JsonType::Number),
            NodeKind::Bool { .. } => Ty::literal(JsonType::Boolean),
            NodeKind::Null => Ty::literal(JsonType::Null),
            NodeKind::Regex { .. } => Ty::scalar(),
            NodeKind::Parent { .. } | NodeKind::Placeholder | NodeKind::Error { .. } => {
                Ty::unknown()
            }
        };
        let mut ty = ty;
        let elements = ty.elements();
        let index = Self::index_filter_of(&node.attrs.predicate);
        for stage in &node.attrs.predicate {
            if let Stage::Filter { expr, span, .. } = stage {
                self.walk(expr, &elements, scope);
                // A filter gives the items it keeps: one for an index or of one value, a
                // sequence otherwise.
                let undefined = ty.undefined.or(Some(*span));
                let sequence = (!index && ty.may_hold_several()).then_some(*span);
                ty = ty.selected(sequence);
                ty.undefined = undefined;
            }
        }
        if let Some(group) = &node.attrs.group {
            for (k, v) in &group.pairs {
                self.walk(k, &elements, scope);
                self.walk(v, &elements, scope);
            }
            return Ty::unknown();
        }
        ty
    }

    /// The result of one path step. Its stages and group-by are handled by the path.
    fn step(&mut self, step: &Node, ctx: &Ty<'s>, scope: &mut Scope<'s>) -> Ty<'s> {
        match &step.kind {
            NodeKind::Name { name } => {
                (self.visit)(Event::Field {
                    node: step,
                    name,
                    ctx,
                    scope,
                });
                ctx.field(name, step.span)
            }
            // The values of the fields: several where an object may have more than one.
            NodeKind::Wildcard => {
                let miss = ctx.may_have_no_fields();
                let mut ty = ctx
                    .all_field_values()
                    .or_undefined(miss.then_some(step.span));
                if ctx.fields().len() > 1 {
                    ty.sequence = Some(step.span);
                }
                ty
            }
            // Descendants are not told apart: any may be missing, and there may be several.
            NodeKind::Descendant => {
                let mut ty = ctx.descendants().or_undefined(Some(step.span));
                ty.sequence = Some(step.span);
                ty
            }
            NodeKind::Sort { terms } => {
                for t in terms {
                    self.walk(&t.expr, ctx, scope);
                }
                ctx.clone()
            }
            _ => self.walk(step, ctx, scope),
        }
    }

    /// Whether a filter of `step` is a number, `[0]` or `[-1]`: it keeps one item at most of
    /// each array the step gives.
    fn index_filter(step: &Node) -> bool {
        Self::index_filter_of(&step.attrs.stages)
    }

    /// Whether one of the filters of `stages` is a number, which keeps one item at most: of the
    /// items the filters before it kept, and the filters after it keep that one or none.
    fn index_filter_of(stages: &[Stage]) -> bool {
        let number = |e: &Node| match &e.kind {
            NodeKind::Number { .. } => true,
            NodeKind::Negate { expr } => matches!(expr.kind, NodeKind::Number { .. }),
            _ => false,
        };
        stages
            .iter()
            .any(|s| matches!(s, Stage::Filter { expr, .. } if number(expr)))
    }

    fn is_simple_step(step: &Node) -> bool {
        matches!(
            step.kind,
            NodeKind::Name { .. }
                | NodeKind::Wildcard
                | NodeKind::Descendant
                | NodeKind::Sort { .. }
        )
    }

    fn lambda(&mut self, def: &LambdaDef, hints: &[Ty<'s>], ctx: &Ty<'s>, scope: &mut Scope<'s>) {
        let mark = scope.len();
        for (i, p) in def.params.iter().enumerate() {
            (self.visit)(Event::Bind { name: p });
            scope.push((p.clone(), hints.get(i).cloned().unwrap_or_else(Ty::unknown)));
        }
        self.walk(&def.body, ctx, scope);
        scope.truncate(mark);
    }

    /// The type of a call: `span` is the call's, `applied` the value applied with `~>`.
    fn call(
        &mut self,
        span: Span,
        procedure: &Node,
        args: &[Node],
        applied: Option<(Span, Ty<'s>)>,
        ctx: &Ty<'s>,
        scope: &mut Scope<'s>,
    ) -> Ty<'s> {
        self.walk(procedure, ctx, scope);
        let name = match &procedure.kind {
            NodeKind::Variable { name } => Some(name.as_str()),
            _ => None,
        };
        let mut typed: Vec<(Span, Ty<'s>)> = applied.into_iter().collect();
        for arg in args {
            let first = typed.first().map_or_else(Ty::unknown, |(_, t)| t.clone());
            match &arg.kind {
                NodeKind::Lambda(def) if typed.len() == 1 => {
                    // Type the parameters of callbacks from the array they iterate over.
                    let elements = first.elements();
                    let hints = match name {
                        Some("map" | "filter" | "single") => vec![elements, Ty::scalar(), first],
                        Some("sort") => vec![elements.clone(), elements],
                        Some("reduce") => vec![Ty::unknown(), elements],
                        _ => Vec::new(),
                    };
                    self.lambda(def, &hints, ctx, scope);
                    typed.push((arg.span, Ty::unknown()));
                }
                _ => typed.push((arg.span, self.walk(arg, ctx, scope))),
            }
        }
        if let Some(name) = name {
            (self.visit)(Event::Call {
                node: procedure,
                name,
                args: typed.len(),
                types: Some(&typed),
            });
        }
        let types: Vec<Ty<'s>> = typed.into_iter().map(|(_, t)| t).collect();
        let builtin = name.is_some_and(|n| {
            jlens_core::function_catalog().iter().any(|f| f.name == n)
                && !scope.iter().any(|(v, _)| v == n)
                && !self.externals.iter().any(|(v, _)| v == n)
        });
        let Some(name) = name.filter(|_| builtin) else {
            return Ty::unknown();
        };
        // An argument left out is the context, where the signature allows it, which is there.
        let args = |n: usize| types.iter().take(n).find_map(|t| t.undefined);
        // An argument not fully known may be undefined too, as may a context not fully known.
        let unknown =
            |n: usize| types.iter().take(n).any(|t| t.partial) || (types.len() < n && ctx.partial);
        let gives = gives_undefined(name).unwrap_or(GivesUndefined::Maybe);
        let unknown = match gives {
            GivesUndefined::For(n) => unknown(n),
            _ => false,
        };
        let undefined = match gives {
            GivesUndefined::Never => None,
            GivesUndefined::For(n) => args(n),
            // Undefined for an empty array, which a sequence never is (it is undefined then).
            GivesUndefined::Maybe if matches!(name, "max" | "min" | "average") => {
                let empty = types.first().is_none_or(|t| {
                    t.partial || t.form != Form::Value || (t.sequence.is_none() && t.may_be_empty())
                });
                args(1).or(empty.then_some(span))
            }
            GivesUndefined::Maybe => args(1).or(Some(span)),
        };
        let first = || types.first().cloned().unwrap_or_else(Ty::unknown);
        let mut ty = match name {
            // An array, even of one value: `$sort(5)` is `[5]`.
            "sort" | "reverse" | "shuffle" => first().into_array(),
            // The items kept, a sequence of them.
            "filter" => first().selected(Some(span)),
            "single" => first().selected(None),
            "append" => {
                // An array, undefined only where both arguments are; one argument alone, as it
                // is, where the other is undefined.
                let mut it = types.into_iter();
                let a = it.next().unwrap_or_default();
                let b = it.next().unwrap_or_default();
                let undefined = a.undefined.filter(|_| b.undefined.is_some());
                let alone =
                    a.undefined.is_some() || b.undefined.is_some() || a.partial || b.partial;
                let mut ty = a.elements().union(b.elements()).into_array();
                if alone {
                    ty.form = Form::Either;
                }
                ty.undefined = undefined;
                return ty;
            }
            "split" => Ty::literal(JsonType::String).into_array(),
            // A sequence of the keys: one alone is a string.
            "keys" => {
                let mut ty = Ty::literal(JsonType::String);
                ty.sequence = Some(span);
                ty
            }
            _ if PASSTHROUGH.contains(&name) => first(),
            _ if OPAQUE.contains(&name) => Ty::unknown(),
            // The type its signature gives.
            _ => match result_symbol(name) {
                Some('n') => Ty::literal(JsonType::Number),
                Some('s') => Ty::literal(JsonType::String),
                Some('b') => Ty::literal(JsonType::Boolean),
                _ => Ty::scalar(),
            },
        };
        ty.undefined = undefined;
        ty.unknown_if(unknown)
    }
}

/// The symbol of the result type in the signature of the built-in function `name`, `<s-:n>` for
/// `$length`, when it is one letter.
fn result_symbol(name: &str) -> Option<char> {
    let f = jlens_core::function_catalog()
        .into_iter()
        .find(|f| f.name == name)?;
    let result = f.signature.strip_suffix('>')?.rsplit_once(':')?.1;
    let mut chars = result.chars();
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

/// When a built-in function gives undefined, as jsonata-js does, short of an error.
#[derive(Debug, Clone, Copy, PartialEq)]
enum GivesUndefined {
    /// Never.
    Never,
    /// Where one of its first `n` arguments is undefined.
    For(usize),
    /// Where its first argument is undefined, and for some values of its arguments too (a key
    /// that is not there, a filter that keeps nothing, an empty array...).
    Maybe,
}

/// When the built-in function `name` gives undefined; `None` for a name this table does not
/// know, which a test keeps from being a built-in. `append` is undefined where both its
/// arguments are, which the walker handles.
fn gives_undefined(name: &str) -> Option<GivesUndefined> {
    use GivesUndefined::*;
    Some(match name {
        "exists" | "count" | "random" | "millis" | "now" | "zip" | "append" => Never,
        "string" | "substring" | "substringBefore" | "substringAfter" | "lowercase"
        | "uppercase" | "length" | "trim" | "pad" | "join" | "formatNumber" | "formatBase"
        | "formatInteger" | "parseInteger" | "number" | "floor" | "ceil" | "round" | "abs"
        | "sqrt" | "power" | "boolean" | "not" | "type" | "clone" | "base64encode"
        | "base64decode" | "encodeUrlComponent" | "encodeUrl" | "decodeUrlComponent"
        | "decodeUrl" | "fromMillis" | "sum" | "sort" | "reverse" | "shuffle" | "distinct"
        | "single" | "merge" => For(1),
        "contains" | "split" | "replace" => For(2),
        "lookup" | "filter" | "sift" | "average" | "max" | "min" | "match" | "map" | "reduce"
        | "keys" | "spread" | "each" | "eval" | "assert" | "error" | "toMillis" => Maybe,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Schema;
    use serde_json::json;

    /// An order: `id` always there, `note` optional, `customer` an object with a required
    /// `name` and an optional `email`, `items` an array of objects with a required `price` and
    /// optional `tags`, `status` a string or null, `meta.sizes` an array.
    fn schema() -> Schema {
        Schema::from_json_schema(&json!({
            "type": "object",
            "required": ["id", "customer", "items", "status", "meta"],
            "properties": {
                "id": {"type": "integer"},
                "note": {"type": "string"},
                "customer": {
                    "type": "object",
                    "required": ["name"],
                    "properties": {
                        "name": {"type": "string"},
                        "email": {"type": "string"},
                        "address": {
                            "type": "object",
                            "required": ["city"],
                            "properties": {"city": {"type": "string"}}
                        }
                    }
                },
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["price"],
                        "properties": {
                            "price": {"type": "number"},
                            "tags": {"type": "array", "items": {"type": "string"}}
                        }
                    }
                },
                "codes": {"type": "array", "items": {"type": "string"}},
                "status": {"type": ["string", "null"]},
                "meta": {
                    "type": "object",
                    "required": ["sizes"],
                    "properties": {"sizes": {"type": "array", "items": {"type": "number"}}}
                }
            }
        }))
        .unwrap()
    }

    /// The text of the operand that may make `expr` undefined, `None` when it is always there.
    fn cause(expr: &str) -> Option<String> {
        let schema = schema();
        let ast = jlens_core::parse_recover(expr);
        assert!(ast.diagnostics.is_empty(), "{expr}: {:?}", ast.diagnostics);
        let root = Ty::of(&schema.root, &schema.definitions);
        let mut visit = |_: Event<'_, '_>| {};
        let mut walker = Walker {
            root: root.clone(),
            externals: &[("bound".to_string(), Ty::literal(JsonType::Number))],
            visit: &mut visit,
        };
        let ty = walker.walk(&ast.root, &root, &mut Vec::new());
        ty.undefined.map(|s| expr[s.start..s.end].to_string())
    }

    /// Documents that follow [`schema`]: every optional field there, none of them, and arrays
    /// empty or with several items.
    fn documents() -> [serde_json::Value; 3] {
        [
            json!({
                "id": 1, "note": "n", "status": "s", "codes": ["c"], "meta": {"sizes": [1]},
                "customer": {"name": "Ann", "email": "a@b", "address": {"city": "X"}},
                "items": [{"price": 2, "tags": ["t"]}]
            }),
            json!({
                "id": 1, "status": null, "meta": {"sizes": []},
                "customer": {"name": "Bo"}, "items": []
            }),
            json!({
                "id": 2, "status": null, "codes": [], "meta": {"sizes": []},
                "customer": {"name": "Cy", "address": {"city": "Y"}},
                "items": [{"price": 1, "tags": []}, {"price": 3, "tags": []}]
            }),
        ]
    }

    /// Whether `expr` is defined on each of the [`documents`], as the engine evaluates it.
    fn defined(expr: &str) -> Vec<bool> {
        let compiled = jlens_core::Expression::compile(expr).unwrap();
        let bindings = jlens_core::Bindings::new().var("bound", 1);
        documents()
            .iter()
            .map(|doc| compiled.evaluate(Some(doc), &bindings).unwrap().is_some())
            .collect()
    }

    #[test]
    fn values_always_there() {
        for expr in [
            "id",
            "customer.name",
            "$.customer.name",
            "$$.id",
            // An array field with no step after it is the array, empty or not.
            "items",
            "meta.sizes",
            // Every item of a path has its required fields, as a predicate's context does.
            "$count(items[price > 10])",
            "items[$exists(tags)] ~> $count()",
            "1",
            "'a'",
            "null",
            "true",
            "id + 1",
            "-id",
            "id > 1",
            "customer.name & note",
            "note = 'a'",
            "note in ['a']",
            "note and true",
            "id > 1 ? 'big' : note ?: 'none'",
            "note ?? 'none'",
            "[note]",
            "[note, items.tags]",
            "{'a': note}",
            "($x := id; $x + 1)",
            "$exists(note)",
            "$count(items.tags)",
            "$sort(meta.sizes)",
            "customer.*",
            "$append(note, id)",
            "$uppercase(customer.name)",
            "$substring(customer.name, $length(note))",
            "$string(id)",
            "customer.name ~> $uppercase()",
            "$replace(customer.name, 'a', note)",
            "$bound + 1",
            "$now()",
        ] {
            assert_eq!(cause(expr), None, "{expr}");
            assert!(
                !defined(expr).contains(&false),
                "{expr} is undefined on a document"
            );
        }
    }

    #[test]
    fn values_that_may_be_undefined() {
        for (expr, by) in [
            // An optional field, and what is under it.
            ("note", "note"),
            ("customer.email", "email"),
            ("customer.address", "address"),
            ("customer.address.city", "address"),
            ("$.customer.email", "email"),
            // A value that is not always an object has no fields.
            ("status.code", "code"),
            // A step over an array that may be empty gives nothing.
            ("items.price", "items"),
            ("meta.sizes.x", "sizes"),
            // The arrays of several items are flattened into a sequence, empty when they are.
            ("items.tags", "items"),
            ("items[0].tags", "items[0]"),
            ("codes", "codes"),
            // A filter may keep nothing.
            ("customer[name = 'a']", "[name = 'a']"),
            ("$.customer[0].name", "[0]"),
            ("customer.name[0]", "[0]"),
            ("[1, 2][0]", "[0]"),
            ("status.*", "*"),
            ("$sum(items.price)", "items"),
            ("**.name", "**"),
            // Operators that give undefined for an undefined operand.
            ("note > 'a'", "note"),
            ("id + $length(customer.email)", "email"),
            ("-$length(note)", "note"),
            ("customer.name ? 1", "customer.name"),
            ("id > 1 ? 'big' : note", "note"),
            ("note ?: customer.email", "email"),
            ("note ?? customer.email", "email"),
            ("($x := note; $x)", "note"),
            ("()", "()"),
            ("customer.(email)", "email"),
            ("customer.($length(email) + 1)", "email"),
            // Functions.
            ("$uppercase(note)", "note"),
            ("note ~> $uppercase()", "note"),
            ("$contains(customer.name, note)", "note"),
            ("$append(note, customer.email)", "note"),
            ("$sort(codes)", "codes"),
            ("$lookup(customer, 'name')", "$lookup(customer, 'name')"),
            (
                "$filter(items, function($i) { $i.price > 1 })",
                "$filter(items, function($i) { $i.price > 1 })",
            ),
            ("$max(items.price)", "items"),
            ("$keys(customer)", "$keys(customer)"),
            ("customer ~> $keys()", "$keys()"),
        ] {
            assert_eq!(cause(expr).as_deref(), Some(by), "{expr}");
            // The typing does not read the values of a filter or a condition, nor which key a
            // function is given: these may be undefined for all it knows.
            let conservative = [
                "$.customer[0].name",
                "customer.name[0]",
                "[1, 2][0]",
                "**.name",
                "customer.name ? 1",
                "$lookup(customer, 'name')",
                "$keys(customer)",
                "customer ~> $keys()",
            ];
            assert!(
                conservative.contains(&expr) || defined(expr).contains(&false),
                "{expr} is defined on every document"
            );
        }
    }

    #[test]
    fn every_built_in_function_is_classified() {
        for f in jlens_core::function_catalog() {
            assert!(gives_undefined(f.name).is_some(), "${}", f.name);
        }
    }
}
