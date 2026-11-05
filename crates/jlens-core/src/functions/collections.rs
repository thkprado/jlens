//! Higher-order, array, object and miscellaneous functions.

use super::{Call, arg, boolean, hof_args, merge_sort, text};
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::rc::Rc;

use crate::error::{Error, Result};
use crate::eval::Evaluator;
use crate::value::{self, DeepHasher, Map, SEQUENCE, Value, write_scalar};

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
            Value::string(key.clone()),
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
/// How many keys the objects of an array give before those found are looked up in a set rather
/// than in the list.
const KEYS_LISTED: usize = 16;

fn keys_of<'a>(v: &Value<'a>, out: &mut Vec<Rc<str>>) {
    match v {
        // The keys of one object are distinct.
        Value::Object(o) => out.extend(o.borrow().keys().cloned()),
        Value::Array(a) => {
            let mut found: Option<HashSet<Rc<str>, foldhash::fast::RandomState>> = None;
            let Ok(()) = value::for_each_leaf(a.items(), |item| {
                if let Value::Object(o) = item {
                    for k in o.borrow().keys() {
                        let new = match &mut found {
                            Some(found) => found.insert(k.clone()),
                            None => !out.contains(k),
                        };
                        if new {
                            out.push(k.clone());
                            if found.is_none() && out.len() > KEYS_LISTED {
                                found = Some(out.iter().cloned().collect());
                            }
                        }
                    }
                }
                Ok::<_, std::convert::Infallible>(())
            });
        }
        _ => {}
    }
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
        let args = hof_args(&func, value, Value::string(key.clone()), obj.clone());
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
    let worth_a_table = |rest: &[Value<'a>]| rest.len() > DISTINCT_BY_LIST / 2;
    if out.len() == DISTINCT_BY_LIST && worth_a_table(rest) && items.len() < Kept::MOST {
        rest = keep_by_first_fields(&mut out, rest)?;
        if worth_a_table(rest) {
            rest = keep_by_hash(&mut out, rest, items.len())?;
        }
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
        if !is_kept(out, v)? {
            out.push(v.clone());
            if out.len() == limit {
                return Ok(&items[i + 1..]);
            }
        }
    }
    Ok(&[])
}

/// Whether `v` equals an item of `kept`, by [`Value::deep_eq`]. A scalar equals only a scalar of
/// its kind and compares as [`Value::strict_eq`] does, never failing and reading nothing of an
/// array or an object, so numbers and strings are compared in a loop of their own: through
/// `deep_eq`, 100,000 numbers of eight values took 2.19 ms against a `Document`, and take
/// 0.31 ms here.
#[inline]
fn is_kept<'a>(kept: &[Value<'a>], v: &Value<'a>) -> Result<bool> {
    match v {
        Value::Number(x) => Ok(kept.iter().any(|k| matches!(k, Value::Number(y) if y == x))),
        Value::String(x) => Ok(kept.iter().any(|k| matches!(k, Value::String(y) if y == x))),
        Value::Array(_) | Value::Object(_) => {
            for k in kept {
                if v.deep_eq(k)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(kept.iter().any(|k| v.strict_eq(k))),
    }
}

/// What [`keep_by_list`] does with no limit, comparing each item only with those kept that hold
/// the same values in the first fields of the first item kept ([`FirstFields`]). Those comparisons
/// are some of the list's, in its order, and the rest are false with nothing below those fields
/// read, so the result and the errors are the list's. It reads of an item the levels comparing
/// reads when it meets an item that starts the same way, where [`keep_by_hash`] reads it whole.
///
/// It stops at an item that does not start with those fields, and when the fields leave
/// [`DISTINCT_BY_LIST`] items to compare one with, returning the items from there.
fn keep_by_first_fields<'i, 'a>(
    out: &mut Vec<Value<'a>>,
    items: &'i [Value<'a>],
) -> Result<&'i [Value<'a>]> {
    let Some((fields, mut table)) = FirstFields::of(out, items.len()) else {
        return Ok(items);
    };
    keep_by_table(out, items, &mut table, DISTINCT_BY_LIST, |v| {
        fields.of_item(v)
    })
}

/// What [`keep_by_list`] does with no limit, comparing each item only with those kept that hash
/// the same. It stops at a value it cannot hash, nested too deep or holding a level of a source
/// that cannot be read, returning the items from there: whether comparing that one fails depends
/// on what it meets, which only the list tells.
fn keep_by_hash<'i, 'a>(
    out: &mut Vec<Value<'a>>,
    items: &'i [Value<'a>],
    all: usize,
) -> Result<&'i [Value<'a>]> {
    let mut hasher = DeepHasher::new(all);
    let mut table = Kept::new(out.len(), items.len());
    for kept in out.iter() {
        let Some(hash) = hasher.hash(kept) else {
            return Ok(items);
        };
        table.add(Some(hash));
    }
    keep_by_table(out, items, &mut table, usize::MAX, |v| hasher.hash(v))
}

/// Keeps each of `items` that equals none of those kept that `table` holds with its hash. Stops
/// at an item with no hash, or that differs from `limit` items with its hash, returning the items
/// from that one. Inlined with the hash it is given: a call for each item costs arrays of numbers
/// 5 ns per item, 7% of what they take.
#[inline(always)]
fn keep_by_table<'i, 'a>(
    out: &mut Vec<Value<'a>>,
    items: &'i [Value<'a>],
    table: &mut Kept,
    limit: usize,
    mut hash: impl FnMut(&Value<'a>) -> Option<u64>,
) -> Result<&'i [Value<'a>]> {
    for (i, v) in items.iter().enumerate() {
        let Some(hash) = hash(v) else {
            return Ok(&items[i..]);
        };
        let mut compared = 0;
        let mut earlier = table.first(hash);
        let mut seen = false;
        while let Some(kept) = earlier {
            if v.deep_eq(&out[kept as usize])? {
                seen = true;
                break;
            }
            compared += 1;
            if compared == limit {
                return Ok(&items[i..]);
            }
            earlier = table.after(kept);
        }
        if !seen {
            table.add(Some(hash));
            out.push(v.clone());
        }
    }
    Ok(&[])
}

/// The items kept by a hash, each hash's in the order they were kept. Indexes are 32 bits, which
/// on 100,000 numbers takes 19% off what `usize` costs: an array of [`Kept::MOST`] items or more
/// stays with the list.
struct Kept {
    /// The first and the last item kept with each hash.
    ends: HashMap<u64, (u32, u32), foldhash::fast::RandomState>,
    /// For each item kept, the next one with its hash, or [`Kept::LAST`].
    next: Vec<u32>,
}

impl Kept {
    /// How many items an array may have to have a table.
    const MOST: usize = u32::MAX as usize;
    /// What [`Kept::next`] holds for the last item with a hash, never an index.
    const LAST: u32 = u32::MAX;

    /// A table for `kept` items, reserved for a few times as many of the `left` to come.
    fn new(kept: usize, left: usize) -> Kept {
        let room = kept + left.min(3 * kept);
        Kept {
            ends: HashMap::with_capacity_and_hasher(room, Default::default()),
            next: Vec::with_capacity(room),
        }
    }

    /// Takes out every item, keeping the room.
    fn clear(&mut self) {
        self.ends.clear();
        self.next.clear();
    }

    /// How many hashes the items kept have.
    fn hashes(&self) -> usize {
        self.ends.len()
    }

    /// The first item kept with `hash`.
    fn first(&self, hash: u64) -> Option<u32> {
        self.ends.get(&hash).map(|&(first, _)| first)
    }

    /// The item kept after `item` with its hash.
    fn after(&self, item: u32) -> Option<u32> {
        Some(self.next[item as usize]).filter(|&next| next != Kept::LAST)
    }

    /// Adds the next item kept, which no item is looked up with when it has no hash.
    fn add(&mut self, hash: Option<u64>) {
        let item = self.next.len() as u32;
        self.next.push(Kept::LAST);
        if let Some(hash) = hash
            && let Some((first, last)) = self.ends.insert(hash, (item, item))
        {
            self.next[last as usize] = item;
            self.ends.insert(hash, (first, item));
        }
    }
}

/// The first values that comparing an item of the array reads, found in the first item kept: the
/// scalars an array or an object starts with, and when an array or an object follows them, the
/// same within it, one level below the other. Comparing an item that starts the same way with
/// another reads those first, the size of each level before its values: if the other holds
/// something else in one of them, the comparison is false, with nothing else of either item read.
/// Looked up by key in the items kept, so that the order of their fields does not count.
///
/// A level below the first is taken only when the items kept, told apart by the levels above it,
/// fall in half as many groups or fewer: an item then shares those levels with some of them, and
/// comparing it with those reads the level below too. Items whose first fields differ, the common
/// shape of a record, are hashed by their first level alone, which reads what comparing reads.
struct FirstFields {
    /// The first level, apart, so that one level takes no list.
    first: Level,
    below: Vec<Level>,
    state: foldhash::fast::RandomState,
}

/// A level of [`FirstFields`]: the scalars an array or an object starts with, and the array or
/// object after them, which the next level reads if there is one.
struct Level {
    /// The keys of an object's fields, that of the array or object after the scalars last; `None`
    /// for an array.
    keys: Option<Vec<Rc<str>>>,
    /// How many of its first values are scalars.
    scalars: usize,
}

/// Why an item kept has no hash of its first fields.
enum Unfit {
    /// It differs from every item that starts with them.
    Apart,
    /// A level of it cannot be read.
    Unread,
}

impl FirstFields {
    /// How many scalars it looks at.
    const MOST: usize = 8;
    /// How many levels it reads.
    const LEVELS: usize = 8;
    /// How much of a string it hashes: a long one is told from another by its length, or when the
    /// items with one hash are compared.
    const STRING: usize = 64;

    /// The first fields of `kept[0]`, with as many levels as tell `kept` apart, and a table of
    /// `kept` by them, with room for some of the `left` to come; `None` when `kept[0]` is not an
    /// array or an object or holds no scalar where they start, or when a level of `kept` cannot be
    /// read.
    fn of(kept: &[Value<'_>], left: usize) -> Option<(FirstFields, Kept)> {
        let mut scalars = 0;
        let (first, mut next) = Level::of(&kept[0], &mut scalars)?;
        let mut fields = FirstFields {
            first,
            below: Vec::new(),
            state: Default::default(),
        };
        let mut table = Kept::new(kept.len(), left);
        loop {
            table.clear();
            // Those that differ from every item that starts the same way count as a group each.
            let mut apart = 0;
            for item in kept {
                let hash = fields.of_kept(item)?;
                apart += usize::from(hash.is_none());
                table.add(hash);
            }
            let few_groups = 2 * (table.hashes() + apart) <= kept.len();
            let deeper = match next.take() {
                Some(below) if few_groups && 1 + fields.below.len() < Self::LEVELS => {
                    // When the level below cannot be read, the levels above it do.
                    Level::of(&below, &mut scalars).map(|(level, next)| {
                        fields.below.push(level);
                        next
                    })
                }
                _ => None,
            };
            match deeper {
                Some(below) => next = below,
                None => return (scalars > 0).then_some((fields, table)),
            }
        }
    }

    /// The hash of an item, or `None` when it does not start the same way, in its own order, or
    /// cannot be read. It runs for each item, in the table's loop: one level of an object, the
    /// common case, has a loop of its own, as the loop over the levels costs records of three
    /// fields 2 ns per item, 4%.
    #[inline(always)]
    fn of_item(&self, item: &Value<'_>) -> Option<u64> {
        if self.below.is_empty()
            && let (Value::Object(object), Some(keys)) = (item, &self.first.keys)
        {
            let map = object.try_borrow()?;
            let mut fields = map.iter();
            let mut hasher = self.state.build_hasher();
            for key in &keys[..self.first.scalars] {
                let (field, value) = fields.next()?;
                if field != key || is_nested(value) {
                    return None;
                }
                write_scalar(&mut hasher, value, Self::STRING);
            }
            return Some(hasher.finish());
        }
        let mut hasher = self.state.build_hasher();
        let mut held: Value<'_>;
        let mut value = item;
        for i in 0..=self.below.len() {
            let level = if i == 0 {
                &self.first
            } else {
                &self.below[i - 1]
            };
            let deeper = i < self.below.len();
            let below = match (value, &level.keys) {
                (Value::Array(array), None) => {
                    let items = array.try_items()?;
                    let (scalars, rest) = items.split_at_checked(level.scalars)?;
                    for item in scalars {
                        if is_nested(item) {
                            return None;
                        }
                        write_scalar(&mut hasher, item, Self::STRING);
                    }
                    if !deeper {
                        break;
                    }
                    rest.first()?.clone()
                }
                (Value::Object(object), Some(keys)) => {
                    let map = object.try_borrow()?;
                    let mut fields = map.iter();
                    for key in &keys[..level.scalars] {
                        let (field, value) = fields.next()?;
                        if field != key || is_nested(value) {
                            return None;
                        }
                        write_scalar(&mut hasher, value, Self::STRING);
                    }
                    if !deeper {
                        break;
                    }
                    let (field, value) = fields.next()?;
                    if *field != keys[level.scalars] {
                        return None;
                    }
                    value.clone()
                }
                _ => return None,
            };
            held = below;
            value = &held;
        }
        Some(hasher.finish())
    }

    /// The hash of an item kept, by key, or `Some(None)` when no item that starts the same way can
    /// equal it; `None` when it cannot be read.
    fn of_kept(&self, item: &Value<'_>) -> Option<Option<u64>> {
        let mut hasher = self.state.build_hasher();
        match Self::kept_level(&self.first, &self.below, item, &mut hasher) {
            Ok(()) => Some(Some(hasher.finish())),
            Err(Unfit::Apart) => Some(None),
            Err(Unfit::Unread) => None,
        }
    }

    /// Hashes `value` by `level` and the `levels` below it, looking its fields up by key.
    fn kept_level(
        level: &Level,
        levels: &[Level],
        value: &Value<'_>,
        hasher: &mut impl Hasher,
    ) -> Result<(), Unfit> {
        let scalar = |hasher: &mut _, value: Option<&Value<'_>>| match value {
            Some(value) if !is_nested(value) => {
                write_scalar(hasher, value, Self::STRING);
                Ok(())
            }
            _ => Err(Unfit::Apart),
        };
        let deeper = |hasher: &mut _, below: Option<&Value<'_>>| match (levels, below) {
            ([level, levels @ ..], Some(below)) => Self::kept_level(level, levels, below, hasher),
            _ => Err(Unfit::Apart),
        };
        match (value, &level.keys) {
            (Value::Array(array), None) => {
                let items = array.try_items().ok_or(Unfit::Unread)?;
                for i in 0..level.scalars {
                    scalar(hasher, items.get(i))?;
                }
                match levels {
                    [] => Ok(()),
                    _ => deeper(hasher, items.get(level.scalars)),
                }
            }
            (Value::Object(object), Some(keys)) => {
                let map = object.try_borrow().ok_or(Unfit::Unread)?;
                for key in &keys[..level.scalars] {
                    scalar(hasher, map.get(key))?;
                }
                match levels {
                    [] => Ok(()),
                    _ => deeper(hasher, map.get(&keys[level.scalars])),
                }
            }
            _ => Err(Unfit::Apart),
        }
    }
}

impl Level {
    /// The level of `value`, counting its scalars in `scalars`, and the array or object that
    /// follows them; `None` when `value` is neither or cannot be read.
    fn of<'a>(value: &Value<'a>, scalars: &mut usize) -> Option<(Level, Option<Value<'a>>)> {
        let mut level = Level {
            keys: None,
            scalars: 0,
        };
        let mut below = None;
        match value {
            Value::Array(array) => {
                for item in array.try_items()? {
                    if *scalars == FirstFields::MOST {
                        break;
                    }
                    if is_nested(item) {
                        below = Some(item.clone());
                        break;
                    }
                    level.scalars += 1;
                    *scalars += 1;
                }
            }
            Value::Object(object) => {
                let mut keys = Vec::new();
                for (key, value) in object.try_borrow()?.iter() {
                    if *scalars == FirstFields::MOST {
                        break;
                    }
                    keys.push(key.clone());
                    if is_nested(value) {
                        below = Some(value.clone());
                        break;
                    }
                    level.scalars += 1;
                    *scalars += 1;
                }
                level.keys = Some(keys);
            }
            _ => return None,
        }
        Some((level, below))
    }
}

/// Whether `value` is an array or an object, which comparing may read further.
fn is_nested(value: &Value<'_>) -> bool {
    matches!(value, Value::Array(_) | Value::Object(_))
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

    /// An object that mostly starts with scalars and holds more below them, its fields in either
    /// order, with some of them left out or nested: many items, few of them equal, so that they
    /// get past the list.
    fn record() -> impl Strategy<Value = JsonValue> {
        let scalar = prop_oneof![
            8 => (0..48).prop_map(|n| json!(n)),
            1 => Just(json!(0.0)),
            1 => Just(json!(-0.0)),
            1 => Just(json!(null)),
            // Longer than what is hashed of a string, told apart by their ends.
            2 => (0..4).prop_map(|n| json!(format!("{}{n}", "x".repeat(70)))),
            1 => (0..4).prop_map(|n| json!(vec![n])),
        ];
        let rest = prop_oneof![
            (0..3).prop_map(|n| json!(n)),
            (0..3).prop_map(|n| json!([n, {"a": n}])),
            (0..3).prop_map(|n| json!({"a": [n]})),
        ];
        (scalar, 0..2, rest, 0..24u8).prop_map(|(id, group, rest, shape)| match shape {
            0 => json!({"rest": rest, "group": group, "id": id}),
            1 => json!({"id": id, "group": group}),
            2 => json!({"group": group, "id": id, "rest": rest}),
            _ => json!({"id": id, "group": group, "rest": rest}),
        })
    }

    /// A value that mostly starts with a field few values share, and differs in a level below it:
    /// in an array or an object, its fields in either order, a level left out or a scalar in its
    /// place, so that the first fields follow into levels that some items do not have.
    fn nested_record() -> impl Strategy<Value = JsonValue> {
        let id = prop_oneof![
            6 => (0..40).prop_map(|n| json!(n)),
            1 => Just(json!(0.0)),
            1 => Just(json!(-0.0)),
            1 => (0..4).prop_map(|n| json!(format!("{}{n}", "x".repeat(70)))),
            1 => (0..3).prop_map(|n| json!([n])),
        ];
        (0..2, id, 0..2, 0..14u8).prop_map(|(kind, id, n, shape)| match shape {
            0 => json!({"kind": kind, "list": [{"a": id, "b": n}, {"a": n}]}),
            1 => json!({"kind": kind, "list": [{"b": n, "a": id}]}),
            2 => json!({"list": [{"a": id, "b": n}], "kind": kind}),
            3 => json!([kind, [id, n]]),
            4 => json!([kind, [id]]),
            5 => json!({"kind": kind, "meta": {"v": n, "list": [id]}}),
            6 => json!({"kind": kind, "list": []}),
            7 => json!({"kind": kind, "list": id}),
            8 => json!({"kind": kind}),
            9 => json!({"kind": kind, "list": [[id], n]}),
            _ => json!({"kind": kind, "list": [{"a": id, "b": n}]}),
        })
    }

    /// Items that start with `id`, and hold a value below it that comparing them may not reach.
    fn with_below<'a>(
        ids: impl Iterator<Item = usize>,
        below: impl Fn() -> Value<'a>,
    ) -> Vec<Value<'a>> {
        ids.map(|id| {
            let mut object = Map::default();
            object.insert(Rc::from("id"), Value::Number(id as f64));
            object.insert(Rc::from("below"), below());
            Value::object(object)
        })
        .collect()
    }

    /// `items`, the first `kept` by the list, and what the first fields leave of the rest.
    fn left_by_first_fields(items: &[Value<'_>], kept: usize) -> usize {
        let mut out = Vec::new();
        let rest = keep_by_list(&mut out, items, kept).unwrap();
        keep_by_first_fields(&mut out, rest).unwrap().len()
    }

    proptest! {
        #[test]
        fn the_hash_gives_what_the_list_gives(items in prop::collection::vec(item(), 0..64)) {
            let items: Vec<Value<'_>> = items.iter().map(Value::from_json).collect();
            agree(&items);
        }

        #[test]
        fn the_first_fields_give_what_the_list_gives(
            items in prop::collection::vec(record(), 0..96),
        ) {
            let items: Vec<Value<'_>> = items.iter().map(Value::from_json).collect();
            agree(&items);
        }

        #[test]
        fn the_levels_below_give_what_the_list_gives(
            items in prop::collection::vec(nested_record(), 0..96),
        ) {
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

    /// The first fields tell the items apart, so that a value below them is never compared, and
    /// comparing fails only for the items that hold the same in those fields.
    #[test]
    fn a_value_too_deep_below_the_first_fields_is_reached_as_in_the_list() {
        let too_deep = || deep(MAX_NESTING + 2);
        let apart = with_below(0..40, too_deep);
        assert_eq!(distinct_items(&apart).unwrap().len(), 40);
        agree(&apart);
        let two_alike = with_below((0..40).chain([7]), too_deep);
        assert_eq!(distinct_items(&two_alike).unwrap_err().code(), "U1001");
        agree(&two_alike);
    }

    #[test]
    fn the_first_fields_settle_the_items_that_start_with_them() {
        let items = with_below(0..40, || Value::array(numbers(3), 0));
        assert_eq!(left_by_first_fields(&items, 16), 0);
    }

    #[test]
    fn the_first_fields_stop_at_an_item_that_does_not_start_with_them() {
        let mut items = with_below(0..30, || Value::Null);
        items.push(Value::Number(1.0));
        items.extend(with_below(30..40, || Value::Null));
        assert_eq!(left_by_first_fields(&items, 16), 11);
        agree(&items);
    }

    /// One `id` and an array below it that starts with `first` and ends with `i`.
    fn shared_id<'a>(i: usize, first: &[f64]) -> Value<'a> {
        let below = || {
            let mut items: Vec<_> = first.iter().map(|&n| Value::Number(n)).collect();
            items.push(Value::Number(i as f64));
            Value::array(items, 0)
        };
        with_below(0..1, below).remove(0)
    }

    #[test]
    fn the_first_fields_follow_into_a_level_that_tells_the_items_apart() {
        let items: Vec<Value<'_>> = (0..40).map(|i| shared_id(i, &[])).collect();
        assert_eq!(left_by_first_fields(&items, 16), 0);
        assert_eq!(distinct_items(&items).unwrap().len(), 40);
    }

    #[test]
    fn the_first_fields_give_up_when_they_tell_too_few_apart() {
        // The `id` and the next seven scalars are the same in all, and 16 are kept with them.
        let first = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
        let items: Vec<Value<'_>> = (0..40).map(|i| shared_id(i, &first)).collect();
        assert_eq!(left_by_first_fields(&items, 16), 24);
        assert_eq!(distinct_items(&items).unwrap().len(), 40);
    }

    #[test]
    fn the_first_fields_stay_on_a_level_that_tells_the_items_apart() {
        let items = with_below(0..40, || Value::array(numbers(3), 0));
        let mut out = Vec::new();
        keep_by_list(&mut out, &items, 16).unwrap();
        assert_eq!(FirstFields::of(&out, 24).unwrap().0.below.len() + 1, 1);
        let items: Vec<Value<'_>> = (0..40).map(|i| shared_id(i, &[])).collect();
        let mut out = Vec::new();
        keep_by_list(&mut out, &items, 16).unwrap();
        assert_eq!(FirstFields::of(&out, 24).unwrap().0.below.len() + 1, 2);
    }

    /// Below a field all the items share, comparing fails only for the items that hold the same
    /// in the level below it.
    #[test]
    fn a_value_too_deep_below_a_shared_field_is_reached_as_in_the_list() {
        let item = |i: usize| {
            let below = || Value::array(vec![Value::Number(i as f64), deep(MAX_NESTING + 2)], 0);
            with_below(0..1, below).remove(0)
        };
        let apart: Vec<Value<'_>> = (0..40).map(item).collect();
        assert_eq!(distinct_items(&apart).unwrap().len(), 40);
        agree(&apart);
        let two_alike: Vec<Value<'_>> = (0..40).chain([7]).map(item).collect();
        assert_eq!(distinct_items(&two_alike).unwrap_err().code(), "U1001");
        agree(&two_alike);
    }

    #[test]
    fn items_that_do_not_start_with_scalars_have_no_first_fields() {
        let nested = with_below(0..1, || Value::Null).remove(0);
        let Value::Object(object) = &nested else {
            unreachable!();
        };
        object
            .borrow_mut()
            .shift_insert(0, Rc::from("list"), Value::array(Vec::new(), 0));
        assert!(FirstFields::of(&[nested], 0).is_none());
        assert!(FirstFields::of(&[Value::Number(1.0)], 0).is_none());
        assert!(FirstFields::of(&[Value::object(Map::default())], 0).is_none());
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
