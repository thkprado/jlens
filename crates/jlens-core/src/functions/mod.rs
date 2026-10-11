//! The built-in function library. Ported from jsonata-js `src/functions.js` and
//! `src/datetime.js` (MIT).
//!
//! Every built-in is described by a `Native` entry: its name, signature, declared parameter
//! names (used when it is partially applied), category, a one-line summary and implementation.
//! The table is the single source of truth for evaluation, editor completion and the generated
//! function reference ([`catalog`]).

mod collections;
mod numeric;
mod strings;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::eval::{Env, Evaluator};
use crate::signature::Signature;
use crate::value::{self, Function, Value};

/// The context of a call to a built-in function.
pub(crate) struct Call<'a> {
    /// The context value of the call (the input of the function call expression).
    pub input: &'a Value,
    /// The environment of the call.
    pub env: &'a Env,
}

pub(crate) type NativeImpl = fn(&mut Evaluator<'_>, Vec<Value>, &Call) -> Result<Value>;

/// Groups of built-in functions, as in the JSONata documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// `$string`, `$substring`, `$replace`, ...
    String,
    /// `$number`, `$round`, `$formatNumber`, ...
    Numeric,
    /// `$sum`, `$count`, `$max`, `$min`, `$average`.
    Aggregation,
    /// `$boolean`, `$not`, `$exists`.
    Boolean,
    /// `$count`, `$append`, `$sort`, `$distinct`, ...
    Array,
    /// `$keys`, `$lookup`, `$merge`, `$each`, ...
    Object,
    /// `$map`, `$filter`, `$reduce`, `$single`, `$sift`.
    HigherOrder,
    /// `$now`, `$fromMillis`, `$toMillis`, ...
    DateTime,
    /// `$base64encode`, `$encodeUrl`, ...
    Encoding,
    /// `$type`, `$eval`, `$error`, `$assert`, `$clone`.
    Misc,
}

/// A built-in function.
pub(crate) struct Native {
    pub name: &'static str,
    pub signature: Option<Signature>,
    /// Parameter names of the reference implementation, used for partial application.
    pub params: &'static [&'static str],
    /// Number of parameters before any default value (JavaScript `Function.length`).
    pub arity: usize,
    pub category: Category,
    pub summary: &'static str,
    pub imp: NativeImpl,
}

/// Public description of a built-in function.
#[derive(Debug, Clone, Serialize)]
pub struct FunctionInfo {
    /// The name, without the leading `$`.
    pub name: &'static str,
    /// The signature, such as `<s-nn?:s>`.
    pub signature: String,
    /// Parameter names.
    pub params: &'static [&'static str],
    /// The group the function belongs to.
    pub category: Category,
    /// One-line description.
    pub summary: &'static str,
}

pub(crate) fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or_default()
}

pub(crate) fn num(args: &[Value], i: usize) -> Option<f64> {
    args.get(i).and_then(Value::as_number)
}

pub(crate) fn text(args: &[Value], i: usize) -> Option<std::rc::Rc<str>> {
    match args.get(i) {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

macro_rules! natives {
    ($( $name:literal $sig:literal [$($param:literal),*] $arity:literal $cat:ident $summary:literal => $imp:expr ;)*) => {
        fn build() -> Vec<Native> {
            vec![$(Native {
                name: $name,
                signature: Some(Signature::parse($sig).expect("valid built-in signature")),
                params: &[$($param),*],
                arity: $arity,
                category: Category::$cat,
                summary: $summary,
                imp: $imp,
            }),*]
        }
    };
}

use collections as c;
use numeric as n;
use strings as s;

natives! {
    // Aggregation
    "sum" "<a<n>:n>" ["array"] 1 Aggregation "Sum of an array of numbers" => |_, a, _| sum(&arg(&a, 0));
    "count" "<a:n>" ["array"] 1 Aggregation "Number of items in an array" => |_, a, _| Ok(match arg(&a, 0) {
        Value::Undefined => Value::Number(0.0),
        Value::Array(arr) => Value::Number(arr.len() as f64),
        _ => Value::Number(1.0),
    });
    "max" "<a<n>:n>" ["array"] 1 Aggregation "Largest number in an array" => |_, a, _| extreme(&arg(&a, 0), f64::max);
    "min" "<a<n>:n>" ["array"] 1 Aggregation "Smallest number in an array" => |_, a, _| extreme(&arg(&a, 0), f64::min);
    "average" "<a<n>:n>" ["array"] 1 Aggregation "Mean of an array of numbers" => |_, a, _| {
        let items = arg(&a, 0);
        let Some(arr) = items.as_array().filter(|a| !a.items.is_empty()) else {
            return Ok(Value::Undefined);
        };
        let total = sum(&items)?.as_number().unwrap_or(0.0);
        Ok(Value::Number(total / arr.len() as f64))
    };

    // String
    "string" "<x-b?:s>" ["arg", "prettify"] 1 String "Casts a value to a string; objects and arrays become JSON" => |ev, a, _| {
        let pretty = matches!(arg(&a, 1), Value::Bool(true));
        Ok(to_string(&arg(&a, 0), pretty, ev.max_string_length())?.map_or(Value::Undefined, Value::string))
    };
    "substring" "<s-nn?:s>" ["str", "start", "length"] 3 String "Characters from a start position, optionally limited to a length" => s::substring;
    "substringBefore" "<s-s:s>" ["str", "chars"] 2 String "The part of a string before the first occurrence of a substring" => s::substring_before;
    "substringAfter" "<s-s:s>" ["str", "chars"] 2 String "The part of a string after the first occurrence of a substring" => s::substring_after;
    "lowercase" "<s-:s>" ["str"] 1 String "Converts a string to lower case" => s::lowercase;
    "uppercase" "<s-:s>" ["str"] 1 String "Converts a string to upper case" => s::uppercase;
    "length" "<s-:n>" ["str"] 1 String "Number of characters in a string" => s::length;
    "trim" "<s-:s>" ["str"] 1 String "Normalizes and trims whitespace" => s::trim;
    "pad" "<s-ns?:s>" ["str", "width", "char"] 3 String "Pads a string to a width; positive pads on the right, negative on the left" => s::pad;
    "match" "<s-f<s:o>n?:a<o>>" ["str", "regex", "limit"] 3 String "Matches a regular expression, returning the matches with their positions and groups" => s::match_;
    "contains" "<s-(sf):b>" ["str", "token"] 2 String "Whether a string contains a substring or matches a regular expression" => s::contains;
    "replace" "<s-(sf)(sf)n?:s>" ["str", "pattern", "replacement", "limit"] 4 String "Replaces occurrences of a substring or regular expression" => s::replace;
    "split" "<s-(sf)n?:a<s>>" ["str", "separator", "limit"] 3 String "Splits a string by a separator or regular expression" => s::split;
    "join" "<a<s>s?:s>" ["strs", "separator"] 2 String "Joins an array of strings with an optional separator" => s::join;
    "formatNumber" "<n-so?:s>" ["value", "picture", "options"] 3 Numeric "Formats a number with an XPath picture string" => n::format_number;
    "formatBase" "<n-n?:s>" ["value", "radix"] 2 Numeric "Formats a number in another base (2 to 36)" => n::format_base;
    "formatInteger" "<n-s:s>" ["value", "picture"] 2 Numeric "Formats an integer with an XPath picture string (words, roman, letters, digits)" => |ev, a, _| match num(&a, 0) {
        None => Ok(Value::Undefined),
        Some(v) => crate::datetime::format_integer(v, &text(&a, 1).unwrap_or_default(), ev.max_string_length()).map(Value::string),
    };
    "parseInteger" "<s-s:n>" ["value", "picture"] 2 Numeric "Parses an integer written according to an XPath picture string" => |_, a, _| match text(&a, 0) {
        None => Ok(Value::Undefined),
        Some(v) => crate::datetime::parse_integer(&v, &text(&a, 1).unwrap_or_default()).map(Value::Number),
    };

    // Numeric
    "number" "<(nsb)-:n>" ["arg"] 1 Numeric "Casts a value to a number" => n::number;
    "floor" "<n-:n>" ["arg"] 1 Numeric "Rounds down" => |_, a, _| Ok(num(&a, 0).map_or(Value::Undefined, |v| Value::Number(v.floor())));
    "ceil" "<n-:n>" ["arg"] 1 Numeric "Rounds up" => |_, a, _| Ok(num(&a, 0).map_or(Value::Undefined, |v| Value::Number(v.ceil())));
    "round" "<n-n?:n>" ["arg", "precision"] 2 Numeric "Rounds half to even, optionally to a number of decimal places" => |_, a, _| {
        Ok(num(&a, 0).map_or(Value::Undefined, |v| Value::Number(n::round(v, num(&a, 1).unwrap_or(0.0)))))
    };
    "abs" "<n-:n>" ["arg"] 1 Numeric "Absolute value" => |_, a, _| Ok(num(&a, 0).map_or(Value::Undefined, |v| Value::Number(v.abs())));
    "sqrt" "<n-:n>" ["arg"] 1 Numeric "Square root" => n::sqrt;
    "power" "<n-n:n>" ["arg", "exp"] 2 Numeric "Raises a number to a power" => n::power;
    "random" "<:n>" [] 0 Numeric "A pseudo-random number between 0 (inclusive) and 1 (exclusive)" => |ev, _, _| Ok(Value::Number(ev.random()));

    // Boolean
    "boolean" "<x-:b>" ["arg"] 1 Boolean "The effective boolean value of a value" => |_, a, _| Ok(boolean(&arg(&a, 0))?.map_or(Value::Undefined, Value::Bool));
    "not" "<x-:b>" ["arg"] 1 Boolean "Logical negation of the effective boolean value" => |_, a, _| Ok(boolean(&arg(&a, 0))?.map_or(Value::Undefined, |b| Value::Bool(!b)));
    "exists" "<x:b>" ["arg"] 1 Boolean "Whether a value is defined" => |_, a, _| Ok(Value::Bool(!arg(&a, 0).is_undefined()));

    // Higher-order
    "map" "<af>" ["arr", "func"] 2 HigherOrder "Applies a function to each item of an array" => c::map;
    "zip" "<a+>" [] 0 Array "Combines arrays item by item into an array of arrays" => c::zip;
    "filter" "<af>" ["arr", "func"] 2 HigherOrder "The items of an array for which a predicate holds" => c::filter;
    "single" "<af?>" ["arr", "func"] 2 HigherOrder "The only item of an array matching a predicate; an error otherwise" => c::single;
    "reduce" "<afj?:j>" ["sequence", "func", "init"] 3 HigherOrder "Folds an array into a single value with a function" => c::reduce;
    "sift" "<o-f?:o>" ["arg", "func"] 2 HigherOrder "The key/value pairs of an object for which a predicate holds" => c::sift;

    // Object
    "keys" "<x-:a<s>>" ["arg"] 1 Object "The keys of an object, or the union of keys of an array of objects" => c::keys;
    "lookup" "<x-s:x>" ["input", "key"] 2 Object "The value of a key in an object, or in each object of an array" => |_, a, _| {
        Ok(lookup(&arg(&a, 0), text(&a, 1).as_deref().unwrap_or_default()))
    };
    "spread" "<x-:a<o>>" ["arg"] 1 Object "Splits an object into an array of single-key objects" => c::spread;
    "merge" "<a<o>:o>" ["arg"] 1 Object "Merges an array of objects into one; later keys win" => c::merge;
    "each" "<o-f:a>" ["obj", "func"] 2 Object "Applies a function to each key/value pair of an object" => c::each;

    // Array
    "append" "<xx:a>" ["arg1", "arg2"] 2 Array "Concatenates two values into an array" => |_, a, _| Ok(append(arg(&a, 0), arg(&a, 1)));
    "reverse" "<a:a>" ["arr"] 1 Array "Reverses an array" => c::reverse;
    "sort" "<af?:a>" ["arr", "comparator"] 2 Array "Sorts an array, optionally with a comparator function" => c::sort;
    "shuffle" "<a:a>" ["arr"] 1 Array "Shuffles an array into a random order" => c::shuffle;
    "distinct" "<x:x>" ["arr"] 1 Array "Removes duplicate values from an array" => c::distinct;

    // Misc
    "error" "<s?:x>" ["message"] 1 Misc "Raises an error (D3137) with a message" => c::error;
    "assert" "<bs?:x>" ["condition", "message"] 2 Misc "Raises an error (D3141) when a condition is false" => c::assert;
    "type" "<x:s>" ["value"] 1 Misc "The type of a value: null, number, string, boolean, array, object or function" => c::type_;
    "eval" "<sx?:x>" ["expr", "focus"] 2 Misc "Parses and evaluates a JSONata expression given as a string" => c::eval;
    "clone" "<(oa)-:o>" ["arg"] 1 Misc "A deep copy of a value" => |ev, a, _| clone(&arg(&a, 0), ev.max_string_length());

    // Encoding
    "base64encode" "<s-:s>" ["str"] 1 Encoding "Encodes a string as Base64 (each character taken as a byte)" => s::base64encode;
    "base64decode" "<s-:s>" ["str"] 1 Encoding "Decodes a Base64 string (each byte becomes a character)" => s::base64decode;
    "encodeUrlComponent" "<s-:s>" ["str"] 1 Encoding "Percent-encodes a URL component" => s::encode_url_component;
    "encodeUrl" "<s-:s>" ["str"] 1 Encoding "Percent-encodes a URL, leaving reserved characters" => s::encode_url;
    "decodeUrlComponent" "<s-:s>" ["str"] 1 Encoding "Decodes a percent-encoded URL component" => s::decode_url_component;
    "decodeUrl" "<s-:s>" ["str"] 1 Encoding "Decodes a percent-encoded URL, leaving reserved characters encoded" => s::decode_url;

    // Date/time
    "now" "<s?s?:s>" ["picture", "timezone"] 2 DateTime "The evaluation's timestamp as an ISO 8601 string, or formatted with a picture" => |ev, a, _| {
        crate::datetime::from_millis(ev.timestamp(), text(&a, 0).as_deref(), text(&a, 1).as_deref()).map(Value::string)
    };
    "millis" "<:n>" [] 0 DateTime "The evaluation's timestamp in milliseconds since the epoch" => |ev, _, _| Ok(Value::Number(ev.timestamp()));
    "fromMillis" "<n-s?s?:s>" ["millis", "picture", "timezone"] 3 DateTime "Formats milliseconds since the epoch as a timestamp string" => |_, a, _| match num(&a, 0) {
        None => Ok(Value::Undefined),
        Some(ms) => crate::datetime::from_millis(ms, text(&a, 1).as_deref(), text(&a, 2).as_deref()).map(Value::string),
    };
    "toMillis" "<s-s?:n>" ["timestamp", "picture"] 2 DateTime "Parses a timestamp string into milliseconds since the epoch" => |ev, a, _| match text(&a, 0) {
        None => Ok(Value::Undefined),
        Some(ts) => crate::datetime::to_millis(&ts, text(&a, 1).as_deref(), ev.timestamp(), |re, s| ev.find_regex(re, s, 0))
            .map(|v| v.map_or(Value::Undefined, Value::Number)),
    };
}

fn table() -> &'static [Native] {
    static TABLE: OnceLock<Vec<Native>> = OnceLock::new();
    TABLE.get_or_init(build)
}

/// The built-in function with this name.
pub(crate) fn native(name: &str) -> Option<&'static Native> {
    table().iter().find(|n| n.name == name)
}

/// Descriptions of every built-in function, in documentation order.
pub fn catalog() -> Vec<FunctionInfo> {
    table()
        .iter()
        .map(|n| FunctionInfo {
            name: n.name,
            signature: n
                .signature
                .as_ref()
                .map(|s| s.definition.clone())
                .unwrap_or_default(),
            params: n.params,
            category: n.category,
            summary: n.summary,
        })
        .collect()
}

thread_local! {
    /// One function value per built-in, so that `$sum = $sum` holds as it does in jsonata-js.
    static VALUES: RefCell<HashMap<&'static str, Value>> = RefCell::new(HashMap::new());
}

/// The value bound to a built-in function name, if there is one.
pub(crate) fn native_value(name: &str) -> Option<Value> {
    let native = native(name)?;
    Some(VALUES.with(|values| {
        values
            .borrow_mut()
            .entry(native.name)
            .or_insert_with(|| Value::function(Function::Native(native)))
            .clone()
    }))
}

// ---------------------------------------------------------------------------------------------
// Helpers shared with the evaluator.

/// The effective boolean value (`$boolean`); `None` for undefined.
pub(crate) fn boolean(v: &Value) -> Result<Option<bool>> {
    Ok(Some(match v {
        Value::Undefined => return Ok(None),
        Value::Array(a) => match a.items.len() {
            0 => false,
            1 => return boolean(&a.items[0]),
            _ => {
                let mut any = false;
                for item in &a.items {
                    if boolean(item)? == Some(true) {
                        any = true;
                    }
                }
                any
            }
        },
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => v.is_numeric()? && *n != 0.0,
        Value::Object(o) => !o.borrow().is_empty(),
        Value::Bool(b) => *b,
        Value::Null | Value::Function(_) => false,
    }))
}

/// Looks up a field in an object, or in each object of an array (flattening one level).
pub(crate) fn lookup(input: &Value, key: &str) -> Value {
    match input {
        Value::Array(a) => {
            let mut out = Vec::new();
            for item in &a.items {
                match lookup(item, key) {
                    Value::Undefined => {}
                    Value::Array(inner) => out.extend(inner.items.iter().cloned()),
                    other => out.push(other),
                }
            }
            Value::sequence(out)
        }
        Value::Object(o) => o.borrow().get(key).cloned().unwrap_or_default(),
        _ => Value::Undefined,
    }
}

/// `$append`: concatenates, treating non-arrays as single items. The result is a plain array.
pub(crate) fn append(a: Value, b: Value) -> Value {
    if a.is_undefined() {
        return b;
    }
    if b.is_undefined() {
        return a;
    }
    let mut items = a.items_or_self();
    match &b {
        Value::Array(arr) => items.extend(arr.items.iter().cloned()),
        other => items.push(other.clone()),
    }
    Value::array(items, 0)
}

/// `$string`: `None` for undefined.
pub(crate) fn to_string(v: &Value, pretty: bool, limit: usize) -> Result<Option<String>> {
    Ok(Some(match v {
        Value::Undefined => return Ok(None),
        Value::String(s) => s.to_string(),
        Value::Function(_) => String::new(),
        Value::Number(n) if !n.is_finite() => {
            return Err(Error::new("D3001").with("value", value::json_number(*n)));
        }
        other => value::stringify(other, pretty, limit)?,
    }))
}

fn sum(items: &Value) -> Result<Value> {
    let Value::Array(a) = items else {
        return Ok(Value::Undefined);
    };
    Ok(Value::Number(
        a.items.iter().filter_map(Value::as_number).sum(),
    ))
}

fn extreme(items: &Value, pick: fn(f64, f64) -> f64) -> Result<Value> {
    match items {
        Value::Array(a) if !a.items.is_empty() => Ok(Value::Number(
            a.items
                .iter()
                .filter_map(Value::as_number)
                .reduce(pick)
                .unwrap_or(f64::NAN),
        )),
        _ => Ok(Value::Undefined),
    }
}

/// `$clone`: a deep copy made the way jsonata-js does it, by serializing and parsing. The text
/// is limited to `limit` bytes, like a string.
pub(crate) fn clone(v: &Value, limit: usize) -> Result<Value> {
    let Some(text) = to_string(v, false, limit)? else {
        return Ok(Value::Undefined);
    };
    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| Error::new("D3141").with("message", e.to_string()))?;
    Ok(Value::from_json(&json))
}

/// The stable merge sort of jsonata-js: `swap(a, b)` returns true when `b` must come first.
pub(crate) fn merge_sort(
    items: Vec<Value>,
    swap: &mut dyn FnMut(&Value, &Value) -> Result<bool>,
) -> Result<Vec<Value>> {
    if items.len() <= 1 {
        return Ok(items);
    }
    let mut left = items;
    let right = left.split_off(left.len() / 2);
    let left = merge_sort(left, swap)?;
    let right = merge_sort(right, swap)?;
    let mut out = Vec::with_capacity(left.len() + right.len());
    let (mut i, mut j) = (0, 0);
    while i < left.len() && j < right.len() {
        if swap(&left[i], &right[j])? {
            out.push(right[j].clone());
            j += 1;
        } else {
            out.push(left[i].clone());
            i += 1;
        }
    }
    out.extend_from_slice(&left[i..]);
    out.extend_from_slice(&right[j..]);
    Ok(out)
}

/// The arguments passed to the function argument of `$map`, `$filter`, `$each`, ...: the value,
/// then the index (or key) and the whole collection if the function declares them.
pub(crate) fn hof_args(func: &Value, a1: Value, a2: Value, a3: Value) -> Vec<Value> {
    let arity = match func {
        Value::Function(f) => f.arity(),
        _ => 1,
    };
    let mut args = vec![a1];
    if arity >= 2 {
        args.push(a2);
    }
    if arity >= 3 {
        args.push(a3);
    }
    args
}
