//! Higher-order, array, object and miscellaneous functions.

use super::{Call, append, arg, boolean, hof_args, merge_sort, text};
use std::rc::Rc;

use crate::error::{Error, Result};
use crate::eval::Evaluator;
use crate::value::{self, Map, SEQUENCE, Value};

type R = Result<Value>;

fn call(ev: &mut Evaluator, func: &Value, args: Vec<Value>, c: &Call) -> R {
    ev.apply(func, args, &Value::Null, c.env)
}

pub(super) fn map(ev: &mut Evaluator, a: Vec<Value>, c: &Call) -> R {
    let arr = arg(&a, 0);
    if arr.is_undefined() {
        return Ok(Value::Undefined);
    }
    let func = arg(&a, 1);
    let items = arr.items_or_self();
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let args = hof_args(&func, item.clone(), Value::Number(i as f64), arr.clone());
        let res = call(ev, &func, args, c)?;
        if !res.is_undefined() {
            out.push(res);
        }
    }
    Ok(Value::sequence(out))
}

pub(super) fn filter(ev: &mut Evaluator, a: Vec<Value>, c: &Call) -> R {
    let arr = arg(&a, 0);
    if arr.is_undefined() {
        return Ok(Value::Undefined);
    }
    let func = arg(&a, 1);
    let mut out = Vec::new();
    for (i, item) in arr.items_or_self().iter().enumerate() {
        let args = hof_args(&func, item.clone(), Value::Number(i as f64), arr.clone());
        if boolean(&call(ev, &func, args, c)?)? == Some(true) {
            out.push(item.clone());
        }
    }
    Ok(Value::sequence(out))
}

pub(super) fn single(ev: &mut Evaluator, a: Vec<Value>, c: &Call) -> R {
    let arr = arg(&a, 0);
    if arr.is_undefined() {
        return Ok(Value::Undefined);
    }
    let func = arg(&a, 1);
    let mut found: Option<Value> = None;
    for (i, item) in arr.items_or_self().iter().enumerate() {
        let positive = if func.is_undefined() {
            true
        } else {
            let args = hof_args(&func, item.clone(), Value::Number(i as f64), arr.clone());
            boolean(&call(ev, &func, args, c)?)? == Some(true)
        };
        if positive {
            if found.is_some() {
                return Err(Error::new("D3138").with("index", i));
            }
            found = Some(item.clone());
        }
    }
    found.ok_or_else(|| Error::new("D3139"))
}

pub(super) fn zip(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let length = a
        .iter()
        .map(|v| v.as_array().map_or(0, |arr| arr.len()))
        .min()
        .unwrap_or(0);
    let tuples = (0..length)
        .map(|i| {
            Value::array(
                a.iter()
                    .map(|v| {
                        v.as_array()
                            .map(|arr| arr.items[i].clone())
                            .unwrap_or_default()
                    })
                    .collect(),
                0,
            )
        })
        .collect();
    Ok(Value::array(tuples, 0))
}

pub(super) fn reduce(ev: &mut Evaluator, a: Vec<Value>, c: &Call) -> R {
    let sequence = arg(&a, 0);
    if sequence.is_undefined() {
        return Ok(Value::Undefined);
    }
    let func = arg(&a, 1);
    let arity = match &func {
        Value::Function(f) => f.arity(),
        _ => 0,
    };
    if arity < 2 {
        return Err(Error::new("D3050").with("index", 1));
    }
    let items = sequence.items_or_self();
    let init = arg(&a, 2);
    let (mut result, mut index) = if init.is_undefined() && !items.is_empty() {
        (items[0].clone(), 1)
    } else {
        (init, 0)
    };
    while index < items.len() {
        let mut args = vec![result, items[index].clone()];
        if arity >= 3 {
            args.push(Value::Number(index as f64));
        }
        if arity >= 4 {
            args.push(sequence.clone());
        }
        result = call(ev, &func, args, c)?;
        index += 1;
    }
    Ok(result)
}

pub(super) fn sift(ev: &mut Evaluator, a: Vec<Value>, c: &Call) -> R {
    let obj = arg(&a, 0);
    let func = arg(&a, 1);
    let mut out = Map::default();
    for (key, value) in entries(&obj) {
        let args = hof_args(
            &func,
            value.clone(),
            Value::String(key.clone()),
            obj.clone(),
        );
        if boolean(&call(ev, &func, args, c)?)? == Some(true) {
            out.insert(key, value);
        }
    }
    Ok(if out.is_empty() {
        Value::Undefined
    } else {
        Value::object(out)
    })
}

fn entries(v: &Value) -> Vec<(Rc<str>, Value)> {
    match v {
        Value::Object(o) => o
            .borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        _ => Vec::new(),
    }
}

fn keys_of(v: &Value, out: &mut Vec<Rc<str>>) {
    match v {
        Value::Array(a) => {
            let mut merged = Vec::new();
            for item in &a.items {
                keys_of(item, &mut merged);
            }
            for k in merged {
                if !out.contains(&k) {
                    out.push(k);
                }
            }
        }
        Value::Object(o) => {
            for k in o.borrow().keys() {
                if !out.contains(k) {
                    out.push(k.clone());
                }
            }
        }
        _ => {}
    }
}

pub(super) fn keys(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let mut out = Vec::new();
    keys_of(&arg(&a, 0), &mut out);
    Ok(Value::sequence(
        out.into_iter().map(Value::string).collect(),
    ))
}

pub(super) fn spread(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    Ok(spread_value(&arg(&a, 0)))
}

fn spread_value(v: &Value) -> Value {
    match v {
        Value::Array(arr) => {
            let mut result = Value::sequence(vec![]);
            for item in &arr.items {
                result = append(result, spread_value(item));
            }
            result
        }
        Value::Object(o) => Value::sequence(
            o.borrow()
                .iter()
                .map(|(k, v)| {
                    let mut m = Map::default();
                    m.insert(k.clone(), v.clone());
                    Value::object(m)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

pub(super) fn merge(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let arr = arg(&a, 0);
    if arr.is_undefined() {
        return Ok(Value::Undefined);
    }
    let mut out = Map::default();
    for item in arr.items_or_self() {
        for (k, v) in entries(&item) {
            out.insert(k, v);
        }
    }
    Ok(Value::object(out))
}

pub(super) fn each(ev: &mut Evaluator, a: Vec<Value>, c: &Call) -> R {
    let obj = arg(&a, 0);
    let func = arg(&a, 1);
    let mut out = Vec::new();
    for (key, value) in entries(&obj) {
        let args = hof_args(&func, value, Value::String(key.clone()), obj.clone());
        let res = call(ev, &func, args, c)?;
        if !res.is_undefined() {
            out.push(res);
        }
    }
    Ok(Value::sequence(out))
}

pub(super) fn reverse(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let arr = arg(&a, 0);
    match &arr {
        Value::Undefined => Ok(Value::Undefined),
        Value::Array(items) if items.len() > 1 => {
            Ok(Value::array(items.items.iter().rev().cloned().collect(), 0))
        }
        _ => Ok(arr),
    }
}

pub(super) fn sort(ev: &mut Evaluator, a: Vec<Value>, c: &Call) -> R {
    let arr = arg(&a, 0);
    let Value::Array(items) = &arr else {
        return Ok(arr);
    };
    if items.len() <= 1 {
        return Ok(arr.clone());
    }
    let comparator = arg(&a, 1);
    let sorted = if comparator.is_undefined() {
        let numbers = arr.is_array_of_numbers()?;
        if !numbers && !arr.is_array_of_strings() {
            return Err(Error::new("D3070").with("index", 1));
        }
        merge_sort(items.items.clone(), &mut |x, y| {
            Ok(match (x, y) {
                (Value::Number(x), Value::Number(y)) => x > y,
                (Value::String(x), Value::String(y)) => value::js_string_cmp(x, y).is_gt(),
                _ => false,
            })
        })?
    } else {
        merge_sort(items.items.clone(), &mut |x, y| {
            let res = call(ev, &comparator, vec![x.clone(), y.clone()], c)?;
            Ok(js_truthy(&res))
        })?
    };
    Ok(Value::array(sorted, 0))
}

fn js_truthy(v: &Value) -> bool {
    match v {
        Value::Undefined | Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => *n != 0.0 && !n.is_nan(),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

pub(super) fn shuffle(ev: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let arr = arg(&a, 0);
    let Value::Array(items) = &arr else {
        return Ok(arr);
    };
    if items.len() <= 1 {
        return Ok(arr.clone());
    }
    // The 'inside-out' variant of the Fisher-Yates shuffle, as in jsonata-js.
    let mut result: Vec<Value> = Vec::with_capacity(items.len());
    for (i, item) in items.items.iter().enumerate() {
        let j = (ev.random() * (i + 1) as f64).floor() as usize;
        if j == i {
            result.push(item.clone());
        } else {
            let moved = result[j].clone();
            result.push(moved);
            result[j] = item.clone();
        }
    }
    Ok(Value::array(result, 0))
}

pub(super) fn distinct(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let arr = arg(&a, 0);
    let Value::Array(items) = &arr else {
        return Ok(arr);
    };
    if items.len() <= 1 {
        return Ok(arr.clone());
    }
    let mut out: Vec<Value> = Vec::new();
    for v in &items.items {
        if !out.iter().any(|seen| v.deep_eq(seen)) {
            out.push(v.clone());
        }
    }
    Ok(Value::array(
        out,
        if items.is_sequence() { SEQUENCE } else { 0 },
    ))
}

pub(super) fn error(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let message = text(&a, 0).filter(|m| !m.is_empty()).map_or_else(
        || "$error() function evaluated".to_owned(),
        |m| m.to_string(),
    );
    Err(Error::new("D3137").with("message", message))
}

pub(super) fn assert(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    if !matches!(arg(&a, 0), Value::Bool(true)) {
        let message = text(&a, 1).filter(|m| !m.is_empty()).map_or_else(
            || "$assert() statement failed".to_owned(),
            |m| m.to_string(),
        );
        return Err(Error::new("D3141").with("message", message));
    }
    Ok(Value::Undefined)
}

pub(super) fn type_(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let v = arg(&a, 0);
    Ok(match &v {
        Value::Undefined => Value::Undefined,
        Value::Null => Value::string("null"),
        Value::Number(_) if v.is_numeric()? => Value::string("number"),
        Value::String(_) => Value::string("string"),
        Value::Bool(_) => Value::string("boolean"),
        Value::Array(_) => Value::string("array"),
        Value::Function(_) => Value::string("function"),
        _ => Value::string("object"),
    })
}

pub(super) fn eval(ev: &mut Evaluator, a: Vec<Value>, c: &Call) -> R {
    let Some(source) = text(&a, 0) else {
        return Ok(Value::Undefined);
    };
    let input = match arg(&a, 1) {
        Value::Undefined => c.input.clone(),
        Value::Array(arr) if !arr.is_sequence() => Value::array(
            vec![Value::Array(arr)],
            value::SEQUENCE | value::OUTER_WRAPPER,
        ),
        focus => focus,
    };
    let ast = crate::parse(&source).map_err(|e| Error::new("D3120").with("value", e.message()))?;
    ev.eval(&ast.root, &input, c.env)
        .map_err(|e| Error::new("D3121").with("value", e.message()))
}
