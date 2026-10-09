//! The built-in function library. Ported from jsonata-js `src/functions.js` (© IBM Corp., MIT).
//!
//! Every built-in is described by a [`Native`] entry: its name, signature, declared parameter
//! names (used when it is partially applied) and implementation. The table is the single source
//! of truth for evaluation, editor completion and the generated function reference.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::error::{Error, Result};
use crate::eval::{Env, Evaluator};
use crate::signature::Signature;
use crate::value::{self, Function, Value};

/// The context of a call to a built-in function.
#[allow(dead_code)] // read by the function library (phase 3)
pub(crate) struct Call<'a> {
    /// The context value of the call (the input of the function call expression).
    pub input: &'a Value,
    /// The environment of the call.
    pub env: &'a Env,
}

pub(crate) type NativeImpl = fn(&mut Evaluator, Vec<Value>, &Call) -> Result<Value>;

/// A built-in function.
pub(crate) struct Native {
    pub name: &'static str,
    pub signature: Option<Signature>,
    /// Parameter names of the reference implementation, used for partial application.
    pub params: &'static [&'static str],
    pub imp: NativeImpl,
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or_default()
}

macro_rules! natives {
    ($( $name:literal $sig:literal [$($param:literal),*] => $imp:expr ;)*) => {
        fn build() -> Vec<Native> {
            vec![$(Native {
                name: $name,
                signature: Some(Signature::parse($sig).expect("valid built-in signature")),
                params: &[$($param),*],
                imp: $imp,
            }),*]
        }
    };
}

natives! {
    "sum" "<a<n>:n>" ["args"] => |_, a, _| sum(&arg(&a, 0));
    "count" "<a:n>" ["args"] => |_, a, _| Ok(match arg(&a, 0) {
        Value::Undefined => Value::Number(0.0),
        Value::Array(arr) => Value::Number(arr.len() as f64),
        _ => Value::Number(1.0),
    });
    "max" "<a<n>:n>" ["args"] => |_, a, _| extreme(&arg(&a, 0), f64::max);
    "min" "<a<n>:n>" ["args"] => |_, a, _| extreme(&arg(&a, 0), f64::min);
    "average" "<a<n>:n>" ["args"] => |_, a, _| {
        let items = arg(&a, 0);
        let Some(arr) = items.as_array().filter(|a| !a.items.is_empty()) else {
            return Ok(Value::Undefined);
        };
        let total = sum(&items)?.as_number().unwrap_or(0.0);
        Ok(Value::Number(total / arr.len() as f64))
    };
    "string" "<x-b?:s>" ["arg", "prettify"] => |_, a, _| {
        let pretty = matches!(arg(&a, 1), Value::Bool(true));
        Ok(to_string(&arg(&a, 0), pretty)?.map_or(Value::Undefined, Value::string))
    };
    "boolean" "<x-:b>" ["arg"] => |_, a, _| Ok(boolean(&arg(&a, 0))?.map_or(Value::Undefined, Value::Bool));
    "not" "<x-:b>" ["arg"] => |_, a, _| Ok(boolean(&arg(&a, 0))?.map_or(Value::Undefined, |b| Value::Bool(!b)));
    "exists" "<x:b>" ["arg"] => |_, a, _| Ok(Value::Bool(!arg(&a, 0).is_undefined()));
    "lookup" "<x-s:x>" ["input", "key"] => |_, a, _| {
        Ok(lookup(&arg(&a, 0), arg(&a, 1).as_str().unwrap_or_default()))
    };
    "append" "<xx:a>" ["arg1", "arg2"] => |_, a, _| Ok(append(arg(&a, 0), arg(&a, 1)));
    "clone" "<(oa)-:o>" ["arg"] => |_, a, _| clone(&arg(&a, 0));
}

fn table() -> &'static [Native] {
    static TABLE: OnceLock<Vec<Native>> = OnceLock::new();
    TABLE.get_or_init(build)
}

/// The built-in function with this name.
pub(crate) fn native(name: &str) -> Option<&'static Native> {
    table().iter().find(|n| n.name == name)
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
pub(crate) fn to_string(v: &Value, pretty: bool) -> Result<Option<String>> {
    Ok(Some(match v {
        Value::Undefined => return Ok(None),
        Value::String(s) => s.to_string(),
        Value::Function(_) => String::new(),
        Value::Number(n) if !n.is_finite() => {
            return Err(Error::new("D3001").with("value", value::json_number(*n)));
        }
        other => value::stringify(other, pretty)?,
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

/// `$clone`: a deep copy made the way jsonata-js does it, by serializing and parsing.
pub(crate) fn clone(v: &Value) -> Result<Value> {
    let Some(text) = to_string(v, false)? else {
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
