//! Conversions at the JavaScript boundary: values, offsets and errors.

use jlens_core::{Bindings, Options, format_json};
use serde::Serialize;
use serde_json::Value;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::host::Scope;

/// Converts between UTF-8 byte offsets (used by the Rust crates) and UTF-16 code unit offsets
/// (used by JavaScript strings).
pub struct Offsets {
    /// For non-ASCII text: the UTF-16 offset of every byte offset (`len + 1` entries). Bytes
    /// inside a character map to the offset of the character.
    table: Option<Vec<u32>>,
    len: usize,
}

impl Offsets {
    pub fn new(text: &str) -> Self {
        let table = (!text.is_ascii()).then(|| {
            let mut table = Vec::with_capacity(text.len() + 1);
            let mut units = 0u32;
            for c in text.chars() {
                table.extend(std::iter::repeat_n(units, c.len_utf8()));
                units += c.len_utf16() as u32;
            }
            table.push(units);
            table
        });
        Offsets {
            table,
            len: text.len(),
        }
    }

    /// The UTF-16 offset of a byte offset.
    pub fn to_utf16(&self, byte: usize) -> usize {
        let byte = byte.min(self.len);
        match &self.table {
            Some(table) => table[byte] as usize,
            None => byte,
        }
    }

    /// The byte offset of a UTF-16 offset. An offset inside a surrogate pair maps to the start
    /// of the character.
    pub fn to_byte(&self, unit: usize) -> usize {
        match &self.table {
            Some(table) => {
                // The first byte whose UTF-16 offset reaches `unit`.
                let mut byte = table
                    .partition_point(|&u| (u as usize) < unit)
                    .min(self.len);
                if table[byte] as usize > unit {
                    // `unit` falls inside a surrogate pair: go back to the character's start.
                    byte -= 1;
                    while byte > 0 && table[byte - 1] == table[byte] {
                        byte -= 1;
                    }
                }
                byte
            }
            None => unit.min(self.len),
        }
    }

    /// Rewrites, recursively, every `span` and `range` object (`{start, end}`) from byte
    /// offsets to UTF-16 offsets.
    pub fn convert_spans(&self, value: &mut Value) {
        if self.table.is_none() {
            return;
        }
        match value {
            Value::Array(items) => items.iter_mut().for_each(|v| self.convert_spans(v)),
            Value::Object(map) => {
                for (key, v) in map.iter_mut() {
                    if (key == "span" || key == "range")
                        && let Value::Object(span) = v
                        && span.len() == 2
                    {
                        for bound in ["start", "end"] {
                            if let Some(n) = span.get(bound).and_then(Value::as_u64) {
                                span.insert(bound.into(), self.to_utf16(n as usize).into());
                            }
                        }
                    } else {
                        self.convert_spans(v);
                    }
                }
            }
            _ => {}
        }
    }

    /// Rewrites the spans of the nodes of `tree`, an audit tree, from byte offsets to UTF-16
    /// offsets. Only the nodes are walked: a captured value may have a field named `span` too.
    pub fn convert_audit_spans(&self, tree: &mut Value) {
        fn walk(offsets: &Offsets, node: &mut Value) {
            jlens_core::stack::grow(|| {
                if let Some(Value::Object(span)) = node.get_mut("span") {
                    for bound in ["start", "end"] {
                        if let Some(n) = span.get(bound).and_then(Value::as_u64) {
                            span.insert(bound.into(), offsets.to_utf16(n as usize).into());
                        }
                    }
                }
                if let Some(Value::Array(children)) = node.get_mut("children") {
                    children.iter_mut().for_each(|child| walk(offsets, child));
                }
            })
        }
        if self.table.is_some()
            && let Some(root) = tree.get_mut("root")
        {
            walk(self, root);
        }
    }
}

/// Converts a JS value to JSON, through `JSON.stringify`. `undefined` (and values that
/// stringify to nothing, such as functions) become `None`.
pub fn from_js(value: &JsValue, what: &str) -> Result<Option<Value>, JsValue> {
    if value.is_undefined() {
        return Ok(None);
    }
    let text = js_sys::JSON::stringify(value)
        .map_err(|e| type_error(&format!("{what} cannot be converted to JSON: {e:?}")))?;
    match text.as_string() {
        Some(text) => parse_json(&text, what).map(Some),
        None => Ok(None),
    }
}

/// Converts a JS value to a jlens document, through `JSON.stringify` and a parse straight into
/// the internal representation. `undefined` (and values that stringify to nothing) become `None`.
pub fn document(value: &JsValue, what: &str) -> Result<Option<jlens_core::Document>, JsValue> {
    if value.is_undefined() {
        return Ok(None);
    }
    let text = js_sys::JSON::stringify(value)
        .map_err(|e| type_error(&format!("{what} cannot be converted to JSON: {e:?}")))?;
    match text.as_string() {
        Some(text) => jlens_core::Document::parse(&text)
            .map(Some)
            .map_err(|e| type_error(&format!("{what} is not valid JSON: {e}"))),
        None => Ok(None),
    }
}

pub fn parse_json(text: &str, what: &str) -> Result<Value, JsValue> {
    serde_json::from_str(text).map_err(|e| type_error(&format!("{what} is not valid JSON: {e}")))
}

/// Converts JSON to a JS value, through `JSON.parse`. Numbers are formatted the way
/// JavaScript formats them, so they round-trip exactly.
pub fn to_js(value: &Value) -> JsValue {
    js_sys::JSON::parse(&format_json(value, false)).expect("format_json produces valid JSON")
}

/// Serializes a Rust value to a JS value through JSON.
pub fn ser_to_js<T: Serialize>(value: &T) -> JsValue {
    to_js(&serde_json::to_value(value).expect("serializable"))
}

/// A function registered on an expression with `registerFunction`.
pub struct Registered {
    pub name: String,
    pub function: js_sys::Function,
    pub signature: Option<String>,
}

/// Reads the `bindings` argument (`undefined`, `null` or an object), after the functions
/// registered on the expression. A function in the bindings becomes a host function of `scope`,
/// and replaces a registered function of the same name.
pub fn bindings(
    value: &JsValue,
    scope: &Scope,
    registered: &[Registered],
) -> Result<Bindings, JsValue> {
    let mut out = Bindings::new();
    for r in registered {
        let function = scope
            .function(&r.name, &r.function, r.signature.as_deref())
            .map_err(|e| type_error(&e.to_string()))?;
        out = out.function(r.name.clone(), function);
    }
    if value.is_undefined() || value.is_null() {
        return Ok(out);
    }
    if !value.is_object() || js_sys::Array::is_array(value) || value.is_function() {
        return Err(type_error("bindings must be an object"));
    }
    for entry in js_sys::Object::entries(value.unchecked_ref()).iter() {
        let entry: js_sys::Array = entry.unchecked_into();
        let Some(name) = entry.get(0).as_string() else {
            continue;
        };
        let value = entry.get(1);
        if let Some(function) = value.dyn_ref::<js_sys::Function>() {
            let function = scope
                .function(&name, function, None)
                .map_err(|e| type_error(&e.to_string()))?;
            out = out.function(name, function);
        } else if let Some(json) = from_js(&value, &format!("binding {name}"))? {
            out = out.var(name, json);
        }
    }
    Ok(out)
}

#[wasm_bindgen(inline_js = r#"
const read = new Float64Array(16);
let now = "";
let set = 0;

// Writes the tag and number of option `i` into `read`. The tag is 0 for undefined or null, 1 for
// a number, 2 for a Date (its time, NaN when invalid), 3 for a string (`now` only, kept for
// jlensNow) and 4 for anything else.
function put(i, v) {
  let tag = 4;
  let n = 0;
  if (v === undefined || v === null) tag = 0;
  else if (typeof v === "number") (tag = 1), (n = v);
  else if (typeof v === "string" && i === 6) (tag = 3), (now = v);
  else if (Object.prototype.toString.call(v) === "[object Date]") (tag = 2), (n = Date.prototype.getTime.call(v));
  read[2 * i] = tag;
  read[2 * i + 1] = n;
  if (tag !== 0) set = 1;
}

// Reads the evaluation options into `read`, in the order of `convert::options`. Each option is
// read once, as a property (getters and inherited properties included, `toJSON` not called),
// each at its own site, which V8 caches for the shape of the options. Returns -1 when `options`
// is not a plain object, 0 when no option is set, and 1 otherwise.
export function jlensOptions(options) {
  if (typeof options !== "object" || Array.isArray(options)) return -1;
  set = 0;
  put(0, options.timeout);
  put(1, options.maxDepth);
  put(2, options.maxSteps);
  put(3, options.maxStringLength);
  put(4, options.maxArrayLength);
  put(5, options.maxResultSize);
  put(6, options.now);
  put(7, options.seed);
  return set;
}

export function jlensOptionsRead() {
  return read;
}

export function jlensNow() {
  return now;
}
"#)]
extern "C" {
    #[wasm_bindgen(catch, js_name = jlensOptions)]
    fn read_options(options: &JsValue) -> Result<i32, JsValue>;
    #[wasm_bindgen(js_name = jlensOptionsRead)]
    fn options_read() -> js_sys::Float64Array;
    #[wasm_bindgen(js_name = jlensNow)]
    fn options_now() -> String;
}

thread_local! {
    /// The array `jlensOptions` writes into.
    static OPTIONS_READ: js_sys::Float64Array = options_read();
    /// `$toMillis`, for a timestamp given as a string.
    static TO_MILLIS: jlens_core::Expression =
        jlens_core::Expression::compile("$toMillis($t)").expect("a valid expression");
}

/// An option, as `jlensOptions` reads it.
enum Opt {
    Unset,
    Number(f64),
    Date(f64),
    Text,
    Other,
}

/// Reads the evaluation options: `timeout` (milliseconds), `maxDepth`, `maxSteps`,
/// `maxStringLength`, `maxArrayLength`, `maxResultSize`, `now` and `seed`. They are read from
/// the object itself, not through JSON: `undefined` and `null` mean the default, a number must
/// be finite, and `now` may be a `Date`.
pub fn options(options: Option<&JsValue>) -> Result<Options, JsValue> {
    let mut out = Options::default();
    let Some(options) = options else {
        return Ok(out);
    };
    match read_options(options)? {
        -1 => return Err(type_error("options must be an object")),
        0 => return Ok(out),
        _ => {}
    }
    let mut read = [0.0; 16];
    OPTIONS_READ.with(|array| array.copy_to(&mut read));
    let get = |i: usize| match read[2 * i] as u8 {
        0 => Opt::Unset,
        1 => Opt::Number(read[2 * i + 1]),
        2 => Opt::Date(read[2 * i + 1]),
        3 => Opt::Text,
        _ => Opt::Other,
    };
    // A non-negative integer below 2^64.
    let integer = |i: usize, key: &str| -> Result<Option<u64>, JsValue> {
        match get(i) {
            Opt::Unset => Ok(None),
            Opt::Number(n) if n >= 0.0 && n.fract() == 0.0 && n < u64::MAX as f64 => {
                Ok(Some(n as u64))
            }
            _ => Err(type_error(&format!(
                "options.{key} must be a non-negative integer"
            ))),
        }
    };
    // A count larger than the address space limits nothing more than the largest one.
    let count = |i: usize, key: &str| -> Result<Option<usize>, JsValue> {
        Ok(integer(i, key)?.map(|n| usize::try_from(n).unwrap_or(usize::MAX)))
    };
    match get(0) {
        Opt::Unset => {}
        Opt::Number(ms) if ms >= 0.0 && ms.is_finite() => {
            // A timeout too long for a `Duration` never expires.
            out.timeout = std::time::Duration::try_from_secs_f64(ms / 1000.0).ok();
        }
        _ => {
            return Err(type_error(
                "options.timeout must be a number of milliseconds",
            ));
        }
    }
    out.max_depth = count(1, "maxDepth")?;
    out.max_steps = integer(2, "maxSteps")?;
    out.max_string_length = count(3, "maxStringLength")?;
    out.max_array_length = count(4, "maxArrayLength")?;
    out.max_result_size = count(5, "maxResultSize")?;
    let now = match get(6) {
        Opt::Unset => None,
        Opt::Number(ms) | Opt::Date(ms) => Some(millis(ms)),
        Opt::Text => Some(timestamp(&options_now())),
        Opt::Other => Some(None),
    };
    if let Some(now) = now {
        out.now = Some(now.ok_or_else(|| {
            type_error(
                "options.now must be milliseconds since the epoch, a Date or an ISO 8601 timestamp",
            )
        })?);
    }
    out.seed = integer(7, "seed")?;
    Ok(out)
}

/// An integer number of milliseconds since the epoch, or `None` (an invalid `Date` is `NaN`).
fn millis(ms: f64) -> Option<i64> {
    (ms.fract() == 0.0 && ms.abs() < i64::MAX as f64).then_some(ms as i64)
}

/// The milliseconds of an ISO 8601 timestamp.
fn timestamp(text: &str) -> Option<i64> {
    TO_MILLIS.with(
        |expr| match expr.evaluate(None, &Bindings::new().var("t", text)) {
            Ok(Some(Value::Number(n))) => n.as_f64().map(|f| f as i64),
            _ => None,
        },
    )
}

/// The fields of a jlens error, as JSON, with offsets in UTF-16 code units.
pub fn error_json(err: &jlens_core::Error, offsets: &Offsets) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("code".into(), err.code().into());
    map.insert("message".into(), err.message().into());
    if let Some(position) = err.position() {
        map.insert("position".into(), position.into());
    }
    if let Some(span) = err.span() {
        map.insert("start".into(), offsets.to_utf16(span.start).into());
        map.insert("end".into(), offsets.to_utf16(span.end).into());
    }
    if let Some(token) = err.token() {
        map.insert("token".into(), token.into());
    }
    if let Some(value) = err.value() {
        map.insert("value".into(), value.clone());
    }
    Value::Object(map)
}

/// A JS `Error` named `JlensError`, carrying the fields of [`error_json`].
pub fn js_error(err: &jlens_core::Error, offsets: &Offsets) -> JsValue {
    let fields = error_json(err, offsets);
    let error = js_sys::Error::new(&err.message());
    error.set_name("JlensError");
    if let Value::Object(map) = &fields {
        for (key, value) in map {
            if key != "message" {
                let _ = js_sys::Reflect::set(&error, &JsValue::from_str(key), &to_js(value));
            }
        }
    }
    error.into()
}

/// Reports Rust panics on the browser console (debug builds only).
pub fn install_hooks() {
    #[cfg(debug_assertions)]
    console_error_panic_hook::set_once();
}

/// A JS `TypeError` for invalid arguments.
pub fn type_error(message: &str) -> JsValue {
    js_sys::TypeError::new(message).into()
}

#[cfg(test)]
mod tests {
    use super::Offsets;

    #[test]
    fn offsets_round_trip() {
        let text = "aé𝄞b";
        let o = Offsets::new(text);
        // bytes: a=0, é=1..3, 𝄞=3..7, b=7, end=8; UTF-16: a=0, é=1, 𝄞=2..4, b=4, end=5.
        assert_eq!([0, 1, 3, 7, 8].map(|b| o.to_utf16(b)), [0, 1, 2, 4, 5]);
        assert_eq!([0, 1, 2, 4, 5].map(|u| o.to_byte(u)), [0, 1, 3, 7, 8]);
        // Inside the surrogate pair: the start of the character.
        assert_eq!(o.to_byte(3), 3);
        assert_eq!(o.to_byte(99), 8);
        let ascii = Offsets::new("abc");
        assert_eq!((ascii.to_utf16(2), ascii.to_byte(9)), (2, 3));
    }
}
