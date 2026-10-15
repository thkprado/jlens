//! The run-time value model (see ADR 0003).
//!
//! JSONata distinguishes *undefined* from `null`, and *sequences* (results of path navigation,
//! which collapse when empty or singleton) from arrays. Arrays therefore carry flags, set and
//! read exactly where jsonata-js sets and reads the corresponding properties on its arrays.
//! Objects are shared and mutable (`Rc<RefCell<…>>`) because the transform operator updates the
//! objects that its pattern selects in place.

use std::cell::{Cell, OnceCell, Ref, RefCell, RefMut};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::rc::Rc;
use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::Value as JsonValue;

use crate::ast::{LambdaDef, Span, TransformDef};
use crate::error::{Error, Result};
use crate::eval::Env;
use crate::functions::Native;
use crate::host::HostFunction;
use crate::regex::Regex;

/// The fields of an object, in insertion order. Keys are shared: the objects of a converted
/// document that have the same key point to one string (see [`Keys`]).
pub(crate) type Map<'a> = IndexMap<Rc<str>, Value<'a>, foldhash::fast::RandomState>;

/// The keys met while converting one document, so that objects with the same keys (the items
/// of an array, typically) share them instead of allocating each.
#[derive(Default)]
pub(crate) struct Keys(HashSet<Rc<str>, foldhash::fast::RandomState>);

impl Keys {
    /// Past this many distinct keys, new keys are allocated without being remembered: a
    /// document whose keys are all different (identifiers used as keys) gains nothing from
    /// the table.
    const CAPACITY: usize = 1024;

    pub(crate) fn get(&mut self, key: &str) -> Rc<str> {
        if let Some(shared) = self.0.get(key) {
            return shared.clone();
        }
        let key: Rc<str> = key.into();
        if self.0.len() < Self::CAPACITY {
            self.0.insert(key.clone());
        }
        key
    }
}

/// What the lazy values of one evaluation share (ADR 0010): the table of their keys, and the
/// first error of a [`Source`] that failed to read a level.
#[derive(Default)]
pub(crate) struct Shared {
    keys: RefCell<Keys>,
    failed: Cell<bool>,
    failure: RefCell<Option<String>>,
}

impl Shared {
    pub(crate) fn new() -> Rc<Shared> {
        Rc::default()
    }

    /// Whether a level could not be read.
    #[inline]
    pub(crate) fn failed(&self) -> bool {
        self.failed.get()
    }

    /// The `U1004` error of the first level that could not be read.
    #[cold]
    #[inline(never)]
    pub(crate) fn failure(&self) -> Error {
        let failure = self.failure.borrow();
        Error::new("U1004").with("message", failure.as_deref().unwrap_or_default())
    }

    fn fail(&self, message: String) {
        self.failed.set(true);
        self.failure.borrow_mut().get_or_insert(message);
    }
}

/// The part of the input that a lazy array or object stands for, not read yet (ADR 0010). The
/// first access to the array's items or the object's map reads one level of it.
struct Pending<'a> {
    origin: Origin<'a>,
    shared: Rc<Shared>,
}

enum Origin<'a> {
    Json(&'a JsonValue),
    Source(Box<dyn Source<'a> + 'a>),
}

impl<'a> Pending<'a> {
    /// The JSON this stands for, when it is JSON.
    fn json(&self) -> Option<&'a JsonValue> {
        match self.origin {
            Origin::Json(json) => Some(json),
            Origin::Source(_) => None,
        }
    }

    /// Reads a level from a source into `level`. A source that fails leaves the level empty,
    /// and its error is recorded for the evaluation (ADR 0010, decision 6).
    fn read_source(&self, source: &dyn Source<'a>, level: Level<'_, 'a>) {
        let mut sink = Sink {
            level,
            shared: &self.shared,
            key: None,
        };
        if let Err(message) = source.read(&mut sink) {
            self.shared.fail(message);
            match sink.level {
                Level::Items(items) => items.clear(),
                Level::Fields(map) => map.clear(),
            }
        }
    }
}

/// A JSONata value.
#[derive(Clone, Default)]
pub(crate) enum Value<'a> {
    #[default]
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(Rc<str>),
    Array(Rc<Array<'a>>),
    Object(Rc<Object<'a>>),
    Function(Rc<Function<'a>>),
}

pub(crate) const SEQUENCE: u8 = 1;
pub(crate) const KEEP_SINGLETON: u8 = 2;
pub(crate) const OUTER_WRAPPER: u8 = 4;
pub(crate) const CONS: u8 = 8;
pub(crate) const TUPLE_STREAM: u8 = 16;

/// An array, with the flags jsonata-js attaches to its arrays.
pub(crate) struct Array<'a> {
    /// The items of an array built by the evaluation, or of a document; empty when the array
    /// is lazy.
    items: Vec<Value<'a>>,
    flags: Cell<u8>,
    /// The items of an array of the input, read on first access. They are kept apart so that
    /// the other arrays read a plain field.
    lazy: Option<Box<LazyItems<'a>>>,
}

struct LazyItems<'a> {
    pending: Pending<'a>,
    items: OnceCell<Vec<Value<'a>>>,
}

impl<'a> LazyItems<'a> {
    /// The items, read from the input on the first call.
    #[inline(never)]
    fn read(&self) -> &Vec<Value<'a>> {
        let pending = &self.pending;
        self.items.get_or_init(|| match &pending.origin {
            Origin::Json(JsonValue::Array(items)) => items
                .iter()
                .map(|v| Value::lazy(v, &pending.shared))
                .collect(),
            Origin::Json(_) => Vec::new(),
            Origin::Source(source) => {
                let mut items = Vec::new();
                pending.read_source(&**source, Level::Items(&mut items));
                items
            }
        })
    }
}

impl<'a> Array<'a> {
    /// The items, read from the input first if the array is lazy.
    #[inline]
    pub fn items(&self) -> &Vec<Value<'a>> {
        match &self.lazy {
            None => &self.items,
            Some(lazy) => lazy.read(),
        }
    }

    /// The input JSON this array stands for, when it has not been read.
    fn unread(&self) -> Option<&'a JsonValue> {
        match &self.lazy {
            Some(lazy) if lazy.items.get().is_none() => lazy.pending.json(),
            _ => None,
        }
    }

    /// Whether the array has not been read from the input yet. Such an array holds only values
    /// of the input.
    pub fn is_unread(&self) -> bool {
        self.lazy
            .as_ref()
            .is_some_and(|lazy| lazy.items.get().is_none())
    }

    pub fn has(&self, flag: u8) -> bool {
        self.flags.get() & flag != 0
    }

    pub fn set(&self, flag: u8) {
        self.flags.set(self.flags.get() | flag);
    }

    pub fn is_sequence(&self) -> bool {
        self.has(SEQUENCE)
    }

    pub fn is_tuple_stream(&self) -> bool {
        self.has(TUPLE_STREAM)
    }

    pub fn len(&self) -> usize {
        match self.unread() {
            Some(JsonValue::Array(items)) => items.len(),
            _ => self.items().len(),
        }
    }
}

/// An object. It is shared and mutable because the transform operator updates in place the
/// objects that its pattern selects.
pub(crate) struct Object<'a> {
    /// The fields; empty until a lazy object is read.
    map: RefCell<Map<'a>>,
    /// The input a lazy object stands for. It is a plain field, so that the other objects
    /// cost one test of it.
    pending: Option<Pending<'a>>,
    read: Cell<bool>,
}

impl<'a> Object<'a> {
    #[inline]
    fn cell(&self) -> &RefCell<Map<'a>> {
        if let Some(pending) = &self.pending
            && !self.read.get()
        {
            self.read(pending);
        }
        &self.map
    }

    /// Reads the fields of a lazy object into its map, which nothing borrows yet: every
    /// borrow goes through `cell`.
    #[cold]
    #[inline(never)]
    fn read(&self, pending: &Pending<'a>) {
        self.read.set(true);
        let Ok(mut map) = self.map.try_borrow_mut() else {
            return;
        };
        match &pending.origin {
            Origin::Json(JsonValue::Object(fields)) => {
                *map = Map::with_capacity_and_hasher(fields.len(), Default::default());
                let mut keys = pending.shared.keys.borrow_mut();
                for (k, v) in fields {
                    map.insert(keys.get(k), Value::lazy(v, &pending.shared));
                }
            }
            Origin::Json(_) => {}
            Origin::Source(source) => pending.read_source(&**source, Level::Fields(&mut map)),
        }
    }

    /// The fields, read from the input first if the object is lazy.
    #[inline]
    pub fn borrow(&self) -> Ref<'_, Map<'a>> {
        self.cell().borrow()
    }

    /// The fields, mutably, read from the input first if the object is lazy.
    #[inline]
    pub fn borrow_mut(&self) -> RefMut<'_, Map<'a>> {
        self.cell().borrow_mut()
    }

    /// The input JSON this object stands for, when it has not been read.
    fn unread(&self) -> Option<&'a JsonValue> {
        match &self.pending {
            Some(p) if !self.read.get() => p.json(),
            _ => None,
        }
    }

    /// Whether the object has not been read from the input yet. Such an object holds only
    /// values of the input.
    pub fn is_unread(&self) -> bool {
        self.pending.is_some() && !self.read.get()
    }
}

// Arrays and objects nested deeper than the native stack allows (built by a tail-recursive
// function, or read from a `Source`) must be dropped too. The drops of those that hold arrays
// or objects of their own count how deep they are nested in one another, and past
// `DROP_DEPTH` levels, the arrays and objects inside are dropped without recursion. The others,
// most of them, drop as usual.
impl Drop for Array<'_> {
    fn drop(&mut self) {
        let lazy = self.lazy.as_mut().and_then(|lazy| lazy.items.take());
        let read = lazy.as_deref().unwrap_or_default();
        if !self.items.iter().chain(read).any(owned) {
            return;
        }
        let items = std::mem::take(&mut self.items);
        nested_drop(|deep| {
            if deep {
                release(items.into_iter().chain(lazy.into_iter().flatten()));
            } else {
                drop((items, lazy));
            }
        });
    }
}

impl Drop for Object<'_> {
    fn drop(&mut self) {
        let map = self.map.get_mut();
        if !map.values().any(owned) {
            return;
        }
        // Moved out with the same hasher, which costs no allocation; clearing it in place would
        // reset its table first.
        let map = std::mem::replace(map, Map::with_hasher(map.hasher().clone()));
        nested_drop(|deep| {
            if deep {
                release(map.into_values());
            } else {
                drop(map);
            }
        });
    }
}

/// Whether a value is an array or object that nothing else holds, which dropping it drops.
fn owned(v: &Value<'_>) -> bool {
    match v {
        Value::Array(a) => Rc::strong_count(a) == 1,
        Value::Object(o) => Rc::strong_count(o) == 1,
        _ => false,
    }
}

/// How deep drops nest before the values inside are released from a list instead.
const DROP_DEPTH: usize = 100;

thread_local! {
    static DROPS: Cell<usize> = const { Cell::new(0) };
}

/// Runs `drop`, a level deeper in the drops of arrays and objects; it is told whether that is
/// past [`DROP_DEPTH`].
fn nested_drop(drop: impl FnOnce(bool)) {
    DROPS.with(|drops| {
        let depth = drops.get();
        drops.set(depth + 1);
        drop(depth >= DROP_DEPTH);
        drops.set(depth);
    });
}

/// Calls `f` with each value of `items` that is not an array, in order, entering the arrays
/// nested in them. The walk keeps its own stack, so arrays nested at any depth are walked.
pub(crate) fn for_each_leaf<'v, 'a, E>(
    items: &'v [Value<'a>],
    mut f: impl FnMut(&'v Value<'a>) -> std::result::Result<(), E>,
) -> std::result::Result<(), E> {
    let mut outer = Vec::new();
    let mut iter = items.iter();
    loop {
        match iter.next() {
            Some(Value::Array(a)) => outer.push(std::mem::replace(&mut iter, a.items().iter())),
            Some(leaf) => f(leaf)?,
            None => match outer.pop() {
                Some(rest) => iter = rest,
                None => return Ok(()),
            },
        }
    }
}

/// Drops values, emptying the arrays and objects that only they hold one at a time, from a
/// list of their own, rather than each inside the drop of its parent.
fn release<'a>(values: impl IntoIterator<Item = Value<'a>>) {
    let mut pending: Vec<Value<'a>> = values.into_iter().filter(owned).collect();
    while let Some(value) = pending.pop() {
        match value {
            Value::Array(a) => {
                if let Some(mut a) = Rc::into_inner(a) {
                    pending.extend(std::mem::take(&mut a.items).into_iter().filter(owned));
                    if let Some(items) = a.lazy.as_mut().and_then(|lazy| lazy.items.take()) {
                        pending.extend(items.into_iter().filter(owned));
                    }
                }
            }
            Value::Object(o) => {
                if let Some(mut o) = Rc::into_inner(o) {
                    let fields = std::mem::take(o.map.get_mut()).into_values();
                    pending.extend(fields.filter(owned));
                }
            }
            _ => {}
        }
    }
}

/// A function value.
pub(crate) enum Function<'a> {
    /// A JSONata `function(...)`, or a tail-call thunk.
    Lambda(Lambda<'a>),
    /// A built-in function.
    Native(&'static Native),
    /// A function of the host application.
    Host(HostFunction),
    /// A built-in or host function with some arguments supplied (`$f(?, 2)`); `None` marks a
    /// `?`.
    Partial {
        target: Rc<Function<'a>>,
        args: Vec<Option<Value<'a>>>,
    },
    /// The function produced by a transform expression.
    Transform {
        def: Arc<TransformDef>,
        env: Env<'a>,
        span: Span,
        position: usize,
    },
    /// A regular expression literal, which is callable as a matcher.
    Regex(Rc<Regex>),
    /// The `next()` function of a regex match object.
    RegexNext {
        re: Rc<Regex>,
        text: Rc<str>,
        last: usize,
    },
}

pub(crate) struct Lambda<'a> {
    pub def: Arc<LambdaDef>,
    /// The parameters still to be supplied, when fewer than `def.params` (partial application).
    pub params: Option<Rc<[String]>>,
    /// Whether arguments are checked against `def.signature`; partial applications are not.
    pub validate: bool,
    pub input: Value<'a>,
    pub env: Env<'a>,
}

impl<'a> Function<'a> {
    /// Number of parameters, used by higher-order functions to decide which arguments to pass.
    pub fn arity(&self) -> usize {
        match self {
            Function::Lambda(l) => l.params().len(),
            Function::Native(n) => n.arity,
            Function::Host(h) => h.arity(),
            Function::Partial { args, .. } => args.iter().filter(|a| a.is_none()).count(),
            Function::Transform { .. } => 1,
            Function::Regex(_) => 2,
            Function::RegexNext { .. } => 0,
        }
    }

    pub fn is_thunk(&self) -> bool {
        matches!(self, Function::Lambda(l) if l.def.thunk)
    }
}

impl<'a> Value<'a> {
    pub fn string(s: impl Into<Rc<str>>) -> Value<'a> {
        Value::String(s.into())
    }

    pub fn array(items: Vec<Value<'a>>, flags: u8) -> Value<'a> {
        Value::Array(Rc::new(Array {
            items,
            flags: Cell::new(flags),
            lazy: None,
        }))
    }

    /// A new sequence (jsonata-js `createSequence`).
    pub fn sequence(items: Vec<Value<'a>>) -> Value<'a> {
        Value::array(items, SEQUENCE)
    }

    pub fn object(map: Map<'a>) -> Value<'a> {
        Value::Object(Rc::new(Object {
            map: RefCell::new(map),
            pending: None,
            read: Cell::new(true),
        }))
    }

    pub fn function(f: Function<'a>) -> Value<'a> {
        Value::Function(Rc::new(f))
    }

    pub fn is_undefined(&self) -> bool {
        matches!(self, Value::Undefined)
    }

    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }

    pub fn is_function(&self) -> bool {
        matches!(self, Value::Function(_))
    }

    pub fn as_array(&self) -> Option<&Rc<Array<'a>>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Whether this is an array with `flag` set.
    pub fn has_flag(&self, flag: u8) -> bool {
        self.as_array().is_some_and(|a| a.has(flag))
    }

    /// The items of an array, or the value itself as a single item (`undefined` included, like
    /// `createSequence(value)` in jsonata-js).
    pub fn items_or_self(&self) -> Vec<Value<'a>> {
        match self {
            Value::Array(a) => a.items().clone(),
            other => vec![other.clone()],
        }
    }

    /// jsonata-js `isNumeric`: a number that is not NaN. Infinite numbers raise D1001.
    pub fn is_numeric(&self) -> Result<bool> {
        match self {
            Value::Number(n) if n.is_nan() => Ok(false),
            Value::Number(n) if n.is_infinite() => {
                Err(Error::new("D1001").with("value", json_number(*n)))
            }
            Value::Number(_) => Ok(true),
            _ => Ok(false),
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// The symbol used by function signatures: `s n b l a o f`, or `m` for undefined.
    pub fn signature_symbol(&self) -> char {
        match self {
            Value::Undefined => 'm',
            Value::Null => 'l',
            Value::Bool(_) => 'b',
            Value::Number(_) => 'n',
            Value::String(_) => 's',
            Value::Array(_) => 'a',
            Value::Object(_) => 'o',
            Value::Function(_) => 'f',
        }
    }

    /// jsonata-js `isArrayOfNumbers`.
    pub fn is_array_of_numbers(&self) -> Result<bool> {
        match self {
            Value::Array(a) => {
                for item in a.items() {
                    if !item.is_numeric()? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// jsonata-js `isArrayOfStrings`.
    pub fn is_array_of_strings(&self) -> bool {
        match self {
            Value::Array(a) => a.items().iter().all(|i| matches!(i, Value::String(_))),
            _ => false,
        }
    }

    /// Strict equality (`===`) as used by the `in` operator.
    pub fn strict_eq(&self, other: &Value<'a>) -> bool {
        match (self, other) {
            (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b),
            (Value::Object(a), Value::Object(b)) => Rc::ptr_eq(a, b),
            (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// Deep equality (jsonata-js `isDeepEqual`), used by `=` and `!=`. Values nested more than
    /// [`MAX_NESTING`] levels deep are not compared: `U1001`.
    pub fn deep_eq(&self, other: &Value<'a>) -> Result<bool> {
        self.deep_eq_at(other, 0)
    }

    /// [`Value::deep_eq`] for values nested in `depth` arrays and objects.
    fn deep_eq_at(&self, other: &Value<'a>, depth: usize) -> Result<bool> {
        let nested = |x: &Value<'a>, y: &Value<'a>| nest(depth + 1, || x.deep_eq_at(y, depth + 1));
        match (self, other) {
            (Value::Array(_), Value::Array(_)) | (Value::Object(_), Value::Object(_))
                if depth >= MAX_NESTING =>
            {
                Err(too_deep())
            }
            (Value::Array(a), Value::Array(b)) => {
                if Rc::ptr_eq(a, b) {
                    return Ok(true);
                }
                if a.items().len() != b.items().len() {
                    return Ok(false);
                }
                for (x, y) in a.items().iter().zip(b.items()) {
                    if !nested(x, y)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (Value::Object(a), Value::Object(b)) => {
                if Rc::ptr_eq(a, b) {
                    return Ok(true);
                }
                let (a, b) = (a.borrow(), b.borrow());
                if a.len() != b.len() {
                    return Ok(false);
                }
                for (k, v) in a.iter() {
                    match b.get(k) {
                        Some(w) if nested(v, w)? => {}
                        _ => return Ok(false),
                    }
                }
                Ok(true)
            }
            _ => Ok(self.strict_eq(other)),
        }
    }

    /// Converts input JSON into a value.
    pub fn from_json(json: &JsonValue) -> Value<'a> {
        Value::from_json_with(json, &mut Keys::default(), 0)
    }

    /// [`Value::from_json`] for JSON nested in `depth` arrays and objects.
    fn from_json_with(json: &JsonValue, keys: &mut Keys, depth: usize) -> Value<'a> {
        match json {
            JsonValue::Null => Value::Null,
            JsonValue::Bool(b) => Value::Bool(*b),
            JsonValue::Number(n) => Value::Number(n.as_f64().unwrap_or(f64::NAN)),
            JsonValue::String(s) => Value::string(s.as_str()),
            JsonValue::Array(items) => Value::array(
                items
                    .iter()
                    .map(|v| nest(depth + 1, || Value::from_json_with(v, keys, depth + 1)))
                    .collect(),
                0,
            ),
            JsonValue::Object(map) => {
                let mut out = Map::with_capacity_and_hasher(map.len(), Default::default());
                for (k, v) in map {
                    let v = nest(depth + 1, || Value::from_json_with(v, keys, depth + 1));
                    out.insert(keys.get(k), v);
                }
                Value::object(out)
            }
        }
    }

    /// A value over input JSON that reads it one level at a time, on first access (ADR 0010):
    /// an array or object is converted only when the evaluation reads it. `keys` is shared by
    /// the lazy values of one evaluation.
    pub fn lazy(json: &'a JsonValue, shared: &Rc<Shared>) -> Value<'a> {
        let pending = || Pending {
            origin: Origin::Json(json),
            shared: shared.clone(),
        };
        match json {
            JsonValue::Array(items) if !items.is_empty() => Value::lazy_array(pending()),
            JsonValue::Object(fields) if !fields.is_empty() => Value::lazy_object(pending()),
            JsonValue::Null => Value::Null,
            JsonValue::Bool(b) => Value::Bool(*b),
            JsonValue::Number(n) => Value::Number(n.as_f64().unwrap_or(f64::NAN)),
            JsonValue::String(s) => Value::string(s.as_str()),
            JsonValue::Array(_) => Value::array(Vec::new(), 0),
            JsonValue::Object(_) => Value::object(Map::default()),
        }
    }

    fn lazy_array(pending: Pending<'a>) -> Value<'a> {
        Value::Array(Rc::new(Array {
            items: Vec::new(),
            flags: Cell::new(0),
            lazy: Some(Box::new(LazyItems {
                pending,
                items: OnceCell::new(),
            })),
        }))
    }

    fn lazy_object(pending: Pending<'a>) -> Value<'a> {
        Value::Object(Rc::new(Object {
            map: RefCell::new(Map::default()),
            pending: Some(pending),
            read: Cell::new(false),
        }))
    }

    /// The input of an evaluation, read from a [`Source`] that writes it as the only value of
    /// its level (nothing for undefined). The error of a source that fails is returned.
    pub(crate) fn from_source(source: &dyn Source<'a>, shared: &Rc<Shared>) -> Result<Value<'a>> {
        let mut items = Vec::new();
        let mut sink = Sink {
            level: Level::Items(&mut items),
            shared,
            key: None,
        };
        source
            .read(&mut sink)
            .map_err(|message| Error::new("U1004").with("message", message))?;
        Ok(items.into_iter().next().unwrap_or_default())
    }

    /// The input JSON an array or object stands for, when it has not been read.
    fn unread(&self) -> Option<&'a JsonValue> {
        match self {
            Value::Array(a) => a.unread(),
            Value::Object(o) => o.unread(),
            _ => None,
        }
    }

    /// Converts a result into JSON, following `JSON.stringify`: undefined and functions are
    /// dropped from objects and become `null` in arrays; a top-level one yields `None`. It has
    /// no nesting limit: it converts documents, whose depth their JSON bounds.
    pub fn to_json(&self) -> Option<JsonValue> {
        self.to_json_at(0)
    }

    /// [`Value::to_json`] for a value nested in `depth` arrays and objects.
    fn to_json_at(&self, depth: usize) -> Option<JsonValue> {
        if let Some(json) = self.unread() {
            return Some(copy_json(json, depth));
        }
        Some(match self {
            Value::Undefined | Value::Function(_) => return None,
            Value::Null => JsonValue::Null,
            Value::Bool(b) => JsonValue::Bool(*b),
            Value::Number(n) => json_number(*n),
            Value::String(s) => JsonValue::String(s.to_string()),
            Value::Array(a) => JsonValue::Array(
                a.items()
                    .iter()
                    .map(|v| nest(depth + 1, || v.to_json_at(depth + 1)).unwrap_or(JsonValue::Null))
                    .collect(),
            ),
            Value::Object(o) => JsonValue::Object(
                o.borrow()
                    .iter()
                    .filter_map(|(k, v)| {
                        nest(depth + 1, || v.to_json_at(depth + 1)).map(|j| (k.to_string(), j))
                    })
                    .collect(),
            ),
        })
    }

    /// Converts a value into JSON like [`Value::to_json`], failing when its compact JSON text
    /// would be longer than `limit` bytes, or when it is nested more than [`MAX_NESTING`]
    /// levels deep. The size is counted while converting, which stops as soon as it is
    /// exceeded.
    pub(crate) fn to_json_within(
        &self,
        limit: usize,
    ) -> std::result::Result<Option<JsonValue>, TooLarge> {
        let mut budget = limit;
        self.to_json_spending(&mut budget, 0)
    }

    /// [`Value::to_json`] for a value nested in `depth` arrays and objects, spending the length
    /// of its compact JSON text from `budget`. Escaped characters count as one byte, and
    /// numbers about as many bytes as JavaScript prints. A value that appears several times (an
    /// object bound to a variable and nested in itself, say) counts each time, as it does in
    /// the text.
    pub(crate) fn to_json_spending(
        &self,
        budget: &mut usize,
        depth: usize,
    ) -> std::result::Result<Option<JsonValue>, TooLarge> {
        if let Some(json) = self.unread() {
            return copy_json_within(json, budget, depth).map(Some);
        }
        if matches!(self, Value::Array(_) | Value::Object(_)) && depth >= MAX_NESTING {
            return Err(TooLarge::Nesting);
        }
        Ok(Some(match self {
            Value::Undefined | Value::Function(_) => return Ok(None),
            Value::Null => {
                charge(budget, 4)?;
                JsonValue::Null
            }
            Value::Bool(b) => {
                charge(budget, if *b { 4 } else { 5 })?;
                JsonValue::Bool(*b)
            }
            Value::Number(n) => {
                charge(budget, number_len(*n))?;
                json_number(*n)
            }
            Value::String(s) => {
                charge(budget, s.len() + 2)?;
                JsonValue::String(s.to_string())
            }
            Value::Array(a) => {
                let items = a.items();
                charge(budget, 1 + items.len().max(1))?;
                let mut out = Vec::with_capacity(items.len());
                for v in items.iter() {
                    out.push(match v {
                        Value::Undefined | Value::Function(_) => {
                            charge(budget, 4)?;
                            JsonValue::Null
                        }
                        v => nest(depth + 1, || v.to_json_spending(budget, depth + 1))?
                            .unwrap_or(JsonValue::Null),
                    });
                }
                JsonValue::Array(out)
            }
            Value::Object(o) => {
                let o = o.borrow();
                charge(budget, if o.is_empty() { 2 } else { 1 })?;
                let mut out = serde_json::Map::with_capacity(o.len());
                for (k, v) in o.iter() {
                    if let Some(json) = match v {
                        Value::Undefined | Value::Function(_) => None,
                        v => {
                            charge(budget, k.len() + 4)?;
                            nest(depth + 1, || v.to_json_spending(budget, depth + 1))?
                        }
                    } {
                        out.insert(k.to_string(), json);
                    }
                }
                JsonValue::Object(out)
            }
        }))
    }

    /// The JSON of a value shown in an error detail; `None` when it is undefined, or larger
    /// than the default size limit of a result.
    pub(crate) fn to_json_detail(&self) -> Option<JsonValue> {
        self.to_json_within(crate::eval::DEFAULT_MAX_RESULT_SIZE)
            .unwrap_or(None)
    }
}

/// Input read one level at a time, for evaluations whose input is not a `serde_json::Value`
/// (see [`Expression::evaluate_source`](crate::Expression::evaluate_source) and ADR 0010).
///
/// A source stands for an array, an object, or the input itself. Each level is read once, on
/// the evaluation's first access to it, and only the levels the expression reaches are read.
/// A source that borrows nothing implements `Source<'a>` for every `'a`.
pub trait Source<'a> {
    /// Writes the values of this level to `sink`, in order: the items of an array, the fields
    /// of an object (each value after its key, given with [`Sink::key`]), or the input itself
    /// as the only value (nothing when it is undefined). Nested arrays and objects are written
    /// as new sources, read when the evaluation reaches them.
    ///
    /// An error stops the evaluation with `U1004`, whose message is the error's. The level is
    /// left empty, and the evaluation fails at its next step, before it calls a host function,
    /// or before it returns its result.
    fn read(&self, sink: &mut Sink<'_, 'a>) -> std::result::Result<(), String>;
}

/// Where a [`Source`] writes the values of a level.
pub struct Sink<'s, 'a> {
    level: Level<'s, 'a>,
    shared: &'s Rc<Shared>,
    key: Option<Rc<str>>,
}

enum Level<'s, 'a> {
    Items(&'s mut Vec<Value<'a>>),
    Fields(&'s mut Map<'a>),
}

impl<'a> Sink<'_, 'a> {
    /// Tells how many more values the level holds, when the source knows: room is made for
    /// them at once. It is only a hint.
    pub fn reserve(&mut self, additional: usize) {
        // A hint larger than any level, from a source that miscounts, is ignored.
        let additional = additional.min(1 << 16);
        match &mut self.level {
            Level::Items(items) => items.reserve(additional),
            Level::Fields(map) => map.reserve(additional),
        }
    }

    /// The key of the next value, in an object; ignored in an array or for the input. In an
    /// object, a value written without a key is left out, and a key written again replaces the
    /// earlier value, keeping its position.
    pub fn key(&mut self, key: &str) {
        if let Level::Fields(_) = self.level {
            self.key = Some(self.shared.keys.borrow_mut().get(key));
        }
    }

    /// Writes `null`.
    pub fn null(&mut self) {
        self.push(Value::Null);
    }

    /// Writes a boolean.
    pub fn bool(&mut self, value: bool) {
        self.push(Value::Bool(value));
    }

    /// Writes a number. JSON has no NaN or infinity: a source that follows JSON writes `null`
    /// for them, as `JSON.stringify` does.
    pub fn number(&mut self, value: f64) {
        self.push(Value::Number(value));
    }

    /// Writes a string.
    pub fn string(&mut self, value: &str) {
        self.push(Value::string(value));
    }

    /// Writes the items of a JSON array, or the fields of a JSON object, given as text: an
    /// array's items in an array, an object's fields in an object. They are converted at once,
    /// with everything they hold, rather than one level at a time. When the text is not valid
    /// JSON, or nests deeper than 128 levels, nothing is written and the error is returned.
    ///
    /// Use it for a level that is cheaper to serialize than to write value by value.
    pub fn json_members(&mut self, text: &str) -> std::result::Result<(), String> {
        let value: Value<'a> =
            crate::document::parse_value(text, &mut self.shared.keys.borrow_mut())
                .map_err(|e| e.to_string())?;
        match (value, &mut self.level) {
            (Value::Array(array), Level::Items(items)) => match Rc::try_unwrap(array) {
                Ok(mut array) => items.extend(std::mem::take(&mut array.items)),
                Err(array) => items.extend(array.items().iter().cloned()),
            },
            (Value::Object(object), Level::Fields(map)) => match Rc::try_unwrap(object) {
                Ok(object) => map.extend(object.map.take()),
                Err(object) => {
                    map.extend(object.borrow().iter().map(|(k, v)| (k.clone(), v.clone())))
                }
            },
            (_, Level::Items(_)) => return Err("expected a JSON array".into()),
            (_, Level::Fields(_)) => return Err("expected a JSON object".into()),
        }
        Ok(())
    }

    /// Writes an array, read from `source` when the evaluation reaches it.
    pub fn array(&mut self, source: impl Source<'a> + 'a) {
        let pending = self.pending(Box::new(source));
        self.push(Value::lazy_array(pending));
    }

    /// Writes an object, read from `source` when the evaluation reaches it.
    pub fn object(&mut self, source: impl Source<'a> + 'a) {
        let pending = self.pending(Box::new(source));
        self.push(Value::lazy_object(pending));
    }

    fn pending(&self, source: Box<dyn Source<'a> + 'a>) -> Pending<'a> {
        Pending {
            origin: Origin::Source(source),
            shared: self.shared.clone(),
        }
    }

    fn push(&mut self, value: Value<'a>) {
        match &mut self.level {
            Level::Items(items) => items.push(value),
            Level::Fields(map) => {
                if let Some(key) = self.key.take() {
                    map.insert(key, value);
                }
            }
        }
    }
}

/// A copy of input JSON, nested in `depth` arrays and objects, with its numbers as
/// [`Value::to_json`] writes them: what converting it to a value and back gives, without
/// building the value.
fn copy_json(json: &JsonValue, depth: usize) -> JsonValue {
    match json {
        JsonValue::Number(n) => json_number(n.as_f64().unwrap_or(f64::NAN)),
        JsonValue::Array(items) => JsonValue::Array(
            items
                .iter()
                .map(|v| nest(depth + 1, || copy_json(v, depth + 1)))
                .collect(),
        ),
        JsonValue::Object(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), nest(depth + 1, || copy_json(v, depth + 1))))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// [`copy_json`] for JSON nested in `depth` arrays and objects, spending the length of the text
/// from `budget` as [`Value::to_json_spending`] does.
fn copy_json_within(
    json: &JsonValue,
    budget: &mut usize,
    depth: usize,
) -> std::result::Result<JsonValue, TooLarge> {
    if matches!(json, JsonValue::Array(_) | JsonValue::Object(_)) && depth >= MAX_NESTING {
        return Err(TooLarge::Nesting);
    }
    Ok(match json {
        JsonValue::Null => {
            charge(budget, 4)?;
            JsonValue::Null
        }
        JsonValue::Bool(b) => {
            charge(budget, if *b { 4 } else { 5 })?;
            JsonValue::Bool(*b)
        }
        JsonValue::Number(n) => {
            let n = n.as_f64().unwrap_or(f64::NAN);
            charge(budget, number_len(n))?;
            json_number(n)
        }
        JsonValue::String(s) => {
            charge(budget, s.len() + 2)?;
            JsonValue::String(s.clone())
        }
        JsonValue::Array(items) => {
            charge(budget, 1 + items.len().max(1))?;
            JsonValue::Array(
                items
                    .iter()
                    .map(|v| nest(depth + 1, || copy_json_within(v, budget, depth + 1)))
                    .collect::<std::result::Result<_, _>>()?,
            )
        }
        JsonValue::Object(fields) => {
            charge(budget, if fields.is_empty() { 2 } else { 1 })?;
            let mut out = serde_json::Map::with_capacity(fields.len());
            for (k, v) in fields {
                charge(budget, k.len() + 4)?;
                let v = nest(depth + 1, || copy_json_within(v, budget, depth + 1))?;
                out.insert(k.clone(), v);
            }
            JsonValue::Object(out)
        }
    })
}

/// A value of any evaluation behind a trait object, whatever the lifetime of its input. Public
/// types that show a value (`audit::ValueRef`, `HostCall`) hold one, so that they keep a single
/// lifetime (ADR 0010).
pub(crate) trait Inspect: std::fmt::Debug {
    fn to_json(&self) -> Option<JsonValue>;
    fn to_json_within(&self, limit: usize) -> std::result::Result<Option<JsonValue>, TooLarge>;
    fn capture(&self, limits: &crate::audit::CaptureLimits) -> Option<JsonValue>;
    fn as_bool(&self) -> Option<bool>;
    fn signature_symbol(&self) -> char;
}

impl Inspect for Value<'_> {
    fn to_json(&self) -> Option<JsonValue> {
        // Within the nesting limit: a tracer may keep the JSON, and a deeper one would
        // overflow the stack when dropped.
        Value::to_json_within(self, usize::MAX).unwrap_or(None)
    }

    fn to_json_within(&self, limit: usize) -> std::result::Result<Option<JsonValue>, TooLarge> {
        Value::to_json_within(self, limit)
    }

    fn capture(&self, limits: &crate::audit::CaptureLimits) -> Option<JsonValue> {
        crate::audit::capture(self, limits, 0)
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn signature_symbol(&self) -> char {
        Value::signature_symbol(self)
    }
}

/// Takes `bytes` from `budget`, failing when there are not enough left.
fn charge(budget: &mut usize, bytes: usize) -> std::result::Result<(), TooLarge> {
    *budget = budget.checked_sub(bytes).ok_or(TooLarge::Size)?;
    Ok(())
}

/// The deepest arrays and objects may nest in a value that is converted to JSON, compared or
/// serialized. jsonata-js is limited by the JavaScript stack; jlens fails cleanly instead, and
/// the JSON it returns can then be dropped or serialized by recursive code (`serde_json`).
pub(crate) const MAX_NESTING: usize = 1_000;

/// Runs the step of a recursive walk over values that enters level `depth`, growing the stack
/// natively every 16 levels: the red zone of [`crate::stack::grow`] covers the frames between,
/// and checking at each value would slow the walks down.
#[inline]
pub(crate) fn nest<R>(depth: usize, step: impl FnOnce() -> R) -> R {
    if depth.is_multiple_of(16) {
        crate::stack::grow(step)
    } else {
        step()
    }
}

/// The error of a value nested more than [`MAX_NESTING`] levels deep.
pub(crate) fn too_deep() -> Error {
    Error::new("U1001").with(
        "message",
        format!("A value is nested more than {MAX_NESTING} levels deep"),
    )
}

/// A value whose JSON text would exceed a size limit, or the nesting limit.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TooLarge {
    Size,
    Nesting,
}

impl TooLarge {
    /// The `U1001` error, whose message is `size` for a value too large.
    pub(crate) fn error(self, size: impl FnOnce() -> String) -> Error {
        match self {
            TooLarge::Size => Error::new("U1001").with("message", size()),
            TooLarge::Nesting => too_deep(),
        }
    }
}

/// The length of a number in JSON text: its digits for an integer, about as many as
/// JavaScript prints otherwise, and 4 for `null` (a number that is not finite).
fn number_len(n: f64) -> usize {
    if !n.is_finite() {
        4
    } else if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        let mut len = usize::from(n < 0.0) + 1;
        let mut rest = (n as i64).unsigned_abs() / 10;
        while rest > 0 {
            len += 1;
            rest /= 10;
        }
        len
    } else {
        ryu_js::Buffer::new().format(n).len()
    }
}

/// A computed number, or D1001 when it overflowed to infinity, which JSON cannot hold.
pub(crate) fn finite<'a>(n: f64) -> Result<Value<'a>> {
    if n.is_infinite() {
        Err(Error::new("D1001").with("value", json_number(n)))
    } else {
        Ok(Value::Number(n))
    }
}

/// A number as JSON: integral values become JSON integers, non-finite ones `null`.
pub(crate) fn json_number(n: f64) -> JsonValue {
    if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        JsonValue::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(JsonValue::Null, JsonValue::Number)
    }
}

/// Formats a number exactly like JavaScript's `Number.prototype.toString`.
pub(crate) fn js_number(n: f64) -> String {
    if n == 0.0 {
        return "0".into();
    }
    ryu_js::Buffer::new().format(n).to_owned()
}

/// `Number(n.toPrecision(15))` for non-integers, as `$string` does before printing a number.
pub(crate) fn round15(n: f64) -> f64 {
    if n.fract() == 0.0 || !n.is_finite() {
        return n;
    }
    format!("{n:.14e}").parse().unwrap_or(n)
}

/// Compares strings like JavaScript's `<`, by UTF-16 code units.
pub(crate) fn js_string_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Writes `s` as a JSON string literal with `JSON.stringify` escaping.
pub(crate) fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `JSON.stringify` with the replacer jsonata-js uses in `$string`: numbers are rounded to 15
/// significant digits, infinite numbers raise D1001, functions become `""`.
pub(crate) fn stringify<'a>(value: &Value<'a>, pretty: bool, limit: usize) -> Result<String> {
    let mut out = String::new();
    let value = match value {
        Value::Array(a) if a.has(OUTER_WRAPPER) && !a.items().is_empty() => &a.items()[0],
        v => v,
    };
    write_json(&mut out, value, pretty, 0, limit)?;
    Ok(out)
}

/// The error of a string that would exceed the size limit of `limit` bytes.
pub(crate) fn string_too_long(limit: usize) -> Error {
    Error::new("U1001").with(
        "message",
        format!("A string would exceed the size limit of {limit} bytes"),
    )
}

/// Writes the JSON text of a value, failing as soon as the text is longer than `limit` bytes:
/// a value that appears many times could otherwise make a text far too long to hold.
fn write_json<'a>(
    out: &mut String,
    value: &Value<'a>,
    pretty: bool,
    depth: usize,
    limit: usize,
) -> Result<()> {
    let newline = |out: &mut String, depth: usize| {
        if pretty {
            out.push('\n');
            out.push_str(&"  ".repeat(depth));
        }
    };
    match value {
        Value::Undefined | Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if !value.is_numeric()? {
                out.push_str("null"); // NaN
            } else {
                out.push_str(&js_number(round15(*n)));
            }
        }
        Value::String(s) => write_json_string(out, s),
        Value::Function(_) => out.push_str("\"\""),
        Value::Array(_) | Value::Object(_) if depth >= MAX_NESTING => return Err(too_deep()),
        Value::Array(a) => {
            if a.items().is_empty() {
                out.push_str("[]");
                return Ok(());
            }
            out.push('[');
            for (i, item) in a.items().iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                nest(depth + 1, || {
                    write_json(out, item, pretty, depth + 1, limit)
                })?;
            }
            newline(out, depth);
            out.push(']');
        }
        Value::Object(o) => {
            let o = o.borrow();
            let mut first = true;
            out.push('{');
            for (k, v) in o.iter() {
                if v.is_undefined() {
                    continue;
                }
                if !first {
                    out.push(',');
                }
                first = false;
                newline(out, depth + 1);
                write_json_string(out, k);
                out.push(':');
                if pretty {
                    out.push(' ');
                }
                nest(depth + 1, || write_json(out, v, pretty, depth + 1, limit))?;
            }
            if !first {
                newline(out, depth);
            }
            out.push('}');
        }
    }
    if out.len() > limit {
        return Err(string_too_long(limit));
    }
    Ok(())
}

impl std::fmt::Debug for Value<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Undefined => f.write_str("undefined"),
            Value::Function(_) => f.write_str("<function>"),
            other => match stringify(other, false, crate::eval::DEFAULT_MAX_STRING_LENGTH) {
                Ok(s) => f.write_str(&s),
                Err(_) => f.write_str("<invalid>"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_like_javascript() {
        assert_eq!(js_number(1.0), "1");
        assert_eq!(js_number(-0.0), "0");
        assert_eq!(js_number(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(js_number(round15(0.1 + 0.2)), "0.3");
        assert_eq!(js_number(1e21), "1e+21");
        assert_eq!(js_number(123456789012.5), "123456789012.5");
    }

    #[test]
    fn json_size_is_the_length_of_the_compact_text() {
        let v = Value::from_json(&serde_json::json!({
            "a": [1, -25, 2.5, null, true, false, 1e21],
            "bc": "xyz",
            "d": {},
            "e": [],
            "f": [{}]
        }));
        let len = stringify(&v, false, usize::MAX).unwrap().len();
        assert_eq!(v.to_json_within(len).unwrap(), v.to_json());
        assert!(v.to_json_within(len - 1).is_err());
    }

    #[test]
    fn stringify_matches_json_stringify() {
        let v = Value::from_json(&serde_json::json!({"a": [1, 2.5, null], "b": "x\"y\n", "c": {}}));
        assert_eq!(
            stringify(&v, false, usize::MAX).unwrap(),
            r#"{"a":[1,2.5,null],"b":"x\"y\n","c":{}}"#
        );
        assert_eq!(
            stringify(&v, true, usize::MAX).unwrap(),
            "{\n  \"a\": [\n    1,\n    2.5,\n    null\n  ],\n  \"b\": \"x\\\"y\\n\",\n  \"c\": {}\n}"
        );
    }

    #[test]
    fn equality() {
        let a = Value::from_json(&serde_json::json!({"x": [1, {"y": 2}]}));
        let b = Value::from_json(&serde_json::json!({"x": [1.0, {"y": 2}]}));
        assert!(a.deep_eq(&b).unwrap());
        assert!(!a.strict_eq(&b));
        assert_eq!(js_string_cmp("a", "b"), Ordering::Less);
    }
}
