//! Higher-order, array, object and miscellaneous functions.

use super::{Call, arg, boolean, hof_args, merge_sort, text};
use std::rc::Rc;

use crate::error::{Error, Result};
use crate::eval::Evaluator;
use crate::value::{self, Map, SEQUENCE, Value};

type R<'a> = Result<Value<'a>>;

fn call<'a>(
    ev: &mut Evaluator<'_, 'a>,
    func: &Value<'a>,
    args: Vec<Value<'a>>,
    c: &Call<'_, 'a>,
) -> R<'a> {
    ev.callback(func, args, c.env)
}

pub(super) fn map<'a>(ev: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, c: &Call<'_, 'a>) -> R<'a> {
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

pub(super) fn filter<'a>(ev: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, c: &Call<'_, 'a>) -> R<'a> {
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

pub(super) fn single<'a>(ev: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, c: &Call<'_, 'a>) -> R<'a> {
    let arr = arg(&a, 0);
    if arr.is_undefined() {
        return Ok(Value::Undefined);
    }
    let func = arg(&a, 1);
    let mut found: Option<Value<'a>> = None;
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

pub(super) fn zip<'a>(_: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, _: &Call<'_, 'a>) -> R<'a> {
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
                            .map(|arr| arr.items()[i].clone())
                            .unwrap_or_default()
                    })
                    .collect(),
                0,
            )
        })
        .collect();
    Ok(Value::array(tuples, 0))
}

pub(super) fn reduce<'a>(ev: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, c: &Call<'_, 'a>) -> R<'a> {
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

pub(super) fn sift<'a>(ev: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, c: &Call<'_, 'a>) -> R<'a> {
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

fn entries<'a>(v: &Value<'a>) -> Vec<(Rc<str>, Value<'a>)> {
    match v {
        Value::Object(o) => o
            .borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        _ => Vec::new(),
    }
}

/// The keys of an object, or of the objects in an array and in the arrays nested in it, each
/// once, in the order they first appear.
fn keys_of<'a>(v: &Value<'a>, out: &mut Vec<Rc<str>>) {
    let mut object_keys = |v: &Value<'a>| {
        if let Value::Object(o) = v {
            for k in o.borrow().keys() {
                if !out.contains(k) {
                    out.push(k.clone());
                }
            }
        }
        Ok::<_, std::convert::Infallible>(())
    };
    let Ok(()) = match v {
        Value::Array(a) => value::for_each_leaf(a.items(), object_keys),
        v => object_keys(v),
    };
}

pub(super) fn keys<'a>(_: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, _: &Call<'_, 'a>) -> R<'a> {
    let mut out = Vec::new();
    keys_of(&arg(&a, 0), &mut out);
    Ok(Value::sequence(
        out.into_iter().map(Value::string).collect(),
    ))
}

pub(super) fn spread<'a>(_: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, _: &Call<'_, 'a>) -> R<'a> {
    Ok(spread_value(&arg(&a, 0)))
}

fn spread_value<'a>(v: &Value<'a>) -> Value<'a> {
    match v {
        // The items spread, those of nested arrays included. jsonata-js appends each item's
        // result to an empty sequence with `$append`, which makes it a plain array as soon as
        // an item is defined.
        Value::Array(arr) => {
            let mut items = Vec::new();
            let Ok(()) = value::for_each_leaf(arr.items(), |item| {
                match item {
                    Value::Object(o) => items.extend(field_objects(o)),
                    Value::Undefined => {}
                    other => items.push(other.clone()),
                }
                Ok::<_, std::convert::Infallible>(())
            });
            let plain = arr.items().iter().any(|item| !item.is_undefined());
            Value::array(items, if plain { 0 } else { SEQUENCE })
        }
        Value::Object(o) => Value::sequence(field_objects(o)),
        other => other.clone(),
    }
}

/// An object for each field of `o`, holding that field alone.
fn field_objects<'a>(o: &value::Object<'a>) -> Vec<Value<'a>> {
    o.borrow()
        .iter()
        .map(|(k, v)| {
            let mut m = Map::default();
            m.insert(k.clone(), v.clone());
            Value::object(m)
        })
        .collect()
}

pub(super) fn merge<'a>(_: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, _: &Call<'_, 'a>) -> R<'a> {
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

pub(super) fn each<'a>(ev: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, c: &Call<'_, 'a>) -> R<'a> {
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

pub(super) fn reverse<'a>(_: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, _: &Call<'_, 'a>) -> R<'a> {
    let arr = arg(&a, 0);
    match &arr {
        Value::Undefined => Ok(Value::Undefined),
        Value::Array(items) if items.len() > 1 => Ok(Value::array(
            items.items().iter().rev().cloned().collect(),
            0,
        )),
        _ => Ok(arr),
    }
}

pub(super) fn sort<'a>(ev: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, c: &Call<'_, 'a>) -> R<'a> {
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
        merge_sort(items.items().clone(), &mut |x, y| {
            Ok(match (x, y) {
                (Value::Number(x), Value::Number(y)) => x > y,
                (Value::String(x), Value::String(y)) => value::js_string_cmp(x, y).is_gt(),
                _ => false,
            })
        })?
    } else {
        merge_sort(items.items().clone(), &mut |x, y| {
            let res = call(ev, &comparator, vec![x.clone(), y.clone()], c)?;
            Ok(js_truthy(&res))
        })?
    };
    Ok(Value::array(sorted, 0))
}

fn js_truthy<'a>(v: &Value<'a>) -> bool {
    match v {
        Value::Undefined | Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => *n != 0.0 && !n.is_nan(),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

pub(super) fn shuffle<'a>(
    ev: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    _: &Call<'_, 'a>,
) -> R<'a> {
    let arr = arg(&a, 0);
    let Value::Array(items) = &arr else {
        return Ok(arr);
    };
    if items.len() <= 1 {
        return Ok(arr.clone());
    }
    // The 'inside-out' variant of the Fisher-Yates shuffle, as in jsonata-js.
    let mut result: Vec<Value<'a>> = Vec::with_capacity(items.len());
    for (i, item) in items.items().iter().enumerate() {
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

pub(super) fn distinct<'a>(
    _: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    _: &Call<'_, 'a>,
) -> R<'a> {
    let arr = arg(&a, 0);
    let Value::Array(items) = &arr else {
        return Ok(arr);
    };
    if items.len() <= 1 {
        return Ok(arr.clone());
    }
    let mut out: Vec<Value<'a>> = Vec::new();
    for v in items.items() {
        let mut seen = false;
        for earlier in &out {
            if v.deep_eq(earlier)? {
                seen = true;
                break;
            }
        }
        if !seen {
            out.push(v.clone());
        }
    }
    Ok(Value::array(
        out,
        if items.is_sequence() { SEQUENCE } else { 0 },
    ))
}

pub(super) fn error<'a>(_: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, _: &Call<'_, 'a>) -> R<'a> {
    let message = text(&a, 0).filter(|m| !m.is_empty()).map_or_else(
        || "$error() function evaluated".to_owned(),
        |m| m.to_string(),
    );
    Err(Error::new("D3137").with("message", message))
}

pub(super) fn assert<'a>(_: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, _: &Call<'_, 'a>) -> R<'a> {
    if !matches!(arg(&a, 0), Value::Bool(true)) {
        let message = text(&a, 1).filter(|m| !m.is_empty()).map_or_else(
            || "$assert() statement failed".to_owned(),
            |m| m.to_string(),
        );
        return Err(Error::new("D3141").with("message", message));
    }
    Ok(Value::Undefined)
}

pub(super) fn type_<'a>(_: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, _: &Call<'_, 'a>) -> R<'a> {
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

pub(super) fn eval<'a>(ev: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, c: &Call<'_, 'a>) -> R<'a> {
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
    let mut ast =
        crate::parse(&source).map_err(|e| Error::new("D3120").with("value", e.message()))?;
    // The text may call `$eval` under a name bound outside it.
    ast.root
        .walk_mut(&mut |node| crate::process::mark_frame(node, true));
    // The text runs under the evaluation's limits, so reaching one is no error of the text's
    // own: it passes through, located at this call (ADR 0018, compatibility page).
    ev.eval(&ast.root, &input, c.env)
        .map_err(|e| match e.code() {
            "D1011" | "D1012" | "U1001" | "U1003" | "U1006" => e.unlocated(),
            _ => Error::new("D3121").with("value", e.message()),
        })
}
