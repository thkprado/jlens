//! JSON text with JavaScript number formatting.

use serde_json::Value as JsonValue;

use crate::value::{js_number, write_json_string};

/// Serializes a result as JSON text the way JavaScript's `JSON.stringify` would. Numbers use
/// JavaScript's formatting, so `2432902008176640000` stays an integer and `1e21` prints as
/// `1e+21`. With `pretty`, nested values are indented by two spaces.
///
/// ```
/// use serde_json::json;
///
/// let v = json!({"n": 2.43290200817664e18, "x": [0.1, 1e21]});
/// assert_eq!(jlens_core::format_json(&v, false), r#"{"n":2432902008176640000,"x":[0.1,1e+21]}"#);
/// ```
pub fn format_json(value: &JsonValue, pretty: bool) -> String {
    let mut out = String::new();
    write(&mut out, value, pretty, 0);
    out
}

fn write(out: &mut String, value: &JsonValue, pretty: bool, depth: usize) {
    let newline = |out: &mut String, depth: usize| {
        if pretty {
            out.push('\n');
            out.push_str(&"  ".repeat(depth));
        }
    };
    match value {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        JsonValue::Number(n) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => out.push_str(&i.to_string()),
            (_, Some(u)) => out.push_str(&u.to_string()),
            _ => out.push_str(&js_number(n.as_f64().unwrap_or(f64::NAN))),
        },
        JsonValue::String(s) => write_json_string(out, s),
        JsonValue::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                write(out, item, pretty, depth + 1);
            }
            newline(out, depth);
            out.push(']');
        }
        JsonValue::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, v)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                write_json_string(out, k);
                out.push(':');
                if pretty {
                    out.push(' ');
                }
                write(out, v, pretty, depth + 1);
            }
            newline(out, depth);
            out.push('}');
        }
    }
}
