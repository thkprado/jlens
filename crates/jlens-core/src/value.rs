//! The run-time value model (see ADR 0003).
//!
//! JSONata distinguishes *undefined* from `null`, and *sequences* (results of path navigation,
//! which collapse when empty or singleton) from arrays. Arrays therefore carry flags, set and
//! read exactly where jsonata-js sets and reads the corresponding properties on its arrays.
//! Objects are shared and mutable (`Rc<RefCell<…>>`) because the transform operator updates the
//! objects that its pattern selects in place.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
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

/// The fields of an object, in insertion order.
pub(crate) type Map = IndexMap<String, Value, foldhash::fast::RandomState>;

/// A JSONata value.
#[derive(Clone, Default)]
pub(crate) enum Value {
    #[default]
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(Rc<str>),
    Array(Rc<Array>),
    Object(Rc<RefCell<Map>>),
    Function(Rc<Function>),
}

pub(crate) const SEQUENCE: u8 = 1;
pub(crate) const KEEP_SINGLETON: u8 = 2;
pub(crate) const OUTER_WRAPPER: u8 = 4;
pub(crate) const CONS: u8 = 8;
pub(crate) const TUPLE_STREAM: u8 = 16;

/// An array, with the flags jsonata-js attaches to its arrays.
pub(crate) struct Array {
    pub items: Vec<Value>,
    flags: Cell<u8>,
}

impl Array {
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
        self.items.len()
    }
}

/// A function value.
pub(crate) enum Function {
    /// A JSONata `function(...)`, or a tail-call thunk.
    Lambda(Lambda),
    /// A built-in function.
    Native(&'static Native),
    /// A function of the host application.
    Host(HostFunction),
    /// A built-in or host function with some arguments supplied (`$f(?, 2)`); `None` marks a
    /// `?`.
    Partial {
        target: Rc<Function>,
        args: Vec<Option<Value>>,
    },
    /// The function produced by a transform expression.
    Transform {
        def: Arc<TransformDef>,
        env: Env,
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

pub(crate) struct Lambda {
    pub def: Arc<LambdaDef>,
    /// The parameters still to be supplied, when fewer than `def.params` (partial application).
    pub params: Option<Rc<[String]>>,
    /// Whether arguments are checked against `def.signature`; partial applications are not.
    pub validate: bool,
    pub input: Value,
    pub env: Env,
}

impl Function {
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

impl Value {
    pub fn string(s: impl Into<Rc<str>>) -> Value {
        Value::String(s.into())
    }

    pub fn array(items: Vec<Value>, flags: u8) -> Value {
        Value::Array(Rc::new(Array {
            items,
            flags: Cell::new(flags),
        }))
    }

    /// A new sequence (jsonata-js `createSequence`).
    pub fn sequence(items: Vec<Value>) -> Value {
        Value::array(items, SEQUENCE)
    }

    pub fn object(map: Map) -> Value {
        Value::Object(Rc::new(RefCell::new(map)))
    }

    pub fn function(f: Function) -> Value {
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

    pub fn as_array(&self) -> Option<&Rc<Array>> {
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
    pub fn items_or_self(&self) -> Vec<Value> {
        match self {
            Value::Array(a) => a.items.clone(),
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
                for item in &a.items {
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
            Value::Array(a) => a.items.iter().all(|i| matches!(i, Value::String(_))),
            _ => false,
        }
    }

    /// Strict equality (`===`) as used by the `in` operator.
    pub fn strict_eq(&self, other: &Value) -> bool {
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

    /// Deep equality (jsonata-js `isDeepEqual`), used by `=` and `!=`.
    pub fn deep_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Array(a), Value::Array(b)) => {
                Rc::ptr_eq(a, b)
                    || (a.items.len() == b.items.len()
                        && a.items.iter().zip(&b.items).all(|(x, y)| x.deep_eq(y)))
            }
            (Value::Object(a), Value::Object(b)) => {
                if Rc::ptr_eq(a, b) {
                    return true;
                }
                let (a, b) = (a.borrow(), b.borrow());
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| b.get(k).is_some_and(|w| v.deep_eq(w)))
            }
            _ => self.strict_eq(other),
        }
    }

    /// Converts input JSON into a value.
    pub fn from_json(json: &JsonValue) -> Value {
        match json {
            JsonValue::Null => Value::Null,
            JsonValue::Bool(b) => Value::Bool(*b),
            JsonValue::Number(n) => Value::Number(n.as_f64().unwrap_or(f64::NAN)),
            JsonValue::String(s) => Value::string(s.as_str()),
            JsonValue::Array(items) => {
                Value::array(items.iter().map(Value::from_json).collect(), 0)
            }
            JsonValue::Object(map) => Value::object(
                map.iter()
                    .map(|(k, v)| (k.clone(), Value::from_json(v)))
                    .collect(),
            ),
        }
    }

    /// Converts a result into JSON, following `JSON.stringify`: undefined and functions are
    /// dropped from objects and become `null` in arrays; a top-level one yields `None`.
    pub fn to_json(&self) -> Option<JsonValue> {
        Some(match self {
            Value::Undefined | Value::Function(_) => return None,
            Value::Null => JsonValue::Null,
            Value::Bool(b) => JsonValue::Bool(*b),
            Value::Number(n) => json_number(*n),
            Value::String(s) => JsonValue::String(s.to_string()),
            Value::Array(a) => JsonValue::Array(
                a.items
                    .iter()
                    .map(|v| v.to_json().unwrap_or(JsonValue::Null))
                    .collect(),
            ),
            Value::Object(o) => JsonValue::Object(
                o.borrow()
                    .iter()
                    .filter_map(|(k, v)| v.to_json().map(|j| (k.clone(), j)))
                    .collect(),
            ),
        })
    }

    /// Converts a value into JSON like [`Value::to_json`], failing when its compact JSON text
    /// would be longer than `limit` bytes (see [`Value::json_fits`]). The size is measured
    /// before anything is converted.
    pub(crate) fn to_json_within(
        &self,
        limit: usize,
    ) -> std::result::Result<Option<JsonValue>, TooLarge> {
        let mut budget = limit;
        if self.json_fits(&mut budget) {
            Ok(self.to_json())
        } else {
            Err(TooLarge)
        }
    }

    /// The JSON of a value shown in an error detail; `None` when it is undefined, or larger
    /// than the default size limit of a result.
    pub(crate) fn to_json_detail(&self) -> Option<JsonValue> {
        self.to_json_within(crate::eval::DEFAULT_MAX_RESULT_SIZE)
            .unwrap_or(None)
    }

    /// Spends the length of the value's compact JSON text from `budget`, and tells whether it
    /// fitted. A value that appears several times (an object bound to a variable and nested in
    /// itself, say) counts each time, as it does in the text, but the walk stops as soon as the
    /// budget runs out. Escaped characters count as one byte, and numbers about as many bytes as
    /// JavaScript prints.
    pub(crate) fn json_fits(&self, budget: &mut usize) -> bool {
        match self {
            Value::Undefined | Value::Function(_) => true,
            Value::Null => spend(budget, 4),
            Value::Bool(b) => spend(budget, if *b { 4 } else { 5 }),
            Value::Number(n) => spend(budget, number_len(*n)),
            Value::String(s) => spend(budget, s.len() + 2),
            Value::Array(a) => {
                // The brackets and the commas; an undefined item or a function becomes `null`.
                spend(budget, 1 + a.items.len().max(1))
                    && a.items.iter().all(|v| match v {
                        Value::Undefined | Value::Function(_) => spend(budget, 4),
                        v => v.json_fits(budget),
                    })
            }
            Value::Object(o) => {
                let o = o.borrow();
                // The opening brace, then for each member its quoted key, the colon, and the
                // comma or closing brace after it.
                spend(budget, if o.is_empty() { 2 } else { 1 })
                    && o.iter().all(|(k, v)| match v {
                        Value::Undefined | Value::Function(_) => true,
                        v => spend(budget, k.len() + 4) && v.json_fits(budget),
                    })
            }
        }
    }
}

/// Takes `bytes` from `budget`; false when there are not enough left.
fn spend(budget: &mut usize, bytes: usize) -> bool {
    match budget.checked_sub(bytes) {
        Some(rest) => {
            *budget = rest;
            true
        }
        None => false,
    }
}

/// A value whose JSON text would exceed a size limit.
#[derive(Debug)]
pub(crate) struct TooLarge;

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
pub(crate) fn stringify(value: &Value, pretty: bool, limit: usize) -> Result<String> {
    let mut out = String::new();
    let value = match value {
        Value::Array(a) if a.has(OUTER_WRAPPER) && !a.items.is_empty() => &a.items[0],
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
fn write_json(
    out: &mut String,
    value: &Value,
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
        Value::Array(a) => {
            if a.items.is_empty() {
                out.push_str("[]");
                return Ok(());
            }
            out.push('[');
            for (i, item) in a.items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                write_json(out, item, pretty, depth + 1, limit)?;
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
                write_json(out, v, pretty, depth + 1, limit)?;
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

impl std::fmt::Debug for Value {
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
        assert!(v.to_json_within(len).is_ok());
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
        assert!(a.deep_eq(&b));
        assert!(!a.strict_eq(&b));
        assert_eq!(js_string_cmp("a", "b"), Ordering::Less);
    }
}
