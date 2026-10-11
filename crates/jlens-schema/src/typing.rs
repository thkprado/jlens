//! Static typing of expressions against a schema.
//!
//! The walk mirrors the evaluator's context rules: each path step is evaluated against the
//! *elements* of the previous step's result (arrays are flattened), predicates see the elements
//! of their step, `@$var` and `:=` bind variables, and block scopes end with the block. The
//! types are sets of schema shapes; `partial` marks that something unknown contributed, in
//! which case no "missing field" warnings are produced.

use std::sync::LazyLock;

use jlens_core::ast::{BinaryOp, LambdaDef, Node, NodeKind, Span, Stage};

use crate::shape::{Field, JsonType, Shape};

/// The type of a value: the shapes it may have.
#[derive(Debug, Clone, Default)]
pub(crate) struct Ty<'s> {
    pub shapes: Vec<&'s Shape>,
    /// Something unknown contributed (a user function, an object constructor, no schema...).
    pub partial: bool,
}

impl<'s> Ty<'s> {
    pub fn unknown() -> Self {
        Ty {
            shapes: Vec::new(),
            partial: true,
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
            Some(shape) => Ty::of(shape),
            None => Ty::scalar(),
        }
    }

    pub fn of(shape: &'s Shape) -> Self {
        Ty {
            shapes: vec![shape],
            partial: shape.types.is_empty(),
        }
    }

    pub fn union(mut self, other: Ty<'s>) -> Self {
        for s in other.shapes {
            if !self.shapes.iter().any(|t| std::ptr::eq(*t, s)) {
                self.shapes.push(s);
            }
        }
        self.partial |= other.partial;
        self
    }

    /// The items a path step iterates over: arrays are replaced by their items, recursively.
    pub fn elements(&self) -> Ty<'s> {
        fn push<'s>(shape: &'s Shape, out: &mut Ty<'s>, depth: usize) {
            if shape.types.is_empty() {
                out.partial = true;
                return;
            }
            if shape.types.contains(&JsonType::Array) {
                match &shape.items {
                    Some(items) if depth < 32 => push(items, out, depth + 1),
                    _ => out.partial = true,
                }
            }
            if shape.types.iter().any(|t| *t != JsonType::Array)
                && !out.shapes.iter().any(|s| std::ptr::eq(*s, shape))
            {
                out.shapes.push(shape);
            }
        }
        let mut out = Ty {
            shapes: Vec::new(),
            partial: self.partial,
        };
        for shape in &self.shapes {
            push(shape, &mut out, 0);
        }
        out
    }

    /// The fields available on the (element) shapes, merged by name in first-seen order.
    pub fn fields(&self) -> Vec<(&'s str, &'s Field)> {
        let mut out: Vec<(&'s str, &'s Field)> = Vec::new();
        for shape in &self.elements().shapes {
            for (name, field) in &shape.fields {
                if !out.iter().any(|(n, _)| *n == name) {
                    out.push((name, field));
                }
            }
        }
        out
    }

    /// The value of field `name` on the element shapes.
    pub fn field(&self, name: &str) -> Ty<'s> {
        let elements = self.elements();
        let mut out = Ty {
            shapes: Vec::new(),
            partial: elements.partial,
        };
        for shape in &elements.shapes {
            if let Some(f) = shape.fields.get(name) {
                out = out.union(Ty::of(&f.shape));
            }
        }
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
        let mut out = Symbols::default();
        for shape in &self.shapes {
            for &t in &shape.types {
                add(&mut out.values, symbol(t));
                if t == JsonType::Array {
                    let items = shape.items.as_deref().filter(|i| !i.types.is_empty())?;
                    for &i in &items.types {
                        add(&mut out.items, symbol(i));
                    }
                }
            }
        }
        Some(out)
    }

    fn all_field_values(&self) -> Ty<'s> {
        let mut out = Ty {
            shapes: Vec::new(),
            partial: self.elements().partial,
        };
        for (_, f) in self.fields() {
            out = out.union(Ty::of(&f.shape));
        }
        out
    }

    fn descendants(&self) -> Ty<'s> {
        fn collect<'s>(shape: &'s Shape, out: &mut Ty<'s>, depth: usize) {
            if depth > 32 {
                out.partial = true;
                return;
            }
            *out = std::mem::take(out).union(Ty::of(shape));
            for f in shape.fields.values() {
                collect(&f.shape, out, depth + 1);
            }
            if let Some(items) = &shape.items {
                collect(items, out, depth + 1);
            }
        }
        let mut out = Ty {
            shapes: Vec::new(),
            partial: self.partial,
        };
        for shape in self.elements().shapes {
            collect(shape, &mut out, 0);
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
                for step in steps {
                    let res = self.step(step, &cur, scope);
                    let elements = res.elements();
                    for stage in &step.attrs.stages {
                        if let Stage::Filter { expr, .. } = stage {
                            self.walk(expr, &elements, scope);
                        }
                    }
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
                }
                scope.truncate(mark);
                return result;
            }
            NodeKind::Name { name } => {
                (self.visit)(Event::Field {
                    node,
                    name,
                    ctx,
                    scope,
                });
                ctx.field(name)
            }
            NodeKind::Wildcard => ctx.all_field_values(),
            NodeKind::Descendant => ctx.descendants(),
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
                self.walk(lhs, ctx, scope);
                self.walk(rhs, ctx, scope);
                match op {
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod => Ty::literal(JsonType::Number),
                    BinaryOp::Concat => Ty::literal(JsonType::String),
                    BinaryOp::Range => Ty::scalar(),
                    _ => Ty::literal(JsonType::Boolean),
                }
            }
            NodeKind::Negate { expr } => {
                self.walk(expr, ctx, scope);
                Ty::literal(JsonType::Number)
            }
            NodeKind::Array { items } => {
                let mut ty = Ty::scalar();
                for item in items {
                    ty = ty.union(self.walk(item, ctx, scope));
                }
                ty
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
                    None => t,
                }
            }
            NodeKind::Fallback { lhs, rhs, .. } => {
                let t = self.walk(lhs, ctx, scope);
                t.union(self.walk(rhs, ctx, scope))
            }
            NodeKind::Block { exprs } => {
                let mark = scope.len();
                let mut last = Ty::unknown();
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
            NodeKind::Function { procedure, args } => self.call(procedure, args, None, ctx, scope),
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
                    NodeKind::Function { procedure, args } => {
                        self.call(procedure, args, Some((lhs.span, value)), ctx, scope)
                    }
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
        let elements = ty.elements();
        for stage in &node.attrs.predicate {
            if let Stage::Filter { expr, .. } = stage {
                self.walk(expr, &elements, scope);
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
                ctx.field(name)
            }
            NodeKind::Wildcard => ctx.all_field_values(),
            NodeKind::Descendant => ctx.descendants(),
            NodeKind::Sort { terms } => {
                for t in terms {
                    self.walk(&t.expr, ctx, scope);
                }
                ctx.clone()
            }
            _ => self.walk(step, ctx, scope),
        }
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

    fn call(
        &mut self,
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
        match name {
            Some(n) if builtin && PASSTHROUGH.contains(&n) => {
                types.first().cloned().unwrap_or_else(Ty::unknown)
            }
            Some("append") if builtin => {
                let mut it = types.into_iter();
                let a = it.next().unwrap_or_default();
                a.union(it.next().unwrap_or_default())
            }
            Some(n) if builtin && !OPAQUE.contains(&n) => Ty::scalar(),
            _ => Ty::unknown(),
        }
    }
}
