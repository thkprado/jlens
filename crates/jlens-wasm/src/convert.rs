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

/// Reads the evaluation options: `timeout` (milliseconds), `maxDepth`, `maxStringLength`,
/// `maxArrayLength`, `maxResultSize`, `now` and `seed`.
pub fn options(options: Option<&Value>) -> Result<Options, JsValue> {
    let mut out = Options::default();
    let Some(options) = options else {
        return Ok(out);
    };
    let Value::Object(map) = options else {
        return Err(type_error("options must be an object"));
    };
    let get = |key: &str| map.get(key).filter(|v| !v.is_null());
    let count = |key: &str| -> Result<Option<usize>, JsValue> {
        get(key)
            .map(|v| {
                v.as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| {
                        type_error(&format!("options.{key} must be a non-negative integer"))
                    })
            })
            .transpose()
    };
    if let Some(v) = get("timeout") {
        let ms = v
            .as_f64()
            .filter(|ms| *ms >= 0.0)
            .ok_or_else(|| type_error("options.timeout must be a number of milliseconds"))?;
        out.timeout = Some(std::time::Duration::from_secs_f64(ms / 1000.0));
    }
    out.max_depth = count("maxDepth")?;
    out.max_string_length = count("maxStringLength")?;
    out.max_array_length = count("maxArrayLength")?;
    out.max_result_size = count("maxResultSize")?;
    if let Some(v) = get("now") {
        out.now = Some(now(v).ok_or_else(|| {
            type_error(
                "options.now must be milliseconds since the epoch, a Date or an ISO 8601 timestamp",
            )
        })?);
    }
    if let Some(v) = get("seed") {
        out.seed = Some(
            v.as_u64()
                .ok_or_else(|| type_error("options.seed must be a non-negative integer"))?,
        );
    }
    Ok(out)
}

/// Reads `now`: an integer number of milliseconds, or a timestamp string (a `Date` arrives as
/// its ISO 8601 string, through `JSON.stringify`).
fn now(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64(),
        Value::String(text) => {
            let expr =
                jlens_core::Expression::compile("$toMillis($t)").expect("a valid expression");
            match expr.evaluate(None, &Bindings::new().var("t", text.as_str())) {
                Ok(Some(Value::Number(n))) => n.as_f64().map(|f| f as i64),
                _ => None,
            }
        }
        _ => None,
    }
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
