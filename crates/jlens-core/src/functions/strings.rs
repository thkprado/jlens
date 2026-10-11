//! String functions, matchers, Base64 and URL encoding.
//!
//! JavaScript strings are sequences of UTF-16 code units. Where jsonata-js counts in code
//! points (`$length`, `$substring`, `$pad`) jlens counts `char`s; positions reported by
//! matchers (`$match`, regex literals) are UTF-16 offsets, as in jsonata-js.

use std::rc::Rc;

use super::{Call, arg, num, text};
use crate::error::{Error, Result};
use crate::eval::Evaluator;
use crate::value::{Map, Value};

type R = Result<Value>;

/// `String.prototype.slice` over characters, with JavaScript's handling of negative and
/// fractional positions.
fn js_slice(chars: &[char], start: f64, end: Option<f64>) -> String {
    let len = chars.len() as f64;
    let relative = |v: f64| -> f64 {
        let v = if v.is_nan() { 0.0 } else { v.trunc() };
        if v < 0.0 {
            (len + v).max(0.0)
        } else {
            v.min(len)
        }
    };
    let from = relative(start);
    let to = end.map_or(len, relative);
    if from >= to {
        return String::new();
    }
    chars[from as usize..to as usize].iter().collect()
}

/// Slices a string by UTF-16 offsets, as `String.prototype.substring` does.
pub(crate) fn utf16_substring(s: &str, from: usize, to: Option<usize>) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    let len = units.len();
    let (mut a, mut b) = (from.min(len), to.unwrap_or(len).min(len));
    if a > b {
        std::mem::swap(&mut a, &mut b);
    }
    String::from_utf16_lossy(&units[a..b])
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

pub(super) fn substring(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let Some(s) = text(&a, 0) else {
        return Ok(Value::Undefined);
    };
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len() as f64;
    let mut start = num(&a, 1).unwrap_or(f64::NAN);
    if len + start < 0.0 {
        start = 0.0;
    }
    Ok(Value::string(match num(&a, 2) {
        Some(length) => {
            if length <= 0.0 {
                return Ok(Value::string(""));
            }
            let end = if start >= 0.0 {
                start + length
            } else {
                len + start + length
            };
            js_slice(&chars, start, Some(end))
        }
        None => js_slice(&chars, start, None),
    }))
}

pub(super) fn substring_before(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let Some(s) = text(&a, 0) else {
        return Ok(Value::Undefined);
    };
    // jsonata-js searches for the text "undefined" when `chars` is missing (compatibility.md).
    let Some(chars) = text(&a, 1) else {
        return Ok(Value::String(s));
    };
    Ok(match s.find(chars.as_ref()) {
        Some(pos) => Value::string(&s[..pos]),
        None => Value::String(s),
    })
}

pub(super) fn substring_after(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let Some(s) = text(&a, 0) else {
        return Ok(Value::Undefined);
    };
    let Some(chars) = text(&a, 1) else {
        return Ok(Value::String(s));
    };
    Ok(match s.find(chars.as_ref()) {
        Some(pos) => Value::string(&s[pos + chars.len()..]),
        None => Value::String(s),
    })
}

pub(super) fn lowercase(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    Ok(text(&a, 0).map_or(Value::Undefined, |s| Value::string(s.to_lowercase())))
}

pub(super) fn uppercase(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    Ok(text(&a, 0).map_or(Value::Undefined, |s| Value::string(s.to_uppercase())))
}

pub(super) fn length(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    Ok(text(&a, 0).map_or(
        Value::Undefined,
        |s| Value::Number(s.chars().count() as f64),
    ))
}

pub(super) fn trim(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let Some(s) = text(&a, 0) else {
        return Ok(Value::Undefined);
    };
    let mut out = String::with_capacity(s.len());
    let mut in_space = false;
    for c in s.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\r') {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    let out = out.strip_prefix(' ').unwrap_or(&out);
    let out = out.strip_suffix(' ').unwrap_or(out);
    Ok(Value::string(out))
}

pub(super) fn pad(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let Some(s) = text(&a, 0) else {
        return Ok(Value::Undefined);
    };
    let fill = text(&a, 2)
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| Rc::from(" "));
    let width = num(&a, 1).unwrap_or(0.0).trunc();
    let pad_length = width.abs() - s.chars().count() as f64;
    if pad_length <= 0.0 {
        return Ok(Value::String(s));
    }
    if pad_length > 1e7 {
        return Err(Error::new("D2016").with("value", pad_length));
    }
    let padding: String = fill.chars().cycle().take(pad_length as usize).collect();
    Ok(Value::string(if width > 0.0 {
        format!("{s}{padding}")
    } else {
        format!("{padding}{s}")
    }))
}

/// Applies a matcher (a regex or a user function) to a string, checking the structure it
/// returns (jsonata-js `evaluateMatcher`).
fn evaluate_matcher(ev: &mut Evaluator, matcher: &Value, s: Value, call: &Call) -> R {
    let result = ev.apply(matcher, vec![s], &Value::Null, call.env)?;
    if let Value::Undefined = result {
        return Ok(result);
    }
    let valid = match &result {
        Value::Object(o) => {
            let o = o.borrow();
            matches!(o.get("start"), Some(Value::Number(_)))
                || matches!(o.get("groups"), Some(Value::Array(_)))
                || matches!(o.get("next"), Some(Value::Function(_)))
        }
        _ => false,
    };
    if !valid {
        return Err(Error::new("T1010"));
    }
    Ok(result)
}

fn field(v: &Value, key: &str) -> Value {
    match v {
        Value::Object(o) => o.borrow().get(key).cloned().unwrap_or_default(),
        _ => Value::Undefined,
    }
}

fn field_usize(v: &Value, key: &str) -> usize {
    field(v, key).as_number().unwrap_or(0.0).max(0.0) as usize
}

pub(super) fn contains(ev: &mut Evaluator, a: Vec<Value>, call: &Call) -> R {
    let (Some(s), token) = (text(&a, 0), arg(&a, 1)) else {
        return Ok(Value::Undefined);
    };
    Ok(match token {
        Value::Undefined => Value::Undefined,
        Value::String(t) => Value::Bool(s.contains(t.as_ref())),
        matcher => {
            Value::Bool(!evaluate_matcher(ev, &matcher, Value::String(s), call)?.is_undefined())
        }
    })
}

fn check_limit(a: &[Value], i: usize, code: &str, index: usize) -> Result<Option<f64>> {
    match num(a, i) {
        Some(l) if l < 0.0 => Err(Error::new(code).with("value", l).with("index", index)),
        other => Ok(other),
    }
}

pub(super) fn match_(ev: &mut Evaluator, a: Vec<Value>, call: &Call) -> R {
    let Some(s) = text(&a, 0) else {
        return Ok(Value::Undefined);
    };
    let limit = check_limit(&a, 2, "D3040", 3)?;
    let mut out = Vec::new();
    if limit.is_none_or(|l| l > 0.0) {
        let mut matches = evaluate_matcher(ev, &arg(&a, 1), Value::String(s), call)?;
        let mut count = 0.0;
        while !matches.is_undefined() && limit.is_none_or(|l| count < l) {
            let mut m = Map::default();
            m.insert("match".into(), field(&matches, "match"));
            m.insert("index".into(), field(&matches, "start"));
            m.insert("groups".into(), field(&matches, "groups"));
            out.push(Value::object(m));
            matches = evaluate_matcher(ev, &field(&matches, "next"), Value::Undefined, call)?;
            count += 1.0;
        }
    }
    Ok(Value::sequence(out))
}

/// Expands `$0`, `$1`, ... and `$$` in a replacement string (jsonata-js `replace`).
fn substitute(replacement: &str, matched: &Value) -> String {
    let group_count = match field(matched, "groups") {
        Value::Array(g) => g.items.len(),
        _ => 0,
    };
    let whole = field(matched, "match");
    let chars: Vec<char> = replacement.chars().collect();
    let mut out = String::new();
    let mut position = 0;
    while position < chars.len() {
        let Some(offset) = chars[position..].iter().position(|&c| c == '$') else {
            break;
        };
        let index = position + offset;
        out.extend(&chars[position..index]);
        position = index + 1;
        match chars.get(position) {
            Some('$') => {
                out.push('$');
                position += 1;
            }
            Some('0') => {
                if let Value::String(m) = &whole {
                    out.push_str(m);
                }
                position += 1;
            }
            _ => {
                let max_digits = if group_count == 0 {
                    1
                } else {
                    (group_count as f64).log10().floor() as usize + 1
                };
                let parse = |n: usize| -> Option<usize> {
                    let digits: String = chars[position..]
                        .iter()
                        .take(n)
                        .take_while(|c| c.is_ascii_digit())
                        .collect();
                    digits.parse().ok()
                };
                let mut idx = parse(max_digits);
                if max_digits > 1 && idx.is_some_and(|i| i > group_count) {
                    idx = parse(max_digits - 1);
                }
                match idx {
                    Some(i) => {
                        if group_count > 0
                            && i >= 1
                            && let Value::String(sub) = field(matched, "groups")
                                .as_array()
                                .and_then(|g| g.items.get(i - 1).cloned())
                                .unwrap_or_default()
                        {
                            out.push_str(&sub);
                        }
                        position += i.to_string().len();
                    }
                    None => out.push('$'),
                }
            }
        }
    }
    if position < chars.len() {
        out.extend(&chars[position..]);
    }
    out
}

pub(super) fn replace(ev: &mut Evaluator, a: Vec<Value>, call: &Call) -> R {
    let (Some(s), pattern) = (text(&a, 0), arg(&a, 1)) else {
        return Ok(Value::Undefined);
    };
    if pattern.is_undefined() {
        return Ok(Value::Undefined);
    }
    if pattern.as_str() == Some("") {
        return Err(Error::new("D3010").with("value", "").with("index", 2));
    }
    let limit = check_limit(&a, 3, "D3011", 4)?;
    let replacement = arg(&a, 2);
    if limit.is_some_and(|l| l <= 0.0) {
        return Ok(Value::String(s));
    }
    let mut result = String::new();
    let mut count = 0.0;

    if let Value::String(p) = &pattern {
        let replacement = replacement.as_str().unwrap_or_default().to_owned();
        let mut position = 0;
        while let Some(found) = s[position..].find(p.as_ref()) {
            if limit.is_some_and(|l| count >= l) {
                break;
            }
            let index = position + found;
            result.push_str(&s[position..index]);
            result.push_str(&replacement);
            position = index + p.len();
            count += 1.0;
        }
        result.push_str(&s[position..]);
        return Ok(Value::string(result));
    }

    let mut matches = evaluate_matcher(ev, &pattern, Value::String(s.clone()), call)?;
    if matches.is_undefined() {
        return Ok(Value::String(s));
    }
    let mut position = 0; // UTF-16 offset
    while !matches.is_undefined() && limit.is_none_or(|l| count < l) {
        let start = field_usize(&matches, "start");
        result.push_str(&utf16_substring(&s, position, Some(start)));
        let replaced = match &replacement {
            Value::String(r) => Value::string(substitute(r, &matches)),
            func => ev.apply(func, vec![matches.clone()], &Value::Null, call.env)?,
        };
        match replaced {
            Value::String(r) => result.push_str(&r),
            other => {
                let mut err = Error::new("D3012");
                if let Some(v) = other.to_json_detail() {
                    err = err.with("value", v);
                }
                return Err(err);
            }
        }
        let matched_len = field(&matches, "match").as_str().map_or(0, utf16_len);
        position = start + matched_len;
        count += 1.0;
        matches = evaluate_matcher(ev, &field(&matches, "next"), Value::Undefined, call)?;
    }
    result.push_str(&utf16_substring(&s, position, None));
    Ok(Value::string(result))
}

pub(super) fn split(ev: &mut Evaluator, a: Vec<Value>, call: &Call) -> R {
    let (Some(s), separator) = (text(&a, 0), arg(&a, 1)) else {
        return Ok(Value::Undefined);
    };
    if separator.is_undefined() {
        return Ok(Value::Undefined);
    }
    let limit = check_limit(&a, 2, "D3020", 3)?;
    let mut out: Vec<Value> = Vec::new();
    if limit.is_none_or(|l| l > 0.0) {
        if let Value::String(sep) = &separator {
            let parts: Vec<Value> = if sep.is_empty() {
                s.chars().map(|c| Value::string(c.to_string())).collect()
            } else {
                s.split(sep.as_ref()).map(Value::string).collect()
            };
            let max = limit.map_or(usize::MAX, |l| l as usize);
            out = parts.into_iter().take(max).collect();
        } else {
            let mut count = 0.0;
            let mut matches = evaluate_matcher(ev, &separator, Value::String(s.clone()), call)?;
            if matches.is_undefined() {
                out.push(Value::String(s));
            } else {
                let mut start = 0;
                while !matches.is_undefined() && limit.is_none_or(|l| count < l) {
                    let m_start = field_usize(&matches, "start");
                    out.push(Value::string(utf16_substring(&s, start, Some(m_start))));
                    start = field_usize(&matches, "end");
                    matches =
                        evaluate_matcher(ev, &field(&matches, "next"), Value::Undefined, call)?;
                    count += 1.0;
                }
                if limit.is_none_or(|l| count < l) {
                    out.push(Value::string(utf16_substring(&s, start, None)));
                }
            }
        }
    }
    Ok(Value::array(out, 0))
}

pub(super) fn join(ev: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let strs = arg(&a, 0);
    if strs.is_undefined() {
        return Ok(Value::Undefined);
    }
    let separator = text(&a, 1).unwrap_or_default();
    let items = strs.items_or_self();
    let parts: Vec<&str> = items
        .iter()
        .map(|v| v.as_str().unwrap_or_default())
        .collect();
    // Checked before joining: the parts may share one long string many times.
    let len = parts.iter().map(|p| p.len()).sum::<usize>()
        + separator.len() * parts.len().saturating_sub(1);
    ev.check_string(len)?;
    Ok(Value::string(parts.join(&separator)))
}

// ---------------------------------------------------------------------------------------------
// Base64 (characters are treated as bytes, like Node's `Buffer` with the `binary` encoding)

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(super) fn base64encode(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let Some(s) = text(&a, 0) else {
        return Ok(Value::Undefined);
    };
    let bytes: Vec<u8> = s.encode_utf16().map(|u| u as u8).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    Ok(Value::string(out))
}

pub(super) fn base64decode(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let Some(s) = text(&a, 0) else {
        return Ok(Value::Undefined);
    };
    // Node reads each UTF-16 code unit as its low byte: `Ĵ` (U+0134) is the digit `4`.
    let mut sextets = Vec::with_capacity(s.len());
    for c in s.encode_utf16().map(|u| u as u8 as char) {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' | '-' => 62,
            '/' | '_' => 63,
            '=' => break,
            _ => continue,
        };
        sextets.push(v);
    }
    let mut out = String::new();
    for chunk in sextets.chunks(4) {
        let mut n = 0u32;
        for (i, v) in chunk.iter().enumerate() {
            n |= v << (18 - 6 * i);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        let count = match chunk.len() {
            4 => 3,
            3 => 2,
            2 => 1,
            _ => 0,
        };
        out.extend(bytes[..count].iter().map(|&b| b as char));
    }
    Ok(Value::string(out))
}

// ---------------------------------------------------------------------------------------------
// URL encoding (ECMAScript encodeURI / encodeURIComponent / decodeURI / decodeURIComponent)

const URI_UNRESERVED: &str = "-_.!~*'()";
const URI_RESERVED: &str = ";/?:@&=+$,#";

fn encode(s: &str, keep: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || URI_UNRESERVED.contains(c) || keep.contains(c) {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

fn decode(s: &str, keep_reserved: bool) -> Option<String> {
    let bytes = s.as_bytes();
    let hex = |i: usize| -> Option<u8> {
        let h = std::str::from_utf8(bytes.get(i..i + 2)?).ok()?;
        u8::from_str_radix(h, 16).ok()
    };
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            let c = s[i..].chars().next()?;
            out.push(c);
            i += c.len_utf8();
            continue;
        }
        let first = hex(i + 1)?;
        if first < 0x80 {
            let c = first as char;
            if keep_reserved && URI_RESERVED.contains(c) {
                out.push_str(&s[i..i + 3]);
            } else {
                out.push(c);
            }
            i += 3;
            continue;
        }
        let len = match first {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => return None,
        };
        let mut buf = vec![first];
        for k in 1..len {
            let at = i + 3 * k;
            if bytes.get(at) != Some(&b'%') {
                return None;
            }
            let b = hex(at + 1)?;
            if b & 0xC0 != 0x80 {
                return None;
            }
            buf.push(b);
        }
        out.push_str(std::str::from_utf8(&buf).ok()?);
        i += 3 * len;
    }
    Some(out)
}

fn url_error(name: &str, s: &str) -> Error {
    Error::new("D3140")
        .with("value", s)
        .with("functionName", name)
}

pub(super) fn encode_url_component(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    Ok(text(&a, 0).map_or(Value::Undefined, |s| Value::string(encode(&s, ""))))
}

pub(super) fn encode_url(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    Ok(text(&a, 0).map_or(Value::Undefined, |s| {
        Value::string(encode(&s, URI_RESERVED))
    }))
}

pub(super) fn decode_url_component(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let Some(s) = text(&a, 0) else {
        return Ok(Value::Undefined);
    };
    decode(&s, false)
        .map(Value::string)
        .ok_or_else(|| url_error("decodeUrlComponent", &s))
}

pub(super) fn decode_url(_: &mut Evaluator, a: Vec<Value>, _: &Call) -> R {
    let Some(s) = text(&a, 0) else {
        return Ok(Value::Undefined);
    };
    decode(&s, true)
        .map(Value::string)
        .ok_or_else(|| url_error("decodeUrl", &s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_round_trips() {
        assert_eq!(encode("a b/ç?x=1", ""), "a%20b%2F%C3%A7%3Fx%3D1");
        assert_eq!(encode("a b/ç?x=1", URI_RESERVED), "a%20b/%C3%A7?x=1");
        assert_eq!(decode("a%20b%2F%C3%A7", false).as_deref(), Some("a b/ç"));
        assert_eq!(decode("a%2Fb", true).as_deref(), Some("a%2Fb"));
        assert_eq!(decode("%E0%A4%A", false), None);
    }

    #[test]
    fn slices_like_javascript() {
        let chars: Vec<char> = "hello".chars().collect();
        assert_eq!(js_slice(&chars, -3.0, None), "llo");
        assert_eq!(js_slice(&chars, 1.7, Some(3.0)), "el");
        assert_eq!(utf16_substring("a𝄞b", 1, Some(3)), "𝄞");
    }
}
