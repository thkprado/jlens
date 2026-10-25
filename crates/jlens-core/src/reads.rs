//! The fields an expression reads ([ADR 0016]).
//!
//! [`Expression::reads`](crate::Expression::reads) analyses an expression without evaluating
//! it and returns its [`Reads`]: what it may read of its input and of each binding, as a tree
//! of field names ([`Read`]). An application can fetch only those fields, or tell which
//! expressions depend on a field.
//!
//! The answer is an upper bound. Evaluated on its input and bindings cut by its `Reads`
//! ([`Read::cut`]), an expression gives the same result, or fails with the same error code, as
//! on the whole ones. Where the analysis cannot follow a value it keeps the value whole: under
//! `*` and `**`, in the values of a constructed object, in the arguments of `$string`, `$keys`
//! and most other functions, and in the result.
//!
//! ```
//! use jlens_core::Expression;
//! use serde_json::json;
//!
//! let expr = Expression::compile(
//!     r#"$sum(items[type = "book"].(price * quantity)) * (1 - $discount)"#,
//! )
//! .unwrap();
//! let reads = expr.reads();
//! assert_eq!(
//!     serde_json::to_value(&reads).unwrap(),
//!     json!({
//!         "input": {"items": {"type": true, "price": true, "quantity": true}},
//!         "bindings": {"discount": true}
//!     })
//! );
//!
//! let input = json!({
//!     "items": [{"type": "book", "price": 10, "quantity": 2, "title": "Dune"}],
//!     "customer": {"name": "Ada"}
//! });
//! let cut = reads.input().unwrap().cut(&input);
//! assert_eq!(cut, json!({"items": [{"type": "book", "price": 10, "quantity": 2}]}));
//! ```
//!
//! Two rules of the analysis concern the application rather than the expression:
//!
//! - A call to a function that is not a built-in reads its context whole, since a host
//!   function can read it ([`HostCall::input`](crate::HostCall::input)). `$tax(price)` at the
//!   top of an expression reads the whole input; `price.$tax()` reads `price`. A host function
//!   marked [`without_input`](crate::HostFunction::without_input) cannot read it, and
//!   [`Expression::reads_with`](crate::Expression::reads_with), given the bindings, counts a
//!   call to it as reading its arguments only.
//! - A binding or host function named after a built-in shadows it during the evaluation, but
//!   the analysis takes the name as the built-in's. Such an application cannot cut its input
//!   by the analysis.
//!
//! [ADR 0016]: https://github.com/thkprado/jlens/blob/main/docs/src/adr/0016-fields-read.md

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt;

use serde::de::{self, DeserializeSeed, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value as JsonValue;

use crate::Bindings;
use crate::ast::{Ast, FallbackOp, LambdaDef, Node, NodeKind, Stage, TransformDef};
use crate::functions::{self, Native};
use crate::signature::Signature;

/// The most field names a [`Read`] nests. The analysis reads a value whole at this depth, and
/// deserializing a deeper `Read` fails.
pub const MAX_DEPTH: usize = 100;

/// The most places an analysed value has before the analysis reads them all whole, so that
/// expressions such as `[a, b].[a, b].[a, b]…` stay linear.
const MAX_PLACES: usize = 256;

/// What is needed of a value, whatever its type.
///
/// Its JSON form writes `Whole` as `true` and `Fields` as an object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    /// The value as it is.
    Whole,
    /// Of an object, only these fields, each cut by its own `Read`; of an array, its items,
    /// each needing the same `Fields`, at any depth; of a scalar, the scalar. An empty map
    /// needs the value but none of its fields, as `$count`, `$exists` and `$type` do.
    Fields(BTreeMap<String, Read>),
}

impl Read {
    fn needed() -> Read {
        Read::Fields(BTreeMap::new())
    }

    /// Whether the value is needed whole.
    pub fn is_whole(&self) -> bool {
        matches!(self, Read::Whole)
    }

    /// The fields read, unless the value is needed whole.
    pub fn fields(&self) -> Option<&BTreeMap<String, Read>> {
        match self {
            Read::Whole => None,
            Read::Fields(fields) => Some(fields),
        }
    }

    /// What either `Read` needs: `Whole` absorbs everything, and two `Fields` merge their maps,
    /// uniting the `Read`s of a field present in both.
    pub fn union(&self, other: &Read) -> Read {
        let mut read = self.clone();
        read.unite(other);
        read
    }

    fn unite(&mut self, other: &Read) {
        crate::stack::grow(|| match (&mut *self, other) {
            (Read::Whole, _) => {}
            (_, Read::Whole) => *self = Read::Whole,
            (Read::Fields(mine), Read::Fields(theirs)) => {
                for (name, read) in theirs {
                    match mine.get_mut(name) {
                        Some(own) => own.unite(read),
                        None => {
                            mine.insert(name.clone(), read.clone());
                        }
                    }
                }
            }
        })
    }

    /// The value cut by this `Read`: `Whole` keeps the value; `Fields` keeps an object's listed
    /// fields, each cut by its own `Read`, cuts each item of an array by the same `Fields`, and
    /// keeps a scalar. Fields keep their order.
    pub fn cut(&self, value: &JsonValue) -> JsonValue {
        crate::stack::grow(|| match (self, value) {
            (Read::Fields(fields), JsonValue::Object(object)) => JsonValue::Object(
                object
                    .iter()
                    .filter_map(|(name, v)| fields.get(name).map(|r| (name.clone(), r.cut(v))))
                    .collect(),
            ),
            (Read::Fields(_), JsonValue::Array(items)) => {
                JsonValue::Array(items.iter().map(|item| self.cut(item)).collect())
            }
            (_, value) => value.clone(),
        })
    }
}

impl Serialize for Read {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Read::Whole => serializer.serialize_bool(true),
            Read::Fields(fields) => fields.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Read {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ReadSeed(0).deserialize(deserializer)
    }
}

/// Deserializes a [`Read`] nested `.0` field names deep.
struct ReadSeed(usize);

impl<'de> DeserializeSeed<'de> for ReadSeed {
    type Value = Read;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Read, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for ReadSeed {
    type Value = Read;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("true, or an object of the fields read")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Read, E> {
        if v {
            Ok(Read::Whole)
        } else {
            Err(E::invalid_value(de::Unexpected::Bool(false), &self))
        }
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Read, A::Error> {
        if self.0 > MAX_DEPTH {
            return Err(de::Error::custom(format_args!(
                "a read nests more than {MAX_DEPTH} field names"
            )));
        }
        let mut fields = BTreeMap::new();
        while let Some(name) = map.next_key::<String>()? {
            let read = map.next_value_seed(ReadSeed(self.0 + 1))?;
            fields.insert(name, read);
        }
        Ok(Read::Fields(fields))
    }
}

/// What an expression may read of its input and of each binding, given by
/// [`Expression::reads`](crate::Expression::reads).
///
/// Its JSON form is `{"input": Read, "bindings": {name: Read}, "everyBinding": true}`, without
/// `input` when the expression does not read it and without `everyBinding` when it is false.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reads {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input: Option<Read>,
    #[serde(default)]
    bindings: BTreeMap<String, Read>,
    #[serde(default, skip_serializing_if = "is_false")]
    every_binding: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Reads {
    /// What the expression reads of its input; `None` when it does not read it.
    pub fn input(&self) -> Option<&Read> {
        self.input.as_ref()
    }

    /// What the expression reads of the binding `name` (without the `$`); `None` when it does
    /// not read it, unless [`every_binding`](Reads::every_binding) is set.
    pub fn binding(&self, name: &str) -> Option<&Read> {
        self.bindings.get(name)
    }

    /// The bindings the expression reads, by name.
    pub fn bindings(&self) -> &BTreeMap<String, Read> {
        &self.bindings
    }

    /// Whether the expression may read any binding whole, as `$eval` can: the application
    /// must then keep all its bindings.
    pub fn every_binding(&self) -> bool {
        self.every_binding
    }

    /// What either `Reads` reads: the input and each binding unite their `Read`s, and the
    /// *every binding* flags are or-ed. An input cut by the union of several expressions'
    /// `Reads` gives each of them its result.
    pub fn union(&self, other: &Reads) -> Reads {
        let mut reads = self.clone();
        if let Some(theirs) = &other.input {
            match &mut reads.input {
                Some(mine) => mine.unite(theirs),
                none => *none = Some(theirs.clone()),
            }
        }
        for (name, read) in &other.bindings {
            match reads.bindings.get_mut(name) {
                Some(own) => own.unite(read),
                None => {
                    reads.bindings.insert(name.clone(), read.clone());
                }
            }
        }
        reads.every_binding |= other.every_binding;
        reads
    }
}

// ---------------------------------------------------------------------------------------------
// The analysis: an abstract evaluation of the processed tree (ADR 0016, decision 3).

/// Where a value comes from: the input or a binding, followed by field names.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Place {
    root: Root,
    path: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Root {
    Input,
    Binding(String),
}

impl Place {
    /// The name of the binding whose value this is, which may be a host function.
    fn binding(&self) -> Option<&str> {
        match &self.root {
            Root::Binding(name) if self.path.is_empty() => Some(name),
            _ => None,
        }
    }
}

/// The places of an analysed value; none for literals and computed values.
type Places = Vec<Place>;

struct Scope {
    parent: Option<usize>,
    vars: HashMap<String, Var>,
}

struct Var {
    /// The union of the places of every value assigned to the variable.
    places: Places,
    /// When the variable was first assigned, on the clock of [`Analysis::seq`].
    seq: usize,
}

/// A function body analysed once all the code around its definition has been, so that it
/// sees every value its variables can take.
enum Body<'n> {
    Lambda(&'n LambdaDef),
    Transform(&'n TransformDef),
}

struct Deferred<'n> {
    body: Body<'n>,
    /// The context of the definition.
    context: Places,
    scope: usize,
    frontier: Vec<(usize, usize)>,
}

/// An argument of a call: a node, or the value on the left of `~>`.
enum Arg<'n> {
    Node(&'n Node),
    Value(Places),
}

struct Analysis<'n> {
    /// The host functions marked as not reading their context, with their signatures.
    marked: HashMap<&'n str, Option<&'n Signature>>,
    input: Option<Read>,
    bindings: BTreeMap<String, Read>,
    /// The bindings the expression names, read whole if the analysis gives up.
    named: BTreeSet<String>,
    /// The variables some part of the expression binds.
    locals: BTreeSet<&'n str>,
    scopes: Vec<Scope>,
    queue: VecDeque<Deferred<'n>>,
    /// The scopes that held the definitions of the function bodies being analysed, with the
    /// clock at each definition. A function can be called before a variable of those scopes
    /// assigned after its definition is, and then reads the variable of an outer scope.
    frontier: Vec<(usize, usize)>,
    seq: usize,
    /// The analysis gave up: everything named is read whole.
    everything: bool,
    every_binding: bool,
    /// The contexts of calls to functions that are neither built-ins nor bindings.
    calls: Vec<Places>,
    /// A binding was used as a value, so a host function may be called through a variable or
    /// a computed value, and read the context of the call.
    escaped: bool,
    /// A function that takes its context when an argument is missing (a lambda with `-` in its
    /// signature, or such a built-in) may be called through a variable.
    context_functions: bool,
}

/// Analyses a processed tree, given the bindings it will be evaluated with.
pub(crate) fn analyse<'n>(ast: &'n Ast, bindings: &'n Bindings) -> Reads {
    let mut analysis = Analysis {
        marked: bindings
            .functions()
            .filter(|(_, function)| !function.reads_input())
            .map(|(name, function)| (name, function.signature()))
            .collect(),
        input: None,
        bindings: BTreeMap::new(),
        named: BTreeSet::new(),
        locals: BTreeSet::new(),
        scopes: vec![Scope {
            parent: None,
            vars: HashMap::new(),
        }],
        queue: VecDeque::new(),
        frontier: Vec::new(),
        seq: 0,
        everything: false,
        every_binding: false,
        calls: Vec::new(),
        escaped: false,
        context_functions: false,
    };
    analysis.survey(&ast.root);
    let input = vec![Place {
        root: Root::Input,
        path: Vec::new(),
    }];
    let result = analysis.eval(&ast.root, &input, 0);
    analysis.use_whole(&result);
    while let Some(deferred) = analysis.queue.pop_front() {
        analysis.body(deferred);
    }
    analysis.finish()
}

impl<'n> Analysis<'n> {
    /// Notes the variables the expression binds, and gives up on `$eval`, which can read any
    /// variable of the environment where it is called (decision 7).
    fn survey(&mut self, root: &'n Node) {
        root.walk(&mut |node| {
            match &node.kind {
                NodeKind::Variable { name } if name == "eval" => {
                    self.everything = true;
                    self.every_binding = true;
                }
                NodeKind::Bind { lhs, .. } => {
                    if let NodeKind::Variable { name } = &lhs.kind {
                        self.locals.insert(name.as_str());
                    }
                }
                NodeKind::Lambda(def) => self.locals.extend(def.params.iter().map(String::as_str)),
                _ => {}
            }
            let attrs = &node.attrs;
            for name in [&attrs.focus, &attrs.index, &attrs.ancestor_label]
                .into_iter()
                .flatten()
            {
                self.locals.insert(name.as_str());
            }
            for stage in &attrs.stages {
                if let Stage::Index { var, .. } = stage {
                    self.locals.insert(var.as_str());
                }
            }
        });
    }

    fn body(&mut self, deferred: Deferred<'n>) {
        self.frontier = deferred.frontier;
        let scope = self.new_scope(deferred.scope);
        match deferred.body {
            Body::Lambda(def) => {
                if def
                    .signature
                    .as_ref()
                    .is_some_and(|s| s.params.iter().any(|p| p.context))
                {
                    self.context_functions = true;
                }
                let seq = self.next_seq();
                for param in &def.params {
                    self.bind(scope, param, Vec::new(), seq);
                }
                self.used(&def.body, &deferred.context, scope);
            }
            Body::Transform(def) => {
                self.used(&def.pattern, &[], scope);
                self.used(&def.update, &[], scope);
                if let Some(delete) = &def.delete {
                    self.used(delete, &[], scope);
                }
            }
        }
    }

    fn finish(mut self) -> Reads {
        if self.escaped || self.context_functions {
            for context in std::mem::take(&mut self.calls) {
                self.use_whole(&context);
            }
        }
        if self.everything {
            self.input = Some(Read::Whole);
            for name in &self.named {
                self.bindings.insert(name.clone(), Read::Whole);
            }
        }
        Reads {
            input: self.input,
            bindings: self.bindings,
            every_binding: self.every_binding,
        }
    }

    // -----------------------------------------------------------------------------------------
    // Reads, scopes and places

    fn next_seq(&mut self) -> usize {
        self.seq += 1;
        self.seq
    }

    /// Records that a place is needed, or needed whole.
    fn mark(&mut self, place: &Place, whole: bool) {
        let mut read = match &place.root {
            Root::Input => self.input.get_or_insert_with(Read::needed),
            Root::Binding(name) => self
                .bindings
                .entry(name.clone())
                .or_insert_with(Read::needed),
        };
        for name in &place.path {
            match read {
                Read::Whole => return,
                Read::Fields(fields) => {
                    read = fields.entry(name.clone()).or_insert_with(Read::needed);
                }
            }
        }
        if whole {
            *read = Read::Whole;
        }
    }

    fn need(&mut self, places: &[Place]) {
        for place in places {
            self.mark(place, false);
        }
    }

    /// Uses a value: its places are read whole.
    fn use_whole(&mut self, places: &[Place]) {
        for place in places {
            if let Some(name) = place.binding() {
                match self.marked.get(name) {
                    // A marked host function reads the context of a call only through its
                    // signature, as a lambda does.
                    Some(signature) => {
                        self.context_functions |= signature.is_some_and(takes_context)
                    }
                    None => self.escaped = true,
                }
            }
            self.mark(place, true);
        }
    }

    /// Adds `from` to `into`; past [`MAX_PLACES`], uses them all.
    fn join(&mut self, into: &mut Places, from: Places) {
        for place in from {
            if !into.contains(&place) {
                into.push(place);
            }
        }
        if into.len() > MAX_PLACES {
            let all = std::mem::take(into);
            self.use_whole(&all);
        }
    }

    /// The places of a field of a value, each needed.
    fn field(&mut self, context: &[Place], name: &str) -> Places {
        let mut out = Vec::with_capacity(context.len());
        for place in context {
            if place.path.len() >= MAX_DEPTH {
                self.mark(place, true);
                continue;
            }
            let mut place = place.clone();
            place.path.push(name.to_owned());
            self.mark(&place, false);
            out.push(place);
        }
        out
    }

    fn new_scope(&mut self, parent: usize) -> usize {
        self.scopes.push(Scope {
            parent: Some(parent),
            vars: HashMap::new(),
        });
        self.scopes.len() - 1
    }

    fn bind(&mut self, scope: usize, name: &str, places: Places, seq: usize) {
        let var = self.scopes[scope].vars.remove(name);
        let var = match var {
            Some(mut var) => {
                self.join(&mut var.places, places);
                var
            }
            None => {
                let mut var = Var {
                    places: Vec::new(),
                    seq,
                };
                self.join(&mut var.places, places);
                var
            }
        };
        self.scopes[scope].vars.insert(name.to_owned(), var);
    }

    /// The places a variable of the expression may hold, and whether it was found in a scope,
    /// and whether it is surely bound there. A function body analysed after its definition
    /// also reaches past a variable assigned after the definition, which a call may precede.
    fn lookup(&self, name: &str, scope: usize) -> (Places, bool, bool) {
        let mut out: Places = Vec::new();
        let mut found = false;
        let mut threshold: Option<usize> = None;
        let mut current = Some(scope);
        while let Some(id) = current {
            for &(at, seq) in &self.frontier {
                if at == id {
                    threshold = Some(threshold.map_or(seq, |t| t.min(seq)));
                }
            }
            if let Some(var) = self.scopes[id].vars.get(name) {
                found = true;
                for place in &var.places {
                    if !out.contains(place) {
                        out.push(place.clone());
                    }
                }
                if threshold.is_none_or(|t| var.seq < t) {
                    return (out, true, true);
                }
            }
            current = self.scopes[id].parent;
        }
        (out, found, false)
    }

    /// The built-in a call names, unless a variable of the expression hides it.
    fn builtin(&self, procedure: &Node, scope: usize) -> Option<&'static Native> {
        match &procedure.kind {
            NodeKind::Variable { name } if is_plain(procedure) && !self.lookup(name, scope).1 => {
                functions::native(name)
            }
            _ => None,
        }
    }

    // -----------------------------------------------------------------------------------------
    // Nodes

    fn eval(&mut self, node: &'n Node, context: &[Place], scope: usize) -> Places {
        crate::stack::grow(|| self.node(node, context, scope))
    }

    /// Analyses a node whose value is used.
    fn used(&mut self, node: &'n Node, context: &[Place], scope: usize) {
        let value = self.eval(node, context, scope);
        self.use_whole(&value);
    }

    fn node(&mut self, node: &'n Node, context: &[Place], scope: usize) -> Places {
        let mut value = match &node.kind {
            NodeKind::Path { steps, .. } => self.path(node, steps, context, scope),
            NodeKind::Binary { lhs, rhs, .. } => {
                self.used(lhs, context, scope);
                self.used(rhs, context, scope);
                Vec::new()
            }
            NodeKind::Negate { expr } => {
                self.used(expr, context, scope);
                Vec::new()
            }
            NodeKind::Array { items } => {
                let mut value = Vec::new();
                for item in items {
                    let places = self.eval(item, context, scope);
                    self.join(&mut value, places);
                }
                value
            }
            NodeKind::Object { pairs } => {
                self.group(pairs, context, scope);
                Vec::new()
            }
            NodeKind::Name { name } => self.field(context, name),
            NodeKind::Wildcard | NodeKind::Descendant => {
                self.use_whole(context);
                Vec::new()
            }
            NodeKind::Parent { label, .. } => {
                let (places, found, _) = self.lookup(label, scope);
                if !found {
                    self.everything = true;
                }
                places
            }
            NodeKind::Condition {
                condition,
                then,
                otherwise,
            } => {
                self.used(condition, context, scope);
                let mut value = self.eval(then, context, scope);
                if let Some(otherwise) = otherwise {
                    let places = self.eval(otherwise, context, scope);
                    self.join(&mut value, places);
                }
                value
            }
            NodeKind::Fallback { op, lhs, rhs } => {
                let mut value = self.eval(lhs, context, scope);
                match op {
                    FallbackOp::Default => self.use_whole(&value),
                    FallbackOp::Coalesce => self.need(&value),
                }
                let places = self.eval(rhs, context, scope);
                self.join(&mut value, places);
                value
            }
            NodeKind::Block { exprs, frame } => {
                let scope = if *frame { self.new_scope(scope) } else { scope };
                let mut value = Vec::new();
                for expr in exprs {
                    value = self.eval(expr, context, scope);
                }
                value
            }
            NodeKind::Bind { lhs, rhs } => {
                // A function assigned as it is defined sees its own variable.
                let early = matches!(&rhs.kind, NodeKind::Lambda(def) if !def.thunk);
                let mut seq = self.next_seq();
                let value = self.eval(rhs, context, scope);
                if !early {
                    seq = self.next_seq();
                }
                if let NodeKind::Variable { name } = &lhs.kind {
                    self.bind(scope, name, value.clone(), seq);
                }
                value
            }
            NodeKind::Function { procedure, args } => {
                let args = args.iter().map(Arg::Node).collect();
                self.call(procedure, args, context, scope)
            }
            NodeKind::Partial { procedure, args } => {
                self.used(procedure, context, scope);
                for arg in args {
                    self.used(arg, context, scope);
                }
                Vec::new()
            }
            NodeKind::Variable { name } => self.variable(name, context, scope),
            // A tail call, evaluated where it stands. The trampoline makes the call alone, as
            // jsonata-js does: a group-by on it is not applied, and the call's value is the
            // result.
            NodeKind::Lambda(def) if def.thunk => match &def.body.kind {
                NodeKind::Function { procedure, args } => {
                    let args = args.iter().map(Arg::Node).collect();
                    self.call(procedure, args, context, scope)
                }
                _ => self.eval(&def.body, context, scope),
            },
            NodeKind::Lambda(def) => {
                self.defer(Body::Lambda(def), context, scope);
                Vec::new()
            }
            NodeKind::Transform(def) => {
                self.defer(Body::Transform(def), &[], scope);
                Vec::new()
            }
            NodeKind::Apply { lhs, rhs } => self.apply(lhs, rhs, context, scope),
            NodeKind::String { .. }
            | NodeKind::Number { .. }
            | NodeKind::Bool { .. }
            | NodeKind::Null
            | NodeKind::Regex { .. }
            | NodeKind::Sort { .. }
            | NodeKind::Placeholder
            | NodeKind::Error { .. } => Vec::new(),
        };
        for stage in &node.attrs.predicate {
            if let Stage::Filter { expr, .. } = stage {
                self.filter(expr, &value, scope);
            }
        }
        if !matches!(node.kind, NodeKind::Path { .. })
            && let Some(group) = &node.attrs.group
        {
            self.group(&group.pairs, &value, scope);
            value = Vec::new();
        }
        value
    }

    fn path(
        &mut self,
        node: &'n Node,
        steps: &'n [Node],
        context: &[Place],
        scope: usize,
    ) -> Places {
        // A path that binds `@`, `#` or `%` labels has a scope of its own, unless it gives its
        // tuples to an enclosing path, which then sees its variables.
        let scope = if !node.attrs.tuple && steps.iter().any(|s| s.attrs.tuple) {
            self.new_scope(scope)
        } else {
            scope
        };
        let mut context = context.to_vec();
        for (ii, step) in steps.iter().enumerate() {
            // A step is evaluated for each item of its context, except a variable that starts
            // the path, evaluated once.
            if ii > 0 || !matches!(step.kind, NodeKind::Variable { .. }) {
                self.need(&context);
            }
            let value = match &step.kind {
                NodeKind::Sort { terms } => {
                    for term in terms {
                        self.used(&term.expr, &context, scope);
                    }
                    context.clone()
                }
                _ => self.eval(step, &context, scope),
            };
            if let Some(label) = &step.attrs.ancestor_label {
                let seq = self.next_seq();
                self.bind(scope, label, context.clone(), seq);
            }
            match &step.attrs.focus {
                Some(focus) => {
                    // A tuple for each item of the value, whether its variable is used or not.
                    self.need(&value);
                    let seq = self.next_seq();
                    self.bind(scope, focus, value, seq);
                }
                None => context = value,
            }
            if let Some(index) = &step.attrs.index {
                let seq = self.next_seq();
                self.bind(scope, index, Vec::new(), seq);
            }
            for stage in &step.attrs.stages {
                match stage {
                    Stage::Filter { expr, .. } => self.filter(expr, &context, scope),
                    Stage::Index { var, .. } => {
                        let seq = self.next_seq();
                        self.bind(scope, var, Vec::new(), seq);
                    }
                }
            }
        }
        if let Some(group) = &node.attrs.group {
            self.group(&group.pairs, &context, scope);
            return Vec::new();
        }
        context
    }

    /// A predicate, evaluated with each item of `value` as its context. A number selects
    /// items by position.
    fn filter(&mut self, predicate: &'n Node, value: &[Place], scope: usize) {
        self.need(value);
        if !matches!(predicate.kind, NodeKind::Number { .. }) {
            self.used(predicate, value, scope);
        }
    }

    /// An object constructor or a group-by: keys with each item as their context, values with
    /// each group.
    fn group(&mut self, pairs: &'n [(Node, Node)], context: &[Place], scope: usize) {
        self.need(context);
        for (key, value) in pairs {
            self.used(key, context, scope);
            self.used(value, context, scope);
        }
    }

    fn variable(&mut self, name: &str, context: &[Place], scope: usize) -> Places {
        if name.is_empty() {
            return context.to_vec();
        }
        let (mut places, found, bound) = self.lookup(name, scope);
        if !bound {
            let unbound = if name == "$" {
                Some(Root::Input)
            } else if let Some(native) = functions::native(name) {
                // A built-in taken as a value may be called with its context.
                if has_context_param(native) {
                    self.context_functions = true;
                }
                None
            } else if !found && self.locals.contains(name) {
                // A variable bound somewhere the analysis does not see from here.
                self.everything = true;
                None
            } else {
                self.named.insert(name.to_owned());
                Some(Root::Binding(name.to_owned()))
            };
            if let Some(root) = unbound {
                let place = Place {
                    root,
                    path: Vec::new(),
                };
                self.join(&mut places, vec![place]);
            }
        }
        let mut value = Vec::new();
        self.join(&mut value, places);
        value
    }

    fn defer(&mut self, body: Body<'n>, context: &[Place], scope: usize) {
        let seq = self.next_seq();
        let mut frontier = self.frontier.clone();
        frontier.push((scope, seq));
        self.queue.push_back(Deferred {
            body,
            context: context.to_vec(),
            scope,
            frontier,
        });
    }

    // -----------------------------------------------------------------------------------------
    // Calls (decision 5)

    fn arg(&mut self, arg: &Arg<'n>, context: &[Place], scope: usize) -> Places {
        match arg {
            Arg::Node(node) => self.eval(node, context, scope),
            Arg::Value(places) => places.clone(),
        }
    }

    fn apply(&mut self, lhs: &'n Node, rhs: &'n Node, context: &[Place], scope: usize) -> Places {
        let value = self.eval(lhs, context, scope);
        if let NodeKind::Function { procedure, args } = &rhs.kind {
            let mut all = vec![Arg::Value(value)];
            all.extend(args.iter().map(Arg::Node));
            return self.call(procedure, all, context, scope);
        }
        // `x ~> $f` applies `$f` without a context.
        if let Some(native) = self.builtin(rhs, scope) {
            return self.native(native, vec![Arg::Value(value)], &[], scope);
        }
        let function = self.eval(rhs, context, scope);
        self.use_whole(&function);
        self.use_whole(&value);
        Vec::new()
    }

    fn call(
        &mut self,
        procedure: &'n Node,
        args: Vec<Arg<'n>>,
        context: &[Place],
        scope: usize,
    ) -> Places {
        if let Some(native) = self.builtin(procedure, scope) {
            return self.native(native, args, context, scope);
        }
        // `name()` for `$name()` fails with T1005 when a variable `name` is true.
        if let NodeKind::Path { steps, .. } = &procedure.kind
            && let Some(NodeKind::Name { name }) = steps.first().map(|s| &s.kind)
        {
            let variable = self.variable(name, context, scope);
            self.mark_all(&variable);
        }
        let function = self.eval(procedure, context, scope);
        self.mark_all(&function);
        // A host function can read the context of its call, unless it is marked.
        let mut reads_context = false;
        for name in function.iter().filter_map(Place::binding) {
            match self.marked.get(name) {
                Some(Some(signature)) => self.signature_context(signature, args.len(), context),
                Some(None) => {}
                None => reads_context = true,
            }
        }
        // `$name(...)`, naming a marked function that no variable hides, cannot call an escaped
        // binding.
        let direct = matches!(&procedure.kind, NodeKind::Variable { name }
            if is_plain(procedure)
                && self.marked.contains_key(name.as_str())
                && !self.lookup(name, scope).1);
        if reads_context {
            self.use_whole(context);
        } else if !direct {
            self.calls.push(context.to_vec());
        }
        for arg in &args {
            let places = self.arg(arg, context, scope);
            self.use_whole(&places);
        }
        Vec::new()
    }

    /// Reads places whole without letting a binding escape: a function called where it is
    /// named.
    fn mark_all(&mut self, places: &[Place]) {
        for place in places {
            self.mark(place, true);
        }
    }

    fn native(
        &mut self,
        native: &'static Native,
        args: Vec<Arg<'n>>,
        context: &[Place],
        scope: usize,
    ) -> Places {
        self.native_context(native, args.len(), context);
        let name = native.name;
        if let [array, Arg::Node(function), rest @ ..] = args.as_slice()
            && matches!(name, "map" | "filter" | "single" | "sort" | "reduce")
            && let Some(def) = inline_lambda(function)
        {
            return self.higher_order(name, array, def, rest, context, scope);
        }
        if let ("lookup", [object, Arg::Node(key)]) = (name, args.as_slice())
            && let Some(key) = string_literal(key)
        {
            let places = self.arg(object, context, scope);
            return self.field(&places, key);
        }
        match (name, args.as_slice()) {
            ("count" | "exists" | "type", [first, rest @ ..]) => {
                let places = self.arg(first, context, scope);
                self.need(&places);
                self.use_args(rest, context, scope);
                Vec::new()
            }
            ("append" | "reverse" | "shuffle" | "zip" | "merge", _) => {
                let mut value = Vec::new();
                for arg in &args {
                    let places = self.arg(arg, context, scope);
                    // `$merge` fails on an item that is not an object, used or not.
                    if name == "merge" {
                        self.need(&places);
                    }
                    self.join(&mut value, places);
                }
                value
            }
            ("single", [array]) => {
                let places = self.arg(array, context, scope);
                self.need(&places);
                places
            }
            _ => {
                self.use_args(&args, context, scope);
                Vec::new()
            }
        }
    }

    fn use_args(&mut self, args: &[Arg<'n>], context: &[Place], scope: usize) {
        for arg in args {
            let places = self.arg(arg, context, scope);
            self.use_whole(&places);
        }
    }

    fn native_context(&mut self, native: &Native, count: usize, context: &[Place]) {
        if let Some(signature) = &native.signature {
            self.signature_context(signature, count, context);
        }
    }

    /// A function whose first parameter takes the context (`-`) reads it when the arguments
    /// can leave that parameter empty: whole if it accepts objects or arrays, else as a scalar,
    /// which a cut keeps. A later parameter that takes it reads it whole.
    fn signature_context(&mut self, signature: &Signature, count: usize, context: &[Place]) {
        let Some((first, rest)) = signature.params.split_first() else {
            return;
        };
        if rest.iter().any(|param| param.context) {
            self.use_whole(context);
            return;
        }
        let Some(pattern) = first.context_pattern.as_deref().filter(|_| first.context) else {
            return;
        };
        let (min, max) = rest.iter().fold((0, Some(0)), |(min, max), param| {
            let (low, high) = repetition(&param.pattern);
            (min + low, max.zip(high).map(|(m, h)| m + h))
        });
        let fits = max.is_none_or(|max| count <= max);
        // A parameter that accepts any value takes the first argument, unless the others
        // cannot match the rest.
        let any = "asnblfom".chars().all(|c| pattern.contains(c));
        let skipped = if any {
            count < 1 + min || (count >= 2 && fits)
        } else {
            fits
        };
        if skipped {
            if pattern.contains(['a', 'o']) {
                self.use_whole(context);
            } else {
                self.need(context);
            }
        }
    }

    /// `$map`, `$filter`, `$single`, `$sort` and `$reduce` with a lambda written as their
    /// argument: its body is analysed at the call, its parameters given the array's places.
    fn higher_order(
        &mut self,
        name: &str,
        array: &Arg<'n>,
        def: &'n LambdaDef,
        rest: &[Arg<'n>],
        context: &[Place],
        scope: usize,
    ) -> Places {
        let items = self.arg(array, context, scope);
        self.need(&items);
        self.use_args(rest, context, scope);
        // Without an initial value, `$reduce` starts from the first item, and returns it when
        // it is the only one.
        let from_items = name == "reduce" && rest.is_empty();
        let lambda = self.new_scope(scope);
        let seq = self.next_seq();
        for (ii, param) in def.params.iter().enumerate() {
            let places = match (name, ii) {
                ("map" | "filter" | "single", 0 | 2) | ("sort", 0 | 1) | ("reduce", 1 | 3) => {
                    items.clone()
                }
                ("reduce", 0) if from_items => items.clone(),
                _ => Vec::new(),
            };
            self.bind(lambda, param, places, seq);
        }
        let body = self.eval(&def.body, context, lambda);
        match name {
            "map" => body,
            "reduce" if !from_items => {
                self.use_whole(&body);
                Vec::new()
            }
            _ => {
                self.use_whole(&body);
                items
            }
        }
    }
}

/// A node evaluated as it is, without predicates, stages or a group.
fn is_plain(node: &Node) -> bool {
    node.attrs.predicate.is_empty() && node.attrs.stages.is_empty() && node.attrs.group.is_none()
}

/// A lambda written as an argument.
fn inline_lambda(node: &Node) -> Option<&LambdaDef> {
    match &node.kind {
        NodeKind::Lambda(def) if !def.thunk && is_plain(node) => Some(def),
        _ => None,
    }
}

/// A string written as an argument.
fn string_literal(node: &Node) -> Option<&str> {
    match &node.kind {
        NodeKind::String { value } if is_plain(node) => Some(value),
        _ => None,
    }
}

fn has_context_param(native: &Native) -> bool {
    native.signature.as_ref().is_some_and(takes_context)
}

fn takes_context(signature: &Signature) -> bool {
    signature.params.iter().any(|p| p.context)
}

/// How many arguments a parameter pattern takes: `?` none or one, `+` one or more.
fn repetition(pattern: &str) -> (usize, Option<usize>) {
    if pattern.ends_with('?') {
        (0, Some(1))
    } else if pattern.ends_with('+') {
        (1, None)
    } else {
        (1, Some(1))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::Expression;

    fn reads(source: &str) -> JsonValue {
        let expr = Expression::compile(source).unwrap();
        serde_json::to_value(expr.reads()).unwrap()
    }

    #[test]
    fn paths_and_filters_read_their_fields() {
        assert_eq!(
            reads(r#"$sum(items[type = "book"].(price * quantity)) * (1 - $discount)"#),
            json!({
                "input": {"items": {"type": true, "price": true, "quantity": true}},
                "bindings": {"discount": true}
            })
        );
        assert_eq!(reads("1 + 2"), json!({"bindings": {}}));
        assert_eq!(reads("$"), json!({"input": true, "bindings": {}}));
    }

    /// One expression for each row of the tables of ADR 0016, with the input it reads.
    #[test]
    fn each_construct_reads_what_its_row_says() {
        let cases = [
            ("'s' & 1 & true & null & /re/", json!(null)),
            ("$$.a", json!({"a": true})),
            ("a.($x := b; $x.c)", json!({"a": {"b": {"c": true}}})),
            ("a.*", json!({"a": true})),
            ("a.**.b", json!({"a": true})),
            ("-a + b * c", json!({"a": true, "b": true, "c": true})),
            (
                "c ? t.x : e.y",
                json!({"c": true, "t": {"x": true}, "e": {"y": true}}),
            ),
            ("(a ?: b).x", json!({"a": true, "b": {"x": true}})),
            ("(a ?? b).x", json!({"a": {"x": true}, "b": {"x": true}})),
            ("[a, b].x", json!({"a": {"x": true}, "b": {"x": true}})),
            ("{'k': a.b}", json!({"a": {"b": true}})),
            (
                "items{type: $sum(price)}",
                json!({"items": {"type": true, "price": true}}),
            ),
            (
                "items^(>price).name",
                json!({"items": {"price": true, "name": true}}),
            ),
            ("items#$i[$i < 2].name", json!({"items": {"name": true}})),
            ("items[0].name", json!({"items": {"name": true}})),
            ("$lookup(a, 'b').c", json!({"a": {"b": {"c": true}}})),
            ("$lookup(a, k)", json!({"a": true, "k": true})),
            (
                "$append(a, b).c",
                json!({"a": {"c": true}, "b": {"c": true}}),
            ),
            (
                "$filter(items, function($v) { $v.ok }).name",
                json!({"items": {"ok": true, "name": true}}),
            ),
            (
                "$sort(items, function($l, $r) { $l.n > $r.n }).m",
                json!({"items": {"n": true, "m": true}}),
            ),
            ("$single(items).name", json!({"items": {"name": true}})),
            (
                "$reduce(items, function($a, $v) { $a + $v.n }, 0)",
                json!({"items": {"n": true}}),
            ),
            ("$map(items, $string)", json!({"items": true})),
            ("$keys(a)", json!({"a": true})),
            ("name.$uppercase()", json!({"name": {}})),
            ("a.$string()", json!({"a": true})),
            ("a ~> $count", json!({"a": {}})),
            ("($f := function($v) { $v.x }; $f(a))", json!({"a": true})),
            (
                "($f := function() { b }; a.$f())",
                json!({"a": {}, "b": true}),
            ),
            // A partial application of a built-in that takes its context.
            ("$substring(?, 1)(a)", json!(true)),
            ("$join(?, ',')(a)", json!({"a": true})),
            ("a ~> |b|{'c': 1}|", json!({"a": true})),
        ];
        for (source, input) in cases {
            let expected = match input {
                JsonValue::Null => json!({"bindings": {}}),
                input => json!({"input": input, "bindings": {}}),
            };
            assert_eq!(reads(source), expected, "{source}");
        }
    }

    #[test]
    fn counting_needs_no_fields() {
        assert_eq!(
            reads("$count(items) + $exists(a.b)"),
            json!({"input": {"items": {}, "a": {"b": {}}}, "bindings": {}})
        );
    }

    #[test]
    fn parents_and_focus_follow_their_places() {
        assert_eq!(
            reads(r#"Order.Product.{"order": %.OrderID}"#),
            json!({"input": {"Order": {"OrderID": true, "Product": {}}}, "bindings": {}})
        );
        assert_eq!(
            reads("library.loans@$l.books[$l.isbn = isbn].title"),
            json!({
                "input": {"library": {
                    "loans": {"isbn": true},
                    "books": {"isbn": true, "title": true}
                }},
                "bindings": {}
            })
        );
    }

    #[test]
    fn inline_lambdas_are_followed() {
        assert_eq!(
            reads("$map(items, function($v) { $v.price * 2 })"),
            json!({"input": {"items": {"price": true}}, "bindings": {}})
        );
        assert_eq!(
            reads("$reduce(items, function($a, $v) { $a + $v.n }).m"),
            json!({"input": {"items": true}, "bindings": {}})
        );
    }

    #[test]
    fn host_functions_read_their_context() {
        assert_eq!(
            reads("$tax(price)"),
            json!({"input": true, "bindings": {"tax": true}})
        );
        assert_eq!(
            reads("order.price.$tax()"),
            json!({"input": {"order": {"price": true}}, "bindings": {"tax": true}})
        );
        // A local function reads its context only if a binding could have become it.
        assert_eq!(
            reads("($f := function($x) { $x * 2 }; $f(price))"),
            json!({"input": {"price": true}, "bindings": {}})
        );
        assert_eq!(
            reads("($f := $sum($g); $h := $g; $h(price))")["input"],
            json!(true)
        );
    }

    #[test]
    fn marked_host_functions_read_their_arguments() {
        use crate::{Bindings, HostFunction};
        let bindings = Bindings::new()
            .function(
                "rate",
                HostFunction::new(1, |_, _| Ok(None)).without_input(),
            )
            .function(
                "tax",
                HostFunction::with_signature("<n-n?:n>", |_, _| Ok(None))
                    .unwrap()
                    .without_input(),
            )
            .function("plain", HostFunction::new(1, |_, _| Ok(None)));
        let input = |source: &str| {
            let expr = Expression::compile(source).unwrap();
            serde_json::to_value(expr.reads_with(&bindings).input()).unwrap()
        };
        assert_eq!(input("$rate(country)"), json!({"country": true}));
        assert_eq!(reads("$rate(country)")["input"], json!(true));
        assert_eq!(input("$rate(country) + $plain(1)"), json!(true));
        // A binding used as a value may be a host function, called through a variable.
        assert_eq!(input("$rate(country) + $kind"), json!({"country": true}));
        assert_eq!(input("($f := $kind; $rate(country) + $f())"), json!(true));
        // Taken as a value, a marked function still reads only its arguments.
        assert_eq!(input("($f := $rate; $f(price))"), json!({"price": true}));
        // The context a signature may take: a scalar here.
        assert_eq!(input("$tax(price)"), json!({"price": true}));
        assert_eq!(
            input("$tax(price, rate)"),
            json!({"price": true, "rate": true})
        );
        assert_eq!(input("order.$tax()"), json!({"order": {}}));
        assert_eq!(input("($f := $tax; $f(price))"), json!({"price": true}));
        // Passed on, a function that takes its context may be called with any context.
        assert_eq!(
            input("($g := function($h) { $h() }; a.$g($tax))"),
            json!(true)
        );
        assert_eq!(
            input("($g := function($h) { $h() }; a.$g($rate))"),
            json!({"a": {}})
        );
    }

    #[test]
    fn eval_reads_everything() {
        assert_eq!(
            reads("$eval($e) + $x.y"),
            json!({"input": true, "bindings": {"e": true, "x": true}, "everyBinding": true})
        );
    }

    #[test]
    fn a_function_sees_variables_assigned_after_it() {
        // `$f` may run before `$y` is assigned, and then reads the binding `y`.
        assert_eq!(
            reads("($f := function() { $y.c }; $f(); $y := a; $f())"),
            json!({"input": {"a": {"c": true}}, "bindings": {"y": {"c": true}}})
        );
    }

    #[test]
    fn deep_paths_are_read_whole_at_the_cap() {
        let source = vec!["a"; MAX_DEPTH + 10].join(".");
        let json = reads(&source);
        let mut read = &json["input"];
        for _ in 0..MAX_DEPTH {
            read = &read["a"];
        }
        assert_eq!(read, &json!(true));
        let text = serde_json::to_string(&json).unwrap();
        let back: Reads = serde_json::from_str(&text).unwrap();
        assert_eq!(serde_json::to_value(back).unwrap(), json);
    }

    #[test]
    fn the_json_form_round_trips_and_is_checked() {
        let reads: Reads = serde_json::from_str(
            r#"{"input": {"a": {"b": true, "c": {}}}, "bindings": {"x": true}}"#,
        )
        .unwrap();
        assert_eq!(reads.binding("x"), Some(&Read::Whole));
        assert!(!reads.every_binding());
        assert!(serde_json::from_str::<Reads>(r#"{"input": false}"#).is_err());
        assert!(serde_json::from_str::<Reads>(r#"{"inputs": true}"#).is_err());
        let deep = format!(
            "{}{}",
            "{\"a\":".repeat(MAX_DEPTH + 2),
            "}".repeat(MAX_DEPTH + 2)
        );
        assert!(serde_json::from_str::<Read>(&deep).is_err());
    }

    #[test]
    fn cutting_keeps_what_is_read() {
        let read: Read = serde_json::from_value(json!({"a": {"b": true}, "c": {}})).unwrap();
        assert_eq!(
            read.cut(
                &json!({"z": 1, "a": [{"b": {"x": 1}, "y": 2}, [{"b": 3}], 4], "c": {"d": 1}})
            ),
            json!({"a": [{"b": {"x": 1}}, [{"b": 3}], 4], "c": {}})
        );
        assert_eq!(read.cut(&json!("s")), json!("s"));
    }

    #[test]
    fn union_merges_and_whole_absorbs() {
        let a: Reads =
            serde_json::from_value(json!({"input": {"a": {"b": true}}, "bindings": {"x": {}}}))
                .unwrap();
        let b: Reads = serde_json::from_value(
            json!({"input": {"a": {"c": true}, "d": true}, "bindings": {"x": true}, "everyBinding": true}),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(a.union(&b)).unwrap(),
            json!({
                "input": {"a": {"b": true, "c": true}, "d": true},
                "bindings": {"x": true},
                "everyBinding": true
            })
        );
        assert_eq!(a.union(&b), b.union(&a));
        assert_eq!(a.union(&a), a);
    }
}
