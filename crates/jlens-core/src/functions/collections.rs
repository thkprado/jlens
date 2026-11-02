//! Higher-order, array, object and miscellaneous functions.

use super::{Call, arg, boolean, hof_args, merge_sort, text};
use std::collections::HashMap;
use std::rc::Rc;

use crate::error::{Error, Result};
use crate::eval::Evaluator;
use crate::value::{self, DeepHasher, Map, SEQUENCE, Value};

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
    Ok(Value::array(
        distinct_items(items.items())?,
        if items.is_sequence() { SEQUENCE } else { 0 },
    ))
}

/// With this many items kept, comparing each item with all of them starts to cost more than
/// hashing it. An array with fewer distinct items than this never pays for a table, however long,
/// and neither does one with half as many items left, as the table hashes those kept as well.
const DISTINCT_BY_LIST: usize = 16;

/// The first of each group of equal items, in order.
fn distinct_items<'a>(items: &[Value<'a>]) -> Result<Vec<Value<'a>>> {
    let mut out = Vec::new();
    let mut rest = keep_by_list(&mut out, items, DISTINCT_BY_LIST)?;
    if out.len() == DISTINCT_BY_LIST && rest.len() > DISTINCT_BY_LIST / 2 {
        rest = keep_by_hash(&mut out, rest, items.len())?;
    }
    keep_by_list(&mut out, rest, usize::MAX)?;
    Ok(out)
}

/// Keeps each of `items` that equals no item kept, comparing it with each, until `limit` are
/// kept. Returns the items left.
fn keep_by_list<'i, 'a>(
    out: &mut Vec<Value<'a>>,
    items: &'i [Value<'a>],
    limit: usize,
) -> Result<&'i [Value<'a>]> {
    for (i, v) in items.iter().enumerate() {
        let mut seen = false;
        for earlier in out.iter() {
            if v.deep_eq(earlier)? {
                seen = true;
                break;
            }
        }
        if !seen {
            out.push(v.clone());
            if out.len() == limit {
                return Ok(&items[i + 1..]);
            }
        }
    }
    Ok(&[])
}

/// What [`keep_by_list`] does with no limit, comparing each item only with those kept that hash
/// the same. It stops at a value nested too deep to hash, returning the items from there: whether
/// comparing that one fails depends on what it meets, which only the list tells.
fn keep_by_hash<'i, 'a>(
    out: &mut Vec<Value<'a>>,
    items: &'i [Value<'a>],
    all: usize,
) -> Result<&'i [Value<'a>]> {
    let mut hasher = DeepHasher::new(all);
    // The last item kept with each hash, and for each item kept the one before it with its hash.
    let room = out.len() + items.len().min(3 * out.len());
    let mut last = HashMap::<u64, usize, foldhash::fast::RandomState>::with_capacity_and_hasher(
        room,
        Default::default(),
    );
    let mut before = Vec::with_capacity(room);
    for (i, kept) in out.iter().enumerate() {
        let Some(hash) = hasher.hash(kept) else {
            return Ok(items);
        };
        before.push(last.insert(hash, i));
    }
    for (i, v) in items.iter().enumerate() {
        let Some(hash) = hasher.hash(v) else {
            return Ok(&items[i..]);
        };
        let mut earlier = last.get(&hash).copied();
        let mut seen = false;
        while let Some(kept) = earlier {
            if v.deep_eq(&out[kept])? {
                seen = true;
                break;
            }
            earlier = before[kept];
        }
        if !seen {
            before.push(last.insert(hash, out.len()));
            out.push(v.clone());
        }
    }
    Ok(&[])
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::MAX_NESTING;
    use proptest::prelude::*;
    use serde_json::{Value as JsonValue, json};

    fn by_list<'a>(items: &[Value<'a>]) -> Result<Vec<Value<'a>>> {
        let mut out = Vec::new();
        keep_by_list(&mut out, items, usize::MAX)?;
        Ok(out)
    }

    /// The same values, by identity, the sign of a zero included.
    fn same<'a>(a: &[Value<'a>], b: &[Value<'a>]) -> bool {
        a.len() == b.len()
            && a.iter().zip(b).all(|pair| match pair {
                (Value::Number(x), Value::Number(y)) => x.to_bits() == y.to_bits(),
                (x, y) => x.strict_eq(y),
            })
    }

    /// What the list and the hash give for `items`, which must agree.
    fn agree(items: &[Value<'_>]) {
        match (by_list(items), distinct_items(items)) {
            (Ok(list), Ok(hash)) => assert!(same(&list, &hash), "different items"),
            (Err(list), Err(hash)) => assert_eq!(list.code(), hash.code()),
            (list, hash) => panic!("{:?} and {:?}", list.is_ok(), hash.is_ok()),
        }
    }

    fn numbers<'a>(n: usize) -> Vec<Value<'a>> {
        (0..n).map(|i| Value::Number(i as f64)).collect()
    }

    /// An array of arrays, `levels` of them.
    fn deep<'a>(levels: usize) -> Value<'a> {
        (1..levels).fold(Value::array(Vec::new(), 0), |value, _| {
            Value::array(vec![value], 0)
        })
    }

    /// An object that holds itself, twice.
    fn cycle<'a>() -> Value<'a> {
        let value = Value::object(Map::default());
        let Value::Object(object) = &value else {
            unreachable!();
        };
        for key in ["a", "b"] {
            object.borrow_mut().insert(Rc::from(key), value.clone());
        }
        value
    }

    fn item() -> impl Strategy<Value = JsonValue> {
        let leaf = prop_oneof![
            Just(json!(null)),
            any::<bool>().prop_map(|b| json!(b)),
            Just(json!(0.0)),
            Just(json!(-0.0)),
            (0..24).prop_map(|n| json!(n)),
            (0..24).prop_map(|n| json!(format!("s{n}"))),
        ];
        leaf.prop_recursive(3, 16, 3, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..3).prop_map(JsonValue::Array),
                prop::collection::vec((0..4, inner), 0..3).prop_map(|fields| {
                    let fields = fields.into_iter().map(|(k, v)| (format!("k{k}"), v));
                    JsonValue::Object(fields.collect())
                }),
            ]
        })
    }

    /// `value` with the fields of each object in the other order and each zero of the other sign.
    fn mirrored(value: &JsonValue) -> JsonValue {
        match value {
            JsonValue::Number(n) if n.as_f64() == Some(0.0) => {
                let negative = n.as_f64().is_some_and(f64::is_sign_negative);
                json!(if negative { 0.0 } else { -0.0 })
            }
            JsonValue::Array(items) => JsonValue::Array(items.iter().map(mirrored).collect()),
            JsonValue::Object(fields) => {
                let fields = fields.iter().rev().map(|(k, v)| (k.clone(), mirrored(v)));
                JsonValue::Object(fields.collect())
            }
            other => other.clone(),
        }
    }

    proptest! {
        #[test]
        fn the_hash_gives_what_the_list_gives(items in prop::collection::vec(item(), 0..64)) {
            let items: Vec<Value<'_>> = items.iter().map(Value::from_json).collect();
            agree(&items);
        }

        #[test]
        fn equal_values_hash_the_same(value in item()) {
            let (one, other) = (Value::from_json(&value), Value::from_json(&mirrored(&value)));
            prop_assert!(one.deep_eq(&other).unwrap());
            // With a budget, and past it from the first array, where hashes are remembered.
            for mut hasher in [DeepHasher::new(1), DeepHasher::new(0)] {
                prop_assert_eq!(hasher.hash(&one), hasher.hash(&other));
                prop_assert_eq!(hasher.hash(&one), hasher.hash(&other));
            }
        }
    }

    #[test]
    fn items_keep_their_order_past_the_table() {
        let mut items = numbers(1_000);
        items.extend(numbers(1_000).into_iter().rev());
        let kept = distinct_items(&items).unwrap();
        assert!(same(&kept, &numbers(1_000)));
    }

    /// One array, held twice at each of 200 levels: 2^200 paths, which [`Value::deep_eq`] never
    /// takes, and neither may the hash.
    #[test]
    fn an_array_held_many_times_is_hashed_once() {
        let shared = (0..200).fold(Value::Null, |value, _| {
            Value::array(vec![value.clone(), value], 0)
        });
        let pair = |i: usize| Value::array(vec![Value::Number(i as f64), shared.clone()], 0);
        let items: Vec<Value<'_>> = (0..40).chain(0..40).map(pair).collect();
        assert_eq!(distinct_items(&items).unwrap().len(), 40);
    }

    #[test]
    fn hashing_stops_at_the_nesting_limit() {
        let (fits, too_deep, cycle) = (deep(MAX_NESTING), deep(MAX_NESTING + 1), cycle());
        for budget in [1, 0] {
            let mut hasher = DeepHasher::new(budget);
            assert!(hasher.hash(&fits).is_some());
            assert!(hasher.hash(&too_deep).is_none());
            assert!(hasher.hash(&cycle).is_none());
        }
    }

    /// A hash remembered is good at the level it was walked from, not at any level.
    #[test]
    fn a_remembered_array_counts_its_levels() {
        let inner = deep(MAX_NESTING);
        let mut hasher = DeepHasher::new(0);
        assert!(hasher.hash(&inner).is_some());
        assert!(hasher.hash(&inner).is_some());
        assert!(hasher.hash(&Value::array(vec![inner], 0)).is_none());
    }

    /// Whether comparing a value too deep fails depends on what it meets, as in the list.
    #[test]
    fn a_value_too_deep_goes_back_to_the_list() {
        let too_deep = || deep(MAX_NESTING + 2);
        let with = |extra: Vec<Value<'static>>| {
            let mut items = numbers(40);
            items.extend(extra);
            items.extend(numbers(50));
            items
        };
        // Alone among numbers, it compares with nothing that fails.
        let once = with(vec![too_deep()]);
        assert_eq!(distinct_items(&once).unwrap().len(), 51);
        agree(&once);
        // Held twice, it is one value.
        let shared = too_deep();
        let twice = with(vec![shared.clone(), shared]);
        assert_eq!(distinct_items(&twice).unwrap().len(), 51);
        agree(&twice);
        // Two of them cannot be compared.
        let two = with(vec![too_deep(), too_deep()]);
        assert_eq!(distinct_items(&two).unwrap_err().code(), "U1001");
        agree(&two);
        // The same among the items kept by the list.
        let mut first = vec![too_deep()];
        first.extend(numbers(40));
        assert_eq!(distinct_items(&first).unwrap().len(), 41);
        agree(&first);
    }

    #[test]
    fn cycles_go_back_to_the_list() {
        let shared = cycle();
        let mut items = numbers(40);
        items.extend([shared.clone(), shared]);
        assert_eq!(distinct_items(&items).unwrap().len(), 41);
        agree(&items);
        let mut items = numbers(40);
        items.extend([cycle(), cycle()]);
        assert_eq!(distinct_items(&items).unwrap_err().code(), "U1001");
        agree(&items);
    }
}
