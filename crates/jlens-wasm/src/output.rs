//! The result of `evaluate`, built as JavaScript values (ADR 0017).
//!
//! Crossing into JavaScript once per value costs more than the value itself, so the builder
//! writes the result as a list of operations, its numbers and its strings, and one call to a
//! JavaScript helper builds the values from them. Each key is decoded once per result: an
//! object's key is a new key's text, or the index of a key seen before.

use std::collections::HashMap;

use foldhash::fast::RandomState;
use jlens_core::input::{Sink, Source};
use jlens_core::output::Builder;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
// Builds the values that `ops` describes, in order: a stack holds the values built, and the
// arrays and objects being filled. `text` holds the strings and the new keys, one after the
// other (lengths in UTF-16 code units). Returns the last value built: the result.
export function jlensBuild(ops, nums, text) {
  const stack = [];
  const keys = [];
  // The index of the key `__proto__`, which an assignment would take for the prototype.
  let proto = -1;
  let num = 0, pos = 0;
  for (let i = 0; i < ops.length; i++) {
    switch (ops[i]) {
      case 0: stack.push(null); break;
      case 1: stack.push(false); break;
      case 2: stack.push(true); break;
      case 3: stack.push(nums[num++]); break;
      case 4: { const end = pos + ops[++i]; stack.push(text.slice(pos, end)); pos = end; break; }
      case 5: stack.push([]); break;
      case 6: { const item = stack.pop(); stack[stack.length - 1].push(item); break; }
      case 7: stack.push({}); break;
      case 8: {
        const key = ops[++i];
        const value = stack.pop();
        if (key === proto) own(stack[stack.length - 1], keys[key], value);
        else stack[stack.length - 1][keys[key]] = value;
        break;
      }
      case 9: {
        const end = pos + ops[++i];
        const key = text.slice(pos, end);
        pos = end;
        const value = stack.pop();
        if (key === "__proto__") {
          proto = keys.length;
          own(stack[stack.length - 1], key, value);
        } else stack[stack.length - 1][key] = value;
        keys.push(key);
        break;
      }
    }
  }
  return stack.pop();
}

// Defines an own property, as JSON.parse does.
function own(object, key, value) {
  Object.defineProperty(object, key, { value, writable: true, enumerable: true, configurable: true });
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = jlensBuild)]
    fn build(ops: &[u32], nums: &[f64], text: &str) -> JsValue;
}

const NULL: u32 = 0;
const FALSE: u32 = 1;
const TRUE: u32 = 2;
const NUMBER: u32 = 3;
/// Followed by the length of the string.
const STRING: u32 = 4;
const ARRAY: u32 = 5;
const PUSH: u32 = 6;
const OBJECT: u32 = 7;
/// Followed by the index of a key seen before.
const KEY: u32 = 8;
/// Followed by the length of a new key.
const NEW_KEY: u32 = 9;

/// Writes a result as operations for `jlensBuild`.
#[derive(Default)]
pub struct JsBuilder {
    ops: Vec<u32>,
    nums: Vec<f64>,
    text: String,
    keys: HashMap<Box<str>, u32, RandomState>,
}

impl JsBuilder {
    /// Builds the JavaScript value of what was written: `undefined` for an undefined result.
    pub fn finish(self, written: Option<()>) -> JsValue {
        match written {
            Some(()) => build(&self.ops, &self.nums, &self.text),
            None => JsValue::UNDEFINED,
        }
    }

    /// Adds `text`, followed by its length in UTF-16 code units.
    fn text(&mut self, text: &str) {
        let len = if text.is_ascii() {
            text.len()
        } else {
            text.encode_utf16().count()
        };
        // A result is shorter than `u32::MAX` bytes: `Options::max_result_size` bounds it, and
        // a Wasm memory holds at most 4 GiB.
        self.ops.push(len as u32);
        self.text.push_str(text);
    }
}

impl Builder for JsBuilder {
    type Value = ();
    type Array = ();
    type Object = ();

    fn null(&mut self) {
        self.ops.push(NULL);
    }

    fn bool(&mut self, value: bool) {
        self.ops.push(if value { TRUE } else { FALSE });
    }

    fn number(&mut self, value: f64) {
        self.ops.push(NUMBER);
        // As through JSON: `-0` is written `0`.
        self.nums.push(value + 0.0);
    }

    fn string(&mut self, value: &str) {
        self.ops.push(STRING);
        self.text(value);
    }

    fn array(&mut self, _len: usize) {
        self.ops.push(ARRAY);
    }

    fn push(&mut self, _array: &mut (), _item: ()) {
        self.ops.push(PUSH);
    }

    fn end_array(&mut self, _array: ()) {}

    fn object(&mut self, _len: usize) {
        self.ops.push(OBJECT);
    }

    fn insert(&mut self, _object: &mut (), key: &str, _value: ()) {
        match self.keys.get(key) {
            Some(&index) => self.ops.extend([KEY, index]),
            None => {
                self.keys.insert(key.into(), self.keys.len() as u32);
                self.ops.push(NEW_KEY);
                self.text(key);
            }
        }
    }

    fn end_object(&mut self, _object: ()) {}
}

/// No input document: an evaluation of `evaluate` without one.
pub struct NoInput;

impl Source<'_> for NoInput {
    fn read(&self, _sink: &mut Sink<'_, '_>) -> Result<(), String> {
        Ok(())
    }
}
