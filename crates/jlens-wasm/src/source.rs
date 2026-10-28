//! The input of `evaluate`, read from the JavaScript object one level at a time (ADR 0010).
//!
//! Each level costs one call to a JavaScript helper, which reads it as `JSON.stringify` would:
//! it calls `toJSON`, unwraps boxed primitives, leaves out `undefined`, functions and symbols
//! (or writes `null` for them in arrays), and writes `null` for numbers that are not finite. A
//! cycle or a `BigInt` throws the `TypeError` that `JSON.stringify` throws, when the level that
//! holds it is read. The evaluation then fails, and `evaluate` rethrows that exception.
//!
//! Writing values one by one costs more per value than `JSON.stringify` and a parse in Rust, so
//! a level of more than [`WHOLE`] values is read whole, everything it holds included, through
//! `JSON.stringify`. A small level stays cheap to reach, and a large one costs what converting
//! it did before levels were read lazily. An index into an array reads the items it picks
//! alone, one call each, whatever the array's length, and a path step reads the one field it
//! looks up, one call per field, whatever the object's width.
//!
//! A long array of objects of which the expression reads at most [`WHOLE`] fields, and never
//! an object whole (`reads()`, ADR 0016), is read in one call without `JSON.stringify`
//! (`jlensItems`): each object with the fields read, which a path step then finds without
//! calling JavaScript again. Any other field is read alone, and the object whole as a level,
//! so the result never depends on the analysis ([`Plan`]).
//!
//! The arrays and objects of a level are the members of one [`Sources`], the list of their
//! nodes that the helper returns, so that each costs one allocation. What the expression reads
//! of each member, and the fields written with it, are kept in vectors that the levels of an
//! input share ([`Shared`]) and that the next input reuses.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use jlens_core::input::{Sink, Source, Sources};
use jlens_core::reads::Read;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
const CIRCULAR = "Converting circular structure to JSON";
const BIGINT = "Do not know how to serialize a BigInt";

// The value JSON.stringify writes for `v`, the value of `key`: what `toJSON` returns, and a
// boxed primitive unwrapped.
function unwrap(v, key) {
  if ((typeof v === "object" && v !== null) || typeof v === "bigint") {
    const toJSON = v.toJSON;
    if (typeof toJSON === "function") v = toJSON.call(v, key);
  }
  if (typeof v === "object" && v !== null) {
    if (v instanceof Number) v = Number(v);
    else if (v instanceof String) v = String(v);
    else if (v instanceof Boolean) v = v.valueOf();
    else if (v instanceof BigInt) throw new TypeError(BIGINT);
  }
  return v;
}

// Whether `v` is an object that is not an array.
function isObject(v) {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

// Throws as JSON.stringify does when `v` is the array or object of the node `up` or of one of
// its ancestors.
function checkCycle(v, up) {
  for (let n = up; n !== null; n = n.up) {
    if (n.v === v) throw new TypeError(CIRCULAR);
  }
}

// Reads values: with `kind` 0 the input itself (`node` is the value); with 1 the array or
// object of the node `node[index]`, the nodes of a level; with 2 the item `n` of that array.
// A node is {v, up}: an array or object and its parent's node, to detect cycles. Returns the
// numbers, starting with twice the number of values, plus 1 for the fields of an object (a
// second leading number made this function half as slow again in V8), then each value's tag
// and number; the strings and keys as one text (lengths in UTF-16 code units); and the nodes
// of the nested arrays and objects. In an object, each value is followed by the length of its
// key. An array or object of more than `n` values is returned as its JSON text instead (a
// cycle back to an ancestor passes through it again, so JSON.stringify sees it).
export function jlensRead(node, kind, index, n) {
  const nums = [2];
  let text = "";
  const nested = [];
  const up = kind === 0 ? null : node[index];
  const put = (v, key) => {
    v = unwrap(v, key);
    switch (typeof v) {
      case "string":
        nums.push(5, v.length);
        text += v;
        return true;
      case "number":
        if (isFinite(v)) nums.push(4, v);
        else nums.push(1);
        return true;
      case "boolean":
        nums.push(v ? 3 : 2);
        return true;
      case "bigint":
        throw new TypeError(BIGINT);
      case "object":
        if (v === null) {
          nums.push(1);
          return true;
        }
        checkCycle(v, up);
        nums.push(Array.isArray(v) ? 6 : 7);
        nested.push({ v, up });
        return true;
      default:
        return false;
    }
  };
  if (kind === 0) {
    put(node, "");
  } else if (kind === 2) {
    if (!put(up.v[n], String(n))) nums.push(1);
  } else if (Array.isArray(up.v)) {
    const items = up.v;
    if (items.length > n) return JSON.stringify(items);
    nums[0] = items.length * 2;
    for (let i = 0; i < items.length; i++) {
      if (!put(items[i], String(i))) nums.push(1);
    }
  } else {
    const fields = up.v;
    const keys = Object.keys(fields);
    if (keys.length > n) return JSON.stringify(fields);
    nums[0] = keys.length * 2 + 1;
    for (const key of keys) {
      if (put(fields[key], key)) {
        nums.push(key.length);
        text += key;
      }
    }
  }
  return [new Float64Array(nums), text, nested];
}

// Reads the items of the array of the node `nodes[index]`, of which the expression reads the
// fields `keys` of each object, as jlensRead does a level of `n` values or fewer. A longer
// array of objects (its first item one) is not returned as JSON text: each object is tag 8,
// its node, and the value of each key in turn, 0 for one JSON.stringify leaves out.
export function jlensItems(nodes, index, n, keys) {
  const up = nodes[index];
  const items = up.v;
  if (items.length <= n) return jlensRead(nodes, 1, index, n);
  const first = items[0];
  if (!isObject(first)) return JSON.stringify(items);
  const nums = [items.length * 2];
  let text = "";
  const nested = [];
  // Writes `v`, unwrapped, an array or object as a node under `parent`: false when
  // JSON.stringify leaves it out.
  const put = (v, parent) => {
    switch (typeof v) {
      case "string":
        nums.push(5, v.length);
        text += v;
        return true;
      case "number":
        if (isFinite(v)) nums.push(4, v);
        else nums.push(1);
        return true;
      case "boolean":
        nums.push(v ? 3 : 2);
        return true;
      case "bigint":
        throw new TypeError(BIGINT);
      case "object":
        if (v === null) {
          nums.push(1);
          return true;
        }
        checkCycle(v, parent);
        nums.push(Array.isArray(v) ? 6 : 7);
        nested.push({ v, up: parent });
        return true;
      default:
        return false;
    }
  };
  for (let i = 0; i < items.length; i++) {
    const v = unwrap(i === 0 ? first : items[i], String(i));
    if (!isObject(v)) {
      if (!put(v, up)) nums.push(1);
      continue;
    }
    checkCycle(v, up);
    const item = { v, up };
    nums.push(8);
    nested.push(item);
    for (const key of keys) {
      const has = Object.prototype.propertyIsEnumerable.call(v, key);
      if (!(has && put(unwrap(v[key], key), item))) nums.push(0);
    }
  }
  return [new Float64Array(nums), text, nested];
}

// Reads the field `key` of the object of the node `nodes[index]`, as JSON.stringify sees it:
// undefined when the object has no own enumerable field `key`, or when JSON.stringify leaves
// its value out; a string, a boolean or null as itself; a number, or null when it is not
// finite; an array or object as [6 or 7, its node], so that the node is member 1.
export function jlensField(nodes, index, key) {
  const up = nodes[index];
  const fields = up.v;
  if (!Object.prototype.propertyIsEnumerable.call(fields, key)) return undefined;
  const v = unwrap(fields[key], key);
  switch (typeof v) {
    case "string":
    case "boolean":
      return v;
    case "number":
      return isFinite(v) ? v : null;
    case "bigint":
      throw new TypeError(BIGINT);
    case "object":
      if (v === null) return null;
      checkCycle(v, up);
      return [Array.isArray(v) ? 6 : 7, { v, up }];
    default:
      return undefined;
  }
}

// The length of the array of the node `nodes[index]`.
export function jlensLength(nodes, index) {
  return nodes[index].v.length;
}
"#)]
extern "C" {
    #[wasm_bindgen(catch, js_name = jlensRead)]
    fn read_js(node: &JsValue, kind: u32, index: u32, n: u32) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = jlensItems)]
    fn read_items_js(
        nodes: &JsValue,
        index: u32,
        n: u32,
        keys: &JsValue,
    ) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = jlensField)]
    fn read_field_js(nodes: &JsValue, index: u32, key: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = jlensLength)]
    fn read_length(nodes: &JsValue, index: u32) -> Result<f64, JsValue>;
}

/// The kinds of reads of the helper: the input, a level, an item.
const INPUT: u32 = 0;
const LEVEL: u32 = 1;
const ITEM: u32 = 2;

/// The number of values above which a level is read whole, as JSON text.
const WHOLE: u32 = 16;

/// What the expression reads of a value it does not read whole (`reads()`, ADR 0016), made
/// ready for the reading of its levels.
pub struct Plan {
    /// The fields it reads, sorted by key, with what it reads of each one's value (`None` when
    /// it reads it whole).
    fields: Vec<(String, Option<Rc<Plan>>)>,
    /// Their keys, as a JavaScript array for the helper; `undefined` when there are more than
    /// [`WHOLE`].
    keys: JsValue,
}

impl Plan {
    /// What `read` reads, `None` when it reads the value whole.
    pub fn new(read: &Read) -> Option<Rc<Plan>> {
        jlens_core::stack::grow(|| {
            let fields: Vec<_> = read
                .fields()?
                .iter()
                .map(|(key, read)| (key.clone(), Plan::new(read)))
                .collect();
            let keys = match fields.len() <= WHOLE as usize {
                true => fields
                    .iter()
                    .map(|(key, _)| JsValue::from_str(key))
                    .collect::<js_sys::Array>()
                    .into(),
                false => JsValue::UNDEFINED,
            };
            Some(Rc::new(Plan { fields, keys }))
        })
    }

    /// The position of field `key` among the fields read.
    fn position(&self, key: &str) -> Option<usize> {
        self.fields
            .binary_search_by(|(k, _)| k.as_str().cmp(key))
            .ok()
    }

    /// What the expression reads of the value of field `key`.
    fn field(&self, key: &str) -> Option<Rc<Plan>> {
        self.position(key).and_then(|i| self.fields[i].1.clone())
    }
}

/// What the levels of one input share, so that a level costs one allocation.
struct Shared {
    /// The first exception a level throws, kept for `evaluate` to rethrow.
    exception: RefCell<Option<JsValue>>,
    /// The members of the levels read with a plan, each level's in a run from its `base`.
    members: RefCell<Vec<Member>>,
    /// The values of the fields written with their objects, in the order of their plans.
    held: RefCell<Vec<Held>>,
    /// The strings of `held`.
    text: RefCell<String>,
}

thread_local! {
    /// The vectors of the last input's `Shared`, emptied, so that an evaluation does not
    /// allocate them again.
    static SPARE: RefCell<(Vec<Member>, Vec<Held>, String)> = RefCell::default();
}

/// The most members whose vector is kept for the next input.
const SPARE_MEMBERS: usize = 1 << 12;

impl Shared {
    fn new() -> Rc<Shared> {
        let (members, held, text) = SPARE.with_borrow_mut(std::mem::take);
        Rc::new(Shared {
            exception: RefCell::default(),
            members: RefCell::new(members),
            held: RefCell::new(held),
            text: RefCell::new(text),
        })
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        let (mut members, mut held, mut text) = (
            std::mem::take(self.members.get_mut()),
            std::mem::take(self.held.get_mut()),
            std::mem::take(self.text.get_mut()),
        );
        if members.capacity() > SPARE_MEMBERS
            || held.capacity() > SPARE_MEMBERS * 4
            || text.capacity() > SPARE_MEMBERS * 16
        {
            return;
        }
        members.clear();
        held.clear();
        text.clear();
        // Dropped during the thread's teardown, when SPARE is gone, the vectors are freed.
        let _ = SPARE.try_with(|spare| *spare.borrow_mut() = (members, held, text));
    }
}

/// What the expression reads of a member, and where its fields start in `held` when it was
/// written with them.
struct Member {
    plan: Option<Rc<Plan>>,
    held: Option<u32>,
    /// Whether it is an array, whose items the plan reads.
    array: bool,
}

/// The value of a field written with its object.
#[derive(Clone, Copy)]
enum Held {
    /// A field the object lacks, or whose value JSON.stringify leaves out.
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    /// The range of the string in the shared text.
    String(u32, u32),
    /// The member that is the array or object.
    Array(u32),
    Object(u32),
}

/// The input of an evaluation, a JavaScript value.
pub struct JsSource {
    value: JsValue,
    plan: Option<Rc<Plan>>,
    shared: Rc<Shared>,
}

impl JsSource {
    /// The input of an evaluation, of which the expression reads what `plan` says.
    pub fn input(value: JsValue, plan: Option<Rc<Plan>>) -> JsSource {
        JsSource {
            value,
            plan,
            shared: Shared::new(),
        }
    }

    /// The exception that stopped the evaluation, when `err` is the failure of a level.
    pub fn exception(&self, err: &jlens_core::Error) -> Option<JsValue> {
        (err.code() == "U1004")
            .then(|| self.shared.exception.borrow().clone())
            .flatten()
    }
}

impl<'a> Source<'a> for JsSource {
    fn read(&self, sink: &mut Sink<'_, 'a>) -> Result<(), String> {
        let parts = read_js(&self.value, INPUT, 0, 0);
        let parts = keep(&self.shared, parts)?;
        write(parts, sink, &self.shared, self.plan.as_ref());
        Ok(())
    }
}

/// The arrays and objects of a level: the nodes the helper returned for them.
struct Nested {
    nodes: JsValue,
    shared: Rc<Shared>,
    this: Weak<Nested>,
    /// Where the level's members start in the shared members, when it was read with a plan:
    /// member `i` is at `base + i`, wrapping.
    base: Option<u32>,
}

impl Nested {
    /// A level whose members, read with a plan when `planned`, start at `base` in the shared
    /// members.
    fn new(nodes: JsValue, shared: &Rc<Shared>, planned: bool, base: u32) -> Rc<Nested> {
        Rc::new_cyclic(|this| Nested {
            nodes,
            shared: shared.clone(),
            this: this.clone(),
            base: planned.then_some(base),
        })
    }

    /// Runs `f` on member `index`, when the level was read with a plan.
    fn with_member<R>(&self, index: u32, f: impl FnOnce(&Member) -> Option<R>) -> Option<R> {
        let at = self.base?.wrapping_add(index);
        f(self.shared.members.borrow().get(at as usize)?)
    }

    /// What the expression reads of member `index`.
    fn plan(&self, index: u32) -> Option<Rc<Plan>> {
        self.with_member(index, |member| member.plan.clone())
    }

    /// Writes field `key` of member `index` to `sink` when it was written with the member:
    /// `false` when it was not. Out of line, as [`plan_member`].
    #[inline(never)]
    fn held_field(&self, index: u32, key: &str, sink: &mut Sink<'_, '_>) -> bool {
        let Some(held) = self.with_member(index, |member| {
            let at = member.plan.as_ref()?.position(key)?;
            let held = self.shared.held.borrow();
            held.get(member.held? as usize + at).copied()
        }) else {
            return false;
        };
        match held {
            Held::Undefined => {}
            Held::Null => sink.null(),
            Held::Bool(b) => sink.bool(b),
            Held::Number(n) => sink.number(n),
            Held::String(from, to) => {
                sink.string(&self.shared.text.borrow()[from as usize..to as usize]);
            }
            Held::Array(member) | Held::Object(member) => {
                // A member is read through its `Rc`, so the level is alive.
                let Some(this) = self.this.upgrade() else {
                    return false;
                };
                match held {
                    Held::Array(_) => sink.array_of(&this, member),
                    _ => sink.object_of(&this, member),
                }
            }
        }
        true
    }
}

impl<'a> Sources<'a> for Nested {
    fn read(&self, index: u32, sink: &mut Sink<'_, 'a>) -> Result<(), String> {
        let (plan, array) = self
            .with_member(index, |member| Some((member.plan.clone(), member.array)))
            .unwrap_or((None, false));
        // An array of which the expression reads a few fields of each item.
        let items = plan
            .as_ref()
            .filter(|plan| array && !plan.keys.is_undefined());
        let read = |n| match items {
            Some(plan) => read_items_js(&self.nodes, index, n, &plan.keys),
            None => read_js(&self.nodes, LEVEL, index, n),
        };
        let mut parts = keep(&self.shared, read(WHOLE))?;
        if let Some(text) = parts.as_string() {
            if sink.json_members(&text).is_ok() {
                return Ok(());
            }
            // Text that does not parse (a lone surrogate, or nesting deeper than serde_json
            // reads): the level is read again, value by value.
            parts = keep(&self.shared, read(u32::MAX))?;
        }
        write(parts, sink, &self.shared, plan.as_ref());
        Ok(())
    }

    fn read_field(&self, index: u32, key: &str, sink: &mut Sink<'_, 'a>) -> Result<bool, String> {
        if self.base.is_some() && self.held_field(index, key, sink) {
            return Ok(true);
        }
        let value = keep(&self.shared, read_field_js(&self.nodes, index, key))?;
        if let Some(n) = value.as_f64() {
            sink.number(n);
        } else if let Some(s) = value.as_string() {
            sink.string(&s);
        } else if let Some(b) = value.as_bool() {
            sink.bool(b);
        } else if value.is_null() {
            sink.null();
        } else if value.is_object() {
            // [6 or 7, the node]: the node is member 1 of a level of its own.
            let array = value.unchecked_ref::<js_sys::Array>().get(0).as_f64() == Some(6.0);
            let plan = self.plan(index).and_then(|plan| plan.field(key));
            let mut members = self.shared.members.borrow_mut();
            let base = (members.len() as u32).wrapping_sub(1);
            if plan.is_some() {
                members.push(Member {
                    plan: plan.clone(),
                    held: None,
                    array,
                });
            }
            drop(members);
            let level = Nested::new(value, &self.shared, plan.is_some(), base);
            if array {
                sink.array_of(&level, 1);
            } else {
                sink.object_of(&level, 1);
            }
        }
        Ok(true)
    }

    fn length(&self, index: u32) -> Option<usize> {
        // A length that throws, a proxy's, is left to the reading of the level.
        let length = read_length(&self.nodes, index).ok()?;
        (length >= 0.0 && length <= f64::from(u32::MAX)).then_some(length as usize)
    }

    fn read_item(&self, index: u32, item: usize, sink: &mut Sink<'_, 'a>) -> Result<bool, String> {
        let Ok(item) = u32::try_from(item) else {
            return Ok(false);
        };
        let parts = read_js(&self.nodes, ITEM, index, item);
        let parts = keep(&self.shared, parts)?;
        // The item is read as its array's plan says.
        write(parts, sink, &self.shared, self.plan(index).as_ref());
        Ok(true)
    }
}

/// The result of a call to the helper, keeping the exception it throws.
fn keep(shared: &Shared, result: Result<JsValue, JsValue>) -> Result<JsValue, String> {
    result.map_err(|e| {
        let message = match e.dyn_ref::<js_sys::Error>() {
            Some(error) => String::from(error.message()),
            None => e.as_string().unwrap_or_else(|| "an exception".into()),
        };
        shared.exception.borrow_mut().get_or_insert(e);
        message
    })
}

/// Writes the values the helper returned, `parts`, to `sink`. `plan` is what the expression
/// reads of the level: of each value when they are an array's items or the one value of an
/// item or the input, of the object when they are its fields.
fn write(parts: JsValue, sink: &mut Sink<'_, '_>, shared: &Rc<Shared>, plan: Option<&Rc<Plan>>) {
    let parts: js_sys::Array = parts.unchecked_into();
    let nums = parts
        .get(0)
        .unchecked_into::<js_sys::Float64Array>()
        .to_vec();
    let text = parts.get(1).as_string().unwrap_or_default();
    let mut nums = nums.into_iter();
    let head = nums.next().unwrap_or(0.0) as usize;
    sink.reserve(head / 2);
    let fields = head % 2 == 1;
    let mut text = Text::new(&text);
    // The members of the level, made when its first array or object is met. With a plan,
    // each member's is pushed to the shared members in the order of the members.
    let mut nested: Option<Rc<Nested>> = None;
    let mut next_nested = 0;
    while let Some(tag) = nums.next() {
        let value = match tag as u8 {
            1 => Entry::Null,
            2 => Entry::Bool(false),
            3 => Entry::Bool(true),
            4 => Entry::Number(nums.next().unwrap_or(f64::NAN)),
            5 => Entry::String(text.take(nums.next().unwrap_or(0.0) as usize)),
            6 => Entry::Array,
            7 => Entry::Object,
            8 => Entry::Some,
            _ => break,
        };
        let key = fields.then(|| text.take(nums.next().unwrap_or(0.0) as usize));
        if let Some(key) = key {
            sink.key(key);
        }
        match value {
            Entry::Null => sink.null(),
            Entry::Bool(b) => sink.bool(b),
            Entry::Number(n) => sink.number(n),
            Entry::String(s) => sink.string(s),
            Entry::Array | Entry::Object | Entry::Some => {
                let level = nested.get_or_insert_with(|| {
                    let base = shared.members.borrow().len() as u32;
                    Nested::new(parts.get(2), shared, plan.is_some(), base)
                });
                let member = next_nested;
                next_nested += 1;
                if let Some(plan) = plan {
                    let array = matches!(value, Entry::Array);
                    let some = matches!(value, Entry::Some);
                    let place = Place {
                        nums: &mut nums,
                        text: &mut text,
                        next_nested: &mut next_nested,
                    };
                    plan_member(shared, plan, key, array, some, place);
                }
                match value {
                    Entry::Array => sink.array_of(level, member),
                    _ => sink.object_of(level, member),
                }
            }
        }
    }
}

/// Where a level's decoding is: the numbers and text left, and the next member's number.
struct Place<'p, 't, I> {
    nums: &'p mut I,
    text: &'p mut Text<'t>,
    next_nested: &'p mut u32,
}

/// Pushes to the shared members what the expression reads of a member of a level read with
/// `plan`, the value of field `key` or an item, an `array` or an object; with `some` of its
/// fields, which follow, kept. Out of line, so that a level read without a plan stays as
/// cheap as before plans.
#[inline(never)]
fn plan_member<I: Iterator<Item = f64>>(
    shared: &Shared,
    plan: &Rc<Plan>,
    key: Option<&str>,
    array: bool,
    some: bool,
    place: Place<'_, '_, I>,
) {
    let plan = match key {
        Some(key) => plan.field(key),
        None => Some(plan.clone()),
    };
    let at = shared.members.borrow().len();
    let held = plan.clone().filter(|_| some);
    shared.members.borrow_mut().push(Member {
        plan,
        held: None,
        array,
    });
    if let Some(plan) = held {
        let start = hold(shared, &plan, place);
        shared.members.borrow_mut()[at].held = Some(start);
    }
}

/// Keeps the values of the fields `plan` lists, which follow in the numbers, the arrays and
/// objects among them as the next members: where they start in the shared `held`.
fn hold<I: Iterator<Item = f64>>(shared: &Shared, plan: &Plan, place: Place<'_, '_, I>) -> u32 {
    let Place {
        nums,
        text,
        next_nested,
    } = place;
    let start = shared.held.borrow().len() as u32;
    for (_, field) in &plan.fields {
        let value = match nums.next().unwrap_or(0.0) as u8 {
            1 => Held::Null,
            2 => Held::Bool(false),
            3 => Held::Bool(true),
            4 => Held::Number(nums.next().unwrap_or(f64::NAN)),
            5 => {
                let s = text.take(nums.next().unwrap_or(0.0) as usize);
                let mut kept = shared.text.borrow_mut();
                let from = kept.len() as u32;
                kept.push_str(s);
                Held::String(from, kept.len() as u32)
            }
            tag @ (6 | 7) => {
                let member = *next_nested;
                *next_nested += 1;
                shared.members.borrow_mut().push(Member {
                    plan: field.clone(),
                    held: None,
                    array: tag == 6,
                });
                match tag {
                    6 => Held::Array(member),
                    _ => Held::Object(member),
                }
            }
            _ => Held::Undefined,
        };
        shared.held.borrow_mut().push(value);
    }
    start
}

enum Entry<'t> {
    Null,
    Bool(bool),
    Number(f64),
    String(&'t str),
    Array,
    Object,
    /// An object written with the fields its plan lists.
    Some,
}

/// The strings of a level, taken in turn by their lengths in UTF-16 code units.
struct Text<'t> {
    rest: &'t str,
    ascii: bool,
}

impl<'t> Text<'t> {
    fn new(text: &'t str) -> Text<'t> {
        Text {
            rest: text,
            ascii: text.is_ascii(),
        }
    }

    fn take(&mut self, units: usize) -> &'t str {
        let end = if self.ascii {
            units.min(self.rest.len())
        } else {
            let mut seen = 0;
            self.rest
                .char_indices()
                .find_map(|(i, c)| {
                    let at = (seen >= units).then_some(i);
                    seen += c.len_utf16();
                    at
                })
                .unwrap_or(self.rest.len())
        };
        let (taken, rest) = self.rest.split_at(end);
        self.rest = rest;
        taken
    }
}

#[cfg(test)]
mod tests {
    use super::Text;

    #[test]
    fn text_is_taken_by_utf16_units() {
        let mut text = Text::new("aé𝄞bcd");
        assert_eq!(text.take(1), "a");
        assert_eq!(text.take(3), "é𝄞");
        assert_eq!(text.take(0), "");
        assert_eq!(text.take(2), "bc");
        assert_eq!(text.take(9), "d");
        let mut ascii = Text::new("abc");
        assert_eq!((ascii.take(2), ascii.take(5)), ("ab", "c"));
    }
}
