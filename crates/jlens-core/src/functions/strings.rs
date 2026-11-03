//! String functions, matchers, Base64 and URL encoding.
//!
//! JavaScript strings are sequences of UTF-16 code units. Where jsonata-js counts in code
//! points (`$length`, `$substring`, `$pad`) jlens counts `char`s; positions reported by
//! matchers (`$match`, regex literals) are UTF-16 offsets, as in jsonata-js.

use std::borrow::Cow;

use super::{Call, arg, num, text};
use crate::error::{Error, Result};
use crate::eval::Evaluator;
use crate::regex::{self, Matches, Offset};
use crate::value::{Map, Text, Value};

type R<'a> = Result<Value<'a>>;

/// The byte offset `chars` characters after the byte offset `at`, or the end of the text when it
/// has fewer. A count past `usize` saturates, which the end of the text stops anyway.
fn chars_after(s: &str, at: usize, chars: f64) -> usize {
    // `Chars::nth` skips by counting the bytes that start a character, faster than decoding them.
    let mut rest = s[at..].chars();
    rest.nth(chars as usize)
        .map_or(s.len(), |c| s.len() - rest.as_str().len() - c.len_utf8())
}

/// The bytes of `$substring(s, start, length)`, which jsonata-js takes with `Array.slice` over
/// the text's code points: a negative position counts from the end, a fractional one is cut
/// towards zero, and a negative start past the start of the text starts it. The text is read up
/// to where the result ends from the start, or from the end back to where it starts; the number
/// of characters it holds is counted only for a negative start or length that is not a whole
/// number, where it enters the arithmetic in floating point.
fn substring_range(s: &str, start: f64, length: Option<f64>) -> std::ops::Range<usize> {
    // A length of NaN gives an end of NaN, which `slice` reads as 0.
    if length.is_some_and(|l| l <= 0.0 || l.is_nan()) {
        return 0..0;
    }
    // A start of NaN is 0 to `slice`, but makes the end NaN.
    if start.is_nan() {
        return if length.is_some() { 0..0 } else { 0..s.len() };
    }
    // The text has at most as many characters as bytes, so that it has fewer than this start
    // asks to go back.
    if start < 0.0 && -start > s.len() as f64 {
        return substring_range(s, 0.0, length);
    }
    if start >= 0.0 {
        let to = length.map_or(f64::INFINITY, |l| (start + l).trunc());
        let from = start.trunc();
        if to <= from {
            return 0..0;
        }
        let a = chars_after(s, 0, from);
        return a..chars_after(s, a, to - from);
    }
    let whole = |v: f64| v.fract() == 0.0 || v.is_infinite();
    if whole(start) && length.is_none_or(whole) {
        // `-start` characters back from the end. The start is a whole number no further back
        // than the text has bytes, so that `len + start` is exact, and a length at least as long
        // as what is left takes the rest. Fewer characters than that, and the start is 0.
        let back = -start as usize;
        let Some((a, _)) = s.char_indices().rev().nth(back - 1) else {
            return substring_range(s, 0.0, length);
        };
        return match length {
            Some(l) if l < -start => a..chars_after(s, a, l),
            _ => a..s.len(),
        };
    }
    let len = s.chars().count() as f64;
    if len + start < 0.0 {
        return substring_range(s, 0.0, length);
    }
    let relative = |v: f64| {
        let v = v.trunc();
        if v < 0.0 {
            (len + v).max(0.0)
        } else {
            v.min(len)
        }
    };
    let from = relative(start);
    let to = length.map_or(len, |l| relative(len + start + l));
    if from >= to {
        return 0..0;
    }
    let a = chars_after(s, 0, from);
    a..chars_after(s, a, to - from)
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

/// Slices one string by UTF-16 offsets that mostly move forward, as the matches of a regular
/// expression do: each offset is looked for from the last one found, so that slicing a text
/// piece by piece walks it once. An offset behind the last one starts again from the start of
/// the text, and one inside a surrogate pair goes through [`utf16_substring`].
struct Utf16Slicer<'s> {
    text: &'s str,
    place: Offset,
}

impl<'s> Utf16Slicer<'s> {
    fn new(text: &'s str) -> Self {
        Utf16Slicer {
            text,
            place: Offset::START,
        }
    }

    /// The byte offset of a UTF-16 offset (the end of the text past it), or `None` inside a
    /// surrogate pair.
    fn seek(&mut self, utf16: usize) -> Option<usize> {
        if utf16 < self.place.utf16 {
            self.place = Offset::START;
        }
        for c in self.text[self.place.byte..].chars() {
            if self.place.utf16 >= utf16 {
                break;
            }
            self.place = self.place.after(c);
        }
        (self.place.utf16 <= utf16).then_some(self.place.byte)
    }

    /// `String.prototype.substring`: to the end of the text without `to`, and an end before the
    /// start swaps them.
    fn slice(&mut self, from: usize, to: Option<usize>) -> Cow<'s, str> {
        let to = to.unwrap_or(usize::MAX);
        match (self.seek(from.min(to)), self.seek(from.max(to))) {
            (Some(a), Some(b)) => Cow::Borrowed(&self.text[a..b]),
            _ => Cow::Owned(utf16_substring(self.text, from, Some(to))),
        }
    }
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

pub(super) fn substring<'a>(
    _: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    _: &Call<'_, 'a>,
) -> R<'a> {
    let Some(s) = text(&a, 0) else {
        return Ok(Value::Undefined);
    };
    let range = substring_range(&s, num(&a, 1).unwrap_or(f64::NAN), num(&a, 2));
    Ok(if range.len() == s.len() {
        Value::String(s)
    } else {
        Value::string(&s[range])
    })
}

pub(super) fn substring_before<'a>(
    _: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    _: &Call<'_, 'a>,
) -> R<'a> {
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

pub(super) fn substring_after<'a>(
    _: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    _: &Call<'_, 'a>,
) -> R<'a> {
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

pub(super) fn lowercase<'a>(
    _: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    _: &Call<'_, 'a>,
) -> R<'a> {
    Ok(text(&a, 0).map_or(Value::Undefined, |s| Value::string(s.to_lowercase())))
}

pub(super) fn uppercase<'a>(
    _: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    _: &Call<'_, 'a>,
) -> R<'a> {
    Ok(text(&a, 0).map_or(Value::Undefined, |s| Value::string(s.to_uppercase())))
}

pub(super) fn length<'a>(_: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, _: &Call<'_, 'a>) -> R<'a> {
    Ok(text(&a, 0).map_or(
        Value::Undefined,
        |s| Value::Number(s.chars().count() as f64),
    ))
}

pub(super) fn trim<'a>(_: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, _: &Call<'_, 'a>) -> R<'a> {
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

pub(super) fn pad<'a>(_: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, _: &Call<'_, 'a>) -> R<'a> {
    let Some(s) = text(&a, 0) else {
        return Ok(Value::Undefined);
    };
    let fill = text(&a, 2)
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| Text::from(" "));
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
fn evaluate_matcher<'a>(
    ev: &mut Evaluator<'_, 'a>,
    matcher: &Value<'a>,
    s: Value<'a>,
    call: &Call<'_, 'a>,
) -> R<'a> {
    let result = ev.callback(matcher, vec![s], call.env)?;
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

fn field<'a>(v: &Value<'a>, key: &str) -> Value<'a> {
    match v {
        Value::Object(o) => o.borrow().get(key).cloned().unwrap_or_default(),
        _ => Value::Undefined,
    }
}

fn field_usize<'a>(v: &Value<'a>, key: &str) -> usize {
    field(v, key).as_number().unwrap_or(0.0).max(0.0) as usize
}

pub(super) fn contains<'a>(
    ev: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    call: &Call<'_, 'a>,
) -> R<'a> {
    let (Some(s), token) = (text(&a, 0), arg(&a, 1)) else {
        return Ok(Value::Undefined);
    };
    if let Some(re) = regex::literal(&token) {
        return Ok(Value::Bool(Matches::new(re, &s).next(ev)?.is_some()));
    }
    Ok(match token {
        Value::Undefined => Value::Undefined,
        Value::String(t) => Value::Bool(s.contains(t.as_ref())),
        matcher => {
            Value::Bool(!evaluate_matcher(ev, &matcher, Value::String(s), call)?.is_undefined())
        }
    })
}

fn check_limit<'a>(a: &[Value<'a>], i: usize, code: &str, index: usize) -> Result<Option<f64>> {
    match num(a, i) {
        Some(l) if l < 0.0 => Err(Error::new(code).with("value", l).with("index", index)),
        other => Ok(other),
    }
}

pub(super) fn match_<'a>(
    ev: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    call: &Call<'_, 'a>,
) -> R<'a> {
    let Some(s) = text(&a, 0) else {
        return Ok(Value::Undefined);
    };
    let limit = check_limit(&a, 2, "D3040", 3)?;
    let mut out = Vec::new();
    let matcher = arg(&a, 1);
    if let Some(re) = regex::literal(&matcher)
        && limit.is_none_or(|l| l > 0.0)
    {
        // The loop of the general path below, without a match object for each match.
        let keys = &re.keys;
        let mut matches = Matches::new(re, &s);
        let mut found = matches.next(ev)?;
        let mut count = 0.0;
        while let Some(f) = found.take()
            && limit.is_none_or(|l| count < l)
        {
            let mut m = Map::with_capacity_and_hasher(3, Default::default());
            m.insert(keys.match_.clone(), Value::string(f.matched(&s)));
            m.insert(keys.index.clone(), Value::Number(f.start.utf16 as f64));
            m.insert(keys.groups.clone(), Value::array(f.group_values(&s), 0));
            out.push(Value::object(m));
            found = matches.next(ev)?;
            count += 1.0;
        }
    } else if limit.is_none_or(|l| l > 0.0) {
        let mut matches = evaluate_matcher(ev, &matcher, Value::String(s), call)?;
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

/// Expands `$0`, `$1`, ... and `$$` in a replacement string into `out` (jsonata-js `replace`):
/// `whole` is the text matched, `group_count` the number of groups and `group(i)` group `i`,
/// counted from 0, when it is a string.
fn substitute<'g>(
    out: &mut String,
    replacement: &str,
    whole: Option<&str>,
    group_count: usize,
    group: impl Fn(usize) -> Option<&'g str>,
) {
    // `$` and digits are ASCII, so the text is walked by bytes and cut only beside them.
    let bytes = replacement.as_bytes();
    let mut position = 0;
    while position < bytes.len() {
        let Some(offset) = bytes[position..].iter().position(|&c| c == b'$') else {
            break;
        };
        let index = position + offset;
        out.push_str(&replacement[position..index]);
        position = index + 1;
        match bytes.get(position) {
            Some(b'$') => {
                out.push('$');
                position += 1;
            }
            Some(b'0') => {
                if let Some(m) = whole {
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
                    let digits = bytes[position..]
                        .iter()
                        .take(n)
                        .take_while(|c| c.is_ascii_digit())
                        .count();
                    replacement[position..position + digits].parse().ok()
                };
                let mut idx = parse(max_digits);
                if max_digits > 1 && idx.is_some_and(|i| i > group_count) {
                    idx = parse(max_digits - 1);
                }
                match idx {
                    Some(i) => {
                        if group_count > 0
                            && i >= 1
                            && let Some(sub) = group(i - 1)
                        {
                            out.push_str(sub);
                        }
                        position += i.to_string().len();
                    }
                    None => out.push('$'),
                }
            }
        }
    }
    if position < bytes.len() {
        out.push_str(&replacement[position..]);
    }
}

/// [`substitute`] for a match object.
fn substitute_object<'a>(out: &mut String, replacement: &str, matched: &Value<'a>) {
    let groups = field(matched, "groups");
    let groups: &[Value<'a>] = match &groups {
        Value::Array(g) => g.items(),
        _ => &[],
    };
    let whole = field(matched, "match");
    substitute(out, replacement, whole.as_str(), groups.len(), |i| {
        groups.get(i).and_then(Value::as_str)
    });
}

pub(super) fn replace<'a>(
    ev: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    call: &Call<'_, 'a>,
) -> R<'a> {
    let (Some(s), pattern) = (text(&a, 0), arg(&a, 1)) else {
        return Ok(Value::Undefined);
    };
    if pattern.as_str() == Some("") {
        return Err(Error::new("D3010").with("value", "").with("index", 2));
    }
    // jsonata-js checks the limit before it uses the pattern.
    let limit = check_limit(&a, 3, "D3011", 4)?;
    if pattern.is_undefined() {
        return Ok(Value::Undefined);
    }
    // A missing replacement removes what the pattern finds, whatever the pattern. jsonata-js
    // writes "undefined" for a string pattern and fails with a `TypeError` for a regex
    // (compatibility.md).
    let replacement = match arg(&a, 2) {
        Value::Undefined => Value::string(""),
        r => r,
    };
    if limit.is_some_and(|l| l <= 0.0) {
        return Ok(Value::String(s));
    }
    let mut result = String::new();
    let mut count = 0.0;

    if let Value::String(p) = &pattern {
        let replacement = replacement.as_str().unwrap_or_default();
        let mut position = 0;
        while let Some(found) = s[position..].find(p.as_ref()) {
            if limit.is_some_and(|l| count >= l) {
                break;
            }
            let index = position + found;
            result.push_str(&s[position..index]);
            result.push_str(replacement);
            position = index + p.len();
            count += 1.0;
        }
        result.push_str(&s[position..]);
        return Ok(Value::string(result));
    }

    // A regex literal with a replacement string: the loop below, without a match object for each
    // match. A function as the replacement is given the match object, so it takes that loop.
    if let (Some(re), Value::String(r)) = (regex::literal(&pattern), &replacement) {
        let mut matches = Matches::new(re, &s);
        let mut found = matches.next(ev)?;
        if found.is_none() {
            return Ok(Value::String(s));
        }
        let mut position = Offset::START;
        while let Some(f) = found.take()
            && limit.is_none_or(|l| count < l)
        {
            result.push_str(&s[position.byte..f.start.byte]);
            substitute(&mut result, r, Some(f.matched(&s)), f.groups.len(), |i| {
                f.group(&s, i)
            });
            position = f.end;
            count += 1.0;
            found = matches.next(ev)?;
        }
        result.push_str(&s[position.byte..]);
        return Ok(Value::string(result));
    }

    let mut matches = evaluate_matcher(ev, &pattern, Value::String(s.clone()), call)?;
    if matches.is_undefined() {
        return Ok(Value::String(s));
    }
    let mut slicer = Utf16Slicer::new(&s);
    let mut position = 0; // UTF-16 offset
    while !matches.is_undefined() && limit.is_none_or(|l| count < l) {
        let start = field_usize(&matches, "start");
        result.push_str(&slicer.slice(position, Some(start)));
        match &replacement {
            Value::String(r) => substitute_object(&mut result, r, &matches),
            func => match ev.callback(func, vec![matches.clone()], call.env)? {
                Value::String(r) => result.push_str(&r),
                other => {
                    let mut err = Error::new("D3012");
                    if let Some(v) = other.to_json_detail() {
                        err = err.with("value", v);
                    }
                    return Err(err);
                }
            },
        }
        let matched_len = field(&matches, "match").as_str().map_or(0, utf16_len);
        position = start + matched_len;
        count += 1.0;
        matches = evaluate_matcher(ev, &field(&matches, "next"), Value::Undefined, call)?;
    }
    result.push_str(&slicer.slice(position, None));
    Ok(Value::string(result))
}

pub(super) fn split<'a>(
    ev: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    call: &Call<'_, 'a>,
) -> R<'a> {
    let (Some(s), separator) = (text(&a, 0), arg(&a, 1)) else {
        return Ok(Value::Undefined);
    };
    // jsonata-js checks the limit before it uses the separator.
    let limit = check_limit(&a, 2, "D3020", 3)?;
    if separator.is_undefined() {
        return Ok(Value::Undefined);
    }
    let mut out: Vec<Value<'a>> = Vec::new();
    if limit.is_none_or(|l| l > 0.0) {
        if let Value::String(sep) = &separator {
            let parts: Vec<Value<'a>> = if sep.is_empty() {
                s.chars().map(|c| Value::string(c.to_string())).collect()
            } else {
                s.split(sep.as_ref()).map(Value::string).collect()
            };
            let max = limit.map_or(usize::MAX, |l| l as usize);
            out = parts.into_iter().take(max).collect();
        } else if let Some(re) = regex::literal(&separator) {
            // The loop below, without a match object for each match.
            let mut count = 0.0;
            let mut matches = Matches::new(re, &s);
            let mut found = matches.next(ev)?;
            if found.is_none() {
                out.push(Value::String(s));
            } else {
                let mut start = Offset::START;
                while let Some(f) = found.take()
                    && limit.is_none_or(|l| count < l)
                {
                    out.push(Value::string(&s[start.byte..f.start.byte]));
                    start = f.end;
                    found = matches.next(ev)?;
                    count += 1.0;
                }
                if limit.is_none_or(|l| count < l) {
                    out.push(Value::string(&s[start.byte..]));
                }
            }
        } else {
            let mut count = 0.0;
            let mut matches = evaluate_matcher(ev, &separator, Value::String(s.clone()), call)?;
            if matches.is_undefined() {
                out.push(Value::String(s));
            } else {
                let mut slicer = Utf16Slicer::new(&s);
                let mut start = 0;
                while !matches.is_undefined() && limit.is_none_or(|l| count < l) {
                    let m_start = field_usize(&matches, "start");
                    out.push(Value::string(slicer.slice(start, Some(m_start))));
                    start = field_usize(&matches, "end");
                    matches =
                        evaluate_matcher(ev, &field(&matches, "next"), Value::Undefined, call)?;
                    count += 1.0;
                }
                if limit.is_none_or(|l| count < l) {
                    out.push(Value::string(slicer.slice(start, None)));
                }
            }
        }
    }
    Ok(Value::array(out, 0))
}

pub(super) fn join<'a>(ev: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, _: &Call<'_, 'a>) -> R<'a> {
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

pub(super) fn base64encode<'a>(
    _: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    _: &Call<'_, 'a>,
) -> R<'a> {
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

pub(super) fn base64decode<'a>(
    _: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    _: &Call<'_, 'a>,
) -> R<'a> {
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

/// The ASCII bytes that `encodeURIComponent` keeps as they are, or, with `reserved`, those that
/// `encodeURI` keeps. Every other byte, each byte of a character past ASCII included, is escaped.
const fn kept(reserved: bool) -> [bool; 128] {
    const fn holds(set: &str, b: u8) -> bool {
        let set = set.as_bytes();
        let mut i = 0;
        while i < set.len() {
            if set[i] == b {
                return true;
            }
            i += 1;
        }
        false
    }
    let mut table = [false; 128];
    let mut b = 0;
    while b < 128 {
        let c = b as u8;
        table[b] = c.is_ascii_alphanumeric()
            || holds(URI_UNRESERVED, c)
            || (reserved && holds(URI_RESERVED, c));
        b += 1;
    }
    table
}

const KEPT_IN_COMPONENT: [bool; 128] = kept(false);
const KEPT_IN_URL: [bool; 128] = kept(true);

/// Escapes each byte of `s` that `keep` does not hold as `%XX`, and copies the runs of those it
/// holds whole. The result's length is counted first, so that it is allocated once.
fn encode(s: &str, keep: &[bool; 128]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let escaped = |b: u8| b >= 0x80 || !keep[usize::from(b)];
    let bytes = s.as_bytes();
    let count = bytes.iter().filter(|&&b| escaped(b)).count();
    let mut out = String::with_capacity(s.len() + 2 * count);
    // Where the bytes kept as they are, not yet copied, start: always a character's start, as
    // every byte of a character past ASCII is escaped.
    let mut run = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if !escaped(b) {
            continue;
        }
        if run < i {
            out.push_str(&s[run..i]);
        }
        out.push('%');
        out.push(char::from(HEX[usize::from(b >> 4)]));
        out.push(char::from(HEX[usize::from(b & 0xF)]));
        run = i + 1;
    }
    if run < bytes.len() {
        out.push_str(&s[run..]);
    }
    out
}

/// Decodes the escapes of `s` as `decodeURIComponent` does, or, with `keep_reserved`, as
/// `decodeURI` does, which leaves those of reserved characters as they are written. `None` for
/// what JavaScript raises a `URIError` for: an escape whose two digits are not hexadecimal, or
/// bytes that are not a character in UTF-8 (a lone continuation byte, an overlong form, a
/// surrogate, a character past U+10FFFF).
fn decode(s: &str, keep_reserved: bool) -> Option<String> {
    let bytes = s.as_bytes();
    // Two hexadecimal digits, no more: `u8::from_str_radix` would also take `+1`.
    let hex = |i: usize| -> Option<u8> {
        let digit = |b: &u8| char::from(*b).to_digit(16);
        let high = digit(bytes.get(i)?)?;
        let low = digit(bytes.get(i + 1)?)?;
        u8::try_from(high * 16 + low).ok()
    };
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        // The text up to the next escape, copied whole: `%` is ASCII, so that it starts and
        // ends at characters' starts.
        let Some(skip) = bytes[i..].iter().position(|&b| b == b'%') else {
            out.push_str(&s[i..]);
            break;
        };
        out.push_str(&s[i..i + skip]);
        i += skip;
        let first = hex(i + 1)?;
        if first < 0x80 {
            if keep_reserved && URI_RESERVED.as_bytes().contains(&first) {
                out.push_str(&s[i..i + 3]);
            } else {
                out.push(char::from(first));
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
        let mut buf = [first, 0, 0, 0];
        for (k, slot) in buf.iter_mut().enumerate().take(len).skip(1) {
            let at = i + 3 * k;
            if bytes.get(at) != Some(&b'%') {
                return None;
            }
            let b = hex(at + 1)?;
            if b & 0xC0 != 0x80 {
                return None;
            }
            *slot = b;
        }
        out.push_str(std::str::from_utf8(&buf[..len]).ok()?);
        i += 3 * len;
    }
    Some(out)
}

fn url_error(name: &str, s: &str) -> Error {
    Error::new("D3140")
        .with("value", s)
        .with("functionName", name)
}

pub(super) fn encode_url_component<'a>(
    _: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    _: &Call<'_, 'a>,
) -> R<'a> {
    Ok(text(&a, 0).map_or(Value::Undefined, |s| {
        Value::string(encode(&s, &KEPT_IN_COMPONENT))
    }))
}

pub(super) fn encode_url<'a>(
    _: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    _: &Call<'_, 'a>,
) -> R<'a> {
    Ok(text(&a, 0).map_or(Value::Undefined, |s| {
        Value::string(encode(&s, &KEPT_IN_URL))
    }))
}

pub(super) fn decode_url_component<'a>(
    _: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    _: &Call<'_, 'a>,
) -> R<'a> {
    let Some(s) = text(&a, 0) else {
        return Ok(Value::Undefined);
    };
    decode(&s, false)
        .map(Value::string)
        .ok_or_else(|| url_error("decodeUrlComponent", &s))
}

pub(super) fn decode_url<'a>(
    _: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    _: &Call<'_, 'a>,
) -> R<'a> {
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
        assert_eq!(
            encode("a b/ç?x=1", &KEPT_IN_COMPONENT),
            "a%20b%2F%C3%A7%3Fx%3D1"
        );
        assert_eq!(encode("a b/ç?x=1", &KEPT_IN_URL), "a%20b/%C3%A7?x=1");
        assert_eq!(decode("a%20b%2F%C3%A7", false).as_deref(), Some("a b/ç"));
        assert_eq!(decode("a%2Fb", true).as_deref(), Some("a%2Fb"));
        assert_eq!(decode("%E0%A4%A", false), None);
    }

    /// An escape is two hexadecimal digits, as in JavaScript: before 0.27, `%+1` decoded to
    /// U+0001, as `u8::from_str_radix` takes a sign.
    #[test]
    fn an_escape_is_two_hex_digits() {
        for s in [
            "%+1", "%-1", "%+F", "%1", "%", "%G0", "%0G", "%é0", "%C3%+7",
        ] {
            assert_eq!(decode(s, false), None, "{s}");
            assert_eq!(decode(s, true), None, "{s}");
        }
        assert_eq!(decode("%c3%a7%2b", false).as_deref(), Some("ç+"));
        assert_eq!(decode("%c3%a7%2b", true).as_deref(), Some("ç%2b"));
    }

    /// `encode` as it was before 0.27, a character at a time, each escape formatted.
    fn encode_by_chars(s: &str, keep: &str) -> String {
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

    /// `decode` as it was before 0.27, but for the `+` that `u8::from_str_radix` took.
    fn decode_by_chars(s: &str, keep_reserved: bool) -> Option<String> {
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

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(20_000))]

        /// Every ASCII character, those of two to four bytes, and lone `%`, around one another:
        /// the escapes are what the code before 0.27 wrote, and decode back.
        #[test]
        fn url_encoding_gives_what_it_gave(s in "([\\x00-\\x7f]|[ç€𝄞]){0,12}") {
            for (keep, reserved) in [(&KEPT_IN_COMPONENT, ""), (&KEPT_IN_URL, URI_RESERVED)] {
                let encoded = encode(&s, keep);
                proptest::prop_assert_eq!(&encoded, &encode_by_chars(&s, reserved));
                proptest::prop_assert_eq!(decode(&encoded, false), Some(s.clone()));
            }
        }

        /// Escapes well and badly formed, of characters of one to four bytes, lone bytes, reserved
        /// characters in either case, a sign: decoding gives what the code before 0.27 gave, and
        /// fails where it failed, but where an escape starts with `+`, which it took.
        #[test]
        fn url_decoding_gives_what_it_gave(
            s in "([a%+;/?#ç€𝄞 -]|%[0-9A-Fa-f+]{0,2}|%[2-3][0-9A-Fa-f]|%[C-Fc-f][0-9A-F]%[7-9A-Ca-c][0-9A-F]){0,8}",
        ) {
            let signed = s.as_bytes().windows(2).any(|w| w == b"%+");
            for keep_reserved in [false, true] {
                let decoded = decode(&s, keep_reserved);
                if signed && decoded.is_none() {
                    continue;
                }
                proptest::prop_assert_eq!(decoded, decode_by_chars(&s, keep_reserved));
            }
        }
    }

    #[test]
    fn slices_like_javascript() {
        let sub = |s, start, length| &s[substring_range(s, start, length)];
        assert_eq!(sub("hello", -3.0, None), "llo");
        assert_eq!(sub("hello", 1.7, Some(1.5)), "el");
        assert_eq!(sub("hello", -3.5, Some(2.0)), "l");
        assert_eq!(sub("hello", -5.5, Some(1.0)), "h");
        assert_eq!(sub("a𝄞b", -2.0, Some(1.0)), "𝄞");
        assert_eq!(utf16_substring("a𝄞b", 1, Some(3)), "𝄞");
    }

    /// `$substring` as it was before 0.27: every character collected into a list, and sliced as
    /// `Array.prototype.slice` slices.
    fn substring_by_list(s: &str, start: f64, length: Option<f64>) -> String {
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len() as f64;
        let slice = |start: f64, end: Option<f64>| -> String {
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
        };
        let mut start = start;
        if len + start < 0.0 {
            start = 0.0;
        }
        match length {
            Some(length) if length <= 0.0 => String::new(),
            Some(length) => {
                let end = if start >= 0.0 {
                    start + length
                } else {
                    len + start + length
                };
                slice(start, Some(end))
            }
            None => slice(start, None),
        }
    }

    /// A position near the text's characters: whole or not, either sign, past either end, and
    /// the values that are not numbers JSONata can write but that a host function can return.
    fn position() -> impl proptest::strategy::Strategy<Value = f64> {
        use proptest::prelude::*;
        prop_oneof![
            4 => (-12i32..12).prop_map(f64::from),
            3 => (-1200i32..1200).prop_map(|v| f64::from(v) / 100.0),
            1 => prop::sample::select(vec![
                f64::NAN,
                f64::INFINITY,
                f64::NEG_INFINITY,
                -0.0,
                1e300,
                -1e300,
                // Added to a whole number, rounds up to the next one.
                1.0 - f64::EPSILON / 2.0,
                -(1.0 - f64::EPSILON / 2.0),
                9_007_199_254_740_993.0,
            ]),
        ]
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(20_000))]

        /// The text is read no further than `$substring` needs, and what it gives is what
        /// collecting every character gave, around characters of one to four bytes.
        #[test]
        fn substring_gives_what_the_list_gave(
            s in "[aé€𝄞]{0,10}",
            start in position(),
            length in proptest::option::of(position()),
        ) {
            let range = substring_range(&s, start, length);
            proptest::prop_assert_eq!(&s[range], substring_by_list(&s, start, length));
        }
    }

    /// Whatever it looked for before, a slicer gives what `utf16_substring` gives: inside
    /// surrogate pairs, backwards and past the end of the text included.
    #[test]
    fn a_slicer_gives_what_substring_gives() {
        let text = "a𝄞é𝄞";
        let offsets = 0..=8;
        let mut slicer = Utf16Slicer::new(text);
        for from in offsets.clone() {
            for to in offsets.clone() {
                assert_eq!(
                    slicer.slice(from, Some(to)),
                    utf16_substring(text, from, Some(to)),
                    "{from}..{to}"
                );
                assert_eq!(
                    slicer.slice(to, None),
                    utf16_substring(text, to, None),
                    "{to}.."
                );
            }
        }
    }
}
