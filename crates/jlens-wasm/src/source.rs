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
//! it did before levels were read lazily.

use std::cell::RefCell;
use std::rc::Rc;

use jlens_core::input::{Sink, Source};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
const CIRCULAR = "Converting circular structure to JSON";
const BIGINT = "Do not know how to serialize a BigInt";

// Reads one level: `kind` 0 is the input itself (`node` is the value), 1 an array and 2 an
// object (`node` is {v, up}: the value and its parent's node, to detect cycles). Returns the
// number of values then their tags and numbers, their strings and keys as one text (lengths
// in UTF-16 code units), and the nodes of the nested arrays and objects. In an object,
// each value is followed by the length of its key. An array or object of more than `whole`
// values is returned as its JSON text instead (a cycle back to an ancestor passes through it
// again, so JSON.stringify sees it).
export function jlensLevel(node, kind, whole) {
  const nums = [0];
  let text = "";
  const nested = [];
  const up = kind === 0 ? null : node;
  const put = (v, key) => {
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
        for (let n = up; n !== null; n = n.up) {
          if (n.v === v) throw new TypeError(CIRCULAR);
        }
        nums.push(Array.isArray(v) ? 6 : 7);
        nested.push({ v, up });
        return true;
      default:
        return false;
    }
  };
  if (kind === 0) {
    put(node, "");
  } else if (kind === 1) {
    const items = node.v;
    if (items.length > whole) return JSON.stringify(items);
    nums[0] = items.length;
    for (let i = 0; i < items.length; i++) {
      if (!put(items[i], String(i))) nums.push(1);
    }
  } else {
    const fields = node.v;
    const keys = Object.keys(fields);
    if (keys.length > whole) return JSON.stringify(fields);
    nums[0] = keys.length;
    for (const key of keys) {
      if (put(fields[key], key)) {
        nums.push(key.length);
        text += key;
      }
    }
  }
  return [new Float64Array(nums), text, nested];
}
"#)]
extern "C" {
    #[wasm_bindgen(catch, js_name = jlensLevel)]
    fn level(node: &JsValue, kind: u32, whole: u32) -> Result<JsValue, JsValue>;
}

/// The number of values above which a level is read whole, as JSON text.
const WHOLE: u32 = 16;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Input = 0,
    Array = 1,
    Object = 2,
}

/// A level of a JavaScript value. The first exception a level throws is kept, for `evaluate` to
/// rethrow.
pub struct JsSource {
    node: JsValue,
    kind: Kind,
    exception: Rc<RefCell<Option<JsValue>>>,
}

impl JsSource {
    /// The input of an evaluation.
    pub fn input(value: JsValue) -> JsSource {
        JsSource {
            node: value,
            kind: Kind::Input,
            exception: Rc::default(),
        }
    }

    /// The exception that stopped the evaluation, when `err` is the failure of a level.
    pub fn exception(&self, err: &jlens_core::Error) -> Option<JsValue> {
        (err.code() == "U1004")
            .then(|| self.exception.borrow().clone())
            .flatten()
    }

    /// Calls the helper, keeping the exception it throws.
    fn level(&self, whole: u32) -> Result<JsValue, String> {
        level(&self.node, self.kind as u32, whole).map_err(|e| {
            let message = match e.dyn_ref::<js_sys::Error>() {
                Some(error) => String::from(error.message()),
                None => e.as_string().unwrap_or_else(|| "an exception".into()),
            };
            self.exception.borrow_mut().get_or_insert(e);
            message
        })
    }

    fn nested(&self, node: JsValue, kind: Kind) -> JsSource {
        JsSource {
            node,
            kind,
            exception: self.exception.clone(),
        }
    }
}

impl<'a> Source<'a> for JsSource {
    fn read(&self, sink: &mut Sink<'_, 'a>) -> Result<(), String> {
        let mut parts = self.level(WHOLE)?;
        if let Some(text) = parts.as_string() {
            if sink.json_members(&text).is_ok() {
                return Ok(());
            }
            // Text that does not parse (a lone surrogate, or nesting deeper than serde_json
            // reads): the level is read again, value by value.
            parts = self.level(u32::MAX)?;
        }
        let parts: js_sys::Array = parts.unchecked_into();
        let nums = parts
            .get(0)
            .unchecked_into::<js_sys::Float64Array>()
            .to_vec();
        let text = parts.get(1).as_string().unwrap_or_default();
        let nested: js_sys::Array = parts.get(2).unchecked_into();
        let mut nums = nums.into_iter();
        sink.reserve(nums.next().unwrap_or(0.0) as usize);
        let mut text = Text::new(&text);
        let mut next_nested = 0;
        while let Some(tag) = nums.next() {
            let value = match tag as u8 {
                1 => Entry::Null,
                2 => Entry::Bool(false),
                3 => Entry::Bool(true),
                4 => Entry::Number(nums.next().unwrap_or(f64::NAN)),
                5 => Entry::String(text.take(nums.next().unwrap_or(0.0) as usize)),
                6 | 7 => {
                    let node = nested.get(next_nested);
                    next_nested += 1;
                    Entry::Nested(
                        node,
                        if tag as u8 == 6 {
                            Kind::Array
                        } else {
                            Kind::Object
                        },
                    )
                }
                _ => break,
            };
            if self.kind == Kind::Object {
                sink.key(text.take(nums.next().unwrap_or(0.0) as usize));
            }
            match value {
                Entry::Null => sink.null(),
                Entry::Bool(b) => sink.bool(b),
                Entry::Number(n) => sink.number(n),
                Entry::String(s) => sink.string(s),
                Entry::Nested(node, Kind::Array) => sink.array(self.nested(node, Kind::Array)),
                Entry::Nested(node, _) => sink.object(self.nested(node, Kind::Object)),
            }
        }
        Ok(())
    }
}

enum Entry<'t> {
    Null,
    Bool(bool),
    Number(f64),
    String(&'t str),
    Nested(JsValue, Kind),
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
