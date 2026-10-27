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
//! The arrays and objects of a level are the members of one [`Sources`], the list of their
//! nodes that the helper returns, so that each costs one allocation.

use std::cell::RefCell;
use std::rc::Rc;

use jlens_core::input::{Sink, Source, Sources};
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

/// The first exception a level throws, kept for `evaluate` to rethrow.
type Exception = Rc<RefCell<Option<JsValue>>>;

/// The input of an evaluation, a JavaScript value.
pub struct JsSource {
    value: JsValue,
    exception: Exception,
}

impl JsSource {
    /// The input of an evaluation.
    pub fn input(value: JsValue) -> JsSource {
        JsSource {
            value,
            exception: Exception::default(),
        }
    }

    /// The exception that stopped the evaluation, when `err` is the failure of a level.
    pub fn exception(&self, err: &jlens_core::Error) -> Option<JsValue> {
        (err.code() == "U1004")
            .then(|| self.exception.borrow().clone())
            .flatten()
    }
}

impl<'a> Source<'a> for JsSource {
    fn read(&self, sink: &mut Sink<'_, 'a>) -> Result<(), String> {
        let parts = keep(&self.exception, read_js(&self.value, INPUT, 0, 0))?;
        write(parts, sink, &self.exception);
        Ok(())
    }
}

/// The arrays and objects of a level: the nodes the helper returned for them.
struct Nested {
    nodes: JsValue,
    exception: Exception,
}

impl Nested {
    fn new(nodes: JsValue, exception: &Exception) -> Rc<Nested> {
        Rc::new(Nested {
            nodes,
            exception: exception.clone(),
        })
    }
}

impl<'a> Sources<'a> for Nested {
    fn read(&self, index: u32, sink: &mut Sink<'_, 'a>) -> Result<(), String> {
        let mut parts = keep(&self.exception, read_js(&self.nodes, LEVEL, index, WHOLE))?;
        if let Some(text) = parts.as_string() {
            if sink.json_members(&text).is_ok() {
                return Ok(());
            }
            // Text that does not parse (a lone surrogate, or nesting deeper than serde_json
            // reads): the level is read again, value by value.
            parts = keep(
                &self.exception,
                read_js(&self.nodes, LEVEL, index, u32::MAX),
            )?;
        }
        write(parts, sink, &self.exception);
        Ok(())
    }

    fn read_field(&self, index: u32, key: &str, sink: &mut Sink<'_, 'a>) -> Result<bool, String> {
        let value = keep(&self.exception, read_field_js(&self.nodes, index, key))?;
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
            let tag = value.unchecked_ref::<js_sys::Array>().get(0).as_f64();
            let level = Nested::new(value, &self.exception);
            if tag == Some(6.0) {
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
        let parts = keep(&self.exception, read_js(&self.nodes, ITEM, index, item))?;
        write(parts, sink, &self.exception);
        Ok(true)
    }
}

/// The result of a call to the helper, keeping the exception it throws.
fn keep(exception: &Exception, result: Result<JsValue, JsValue>) -> Result<JsValue, String> {
    result.map_err(|e| {
        let message = match e.dyn_ref::<js_sys::Error>() {
            Some(error) => String::from(error.message()),
            None => e.as_string().unwrap_or_else(|| "an exception".into()),
        };
        exception.borrow_mut().get_or_insert(e);
        message
    })
}

/// Writes the values the helper returned, `parts`, to `sink`.
fn write(parts: JsValue, sink: &mut Sink<'_, '_>, exception: &Exception) {
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
    // The members of the level, made when its first array or object is met.
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
            _ => break,
        };
        if fields {
            sink.key(text.take(nums.next().unwrap_or(0.0) as usize));
        }
        match value {
            Entry::Null => sink.null(),
            Entry::Bool(b) => sink.bool(b),
            Entry::Number(n) => sink.number(n),
            Entry::String(s) => sink.string(s),
            Entry::Array | Entry::Object => {
                let level = nested.get_or_insert_with(|| Nested::new(parts.get(2), exception));
                if let Entry::Array = value {
                    sink.array_of(level, next_nested);
                } else {
                    sink.object_of(level, next_nested);
                }
                next_nested += 1;
            }
        }
    }
}

enum Entry<'t> {
    Null,
    Bool(bool),
    Number(f64),
    String(&'t str),
    Array,
    Object,
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
