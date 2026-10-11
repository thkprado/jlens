//! Regular expressions with JavaScript semantics, backed by `regress` (see ADR 0002).
//!
//! A regex literal evaluates to a function. Applied to a string it returns a match object
//! `{match, start, end, groups, next}` (or undefined), where `start`/`end` are UTF-16 offsets
//! and `next()` continues the global search, exactly like the closure jsonata-js creates.
//! Matches run through the evaluator, so its timeout and cancellation token stop them (ADR 0009).

use std::rc::Rc;

use crate::error::{Error, Result};
use crate::eval::Evaluator;
use crate::value::{Function, Map, Value};

/// A compiled regular expression literal.
pub(crate) struct Regex {
    pub re: regress::Regex,
    pub pattern: String,
}

impl Regex {
    /// Compiles `/pattern/flags`. Flags are `i` and `m`; matching is always global, as in
    /// jsonata-js.
    pub fn new(pattern: &str, flags: &str) -> Result<Regex> {
        let re = regress::Regex::with_flags(pattern, flags).map_err(|e| {
            // jsonata-js surfaces the JavaScript SyntaxError; jlens reports it as a regex
            // syntax error.
            Error::new("S0302")
                .with("value", pattern)
                .with("message", e.to_string())
        })?;
        Ok(Regex {
            re,
            pattern: pattern.to_owned(),
        })
    }
}

fn utf16_offset(text: &str, byte: usize) -> f64 {
    text[..byte].encode_utf16().count() as f64
}

/// Converts a UTF-16 offset into a byte offset (clamped to the end of the text).
pub(crate) fn byte_offset(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (i, c) in text.char_indices() {
        if units >= utf16 {
            return i;
        }
        units += c.len_utf16();
    }
    text.len()
}

/// Finds the first match at or after byte offset `from`.
pub(crate) fn exec(ev: &Evaluator, re: &Rc<Regex>, text: &Rc<str>, from: usize) -> Result<Value> {
    if from > text.len() {
        return Ok(Value::Undefined);
    }
    let Some(m) = ev.find_regex(&re.re, text, from)? else {
        return Ok(Value::Undefined);
    };
    let range = m.range();
    let groups: Vec<Value> = m
        .captures
        .iter()
        .map(|g| match g {
            Some(r) => Value::string(&text[r.clone()]),
            None => Value::Undefined,
        })
        .collect();
    let mut obj = Map::default();
    obj.insert("match".into(), Value::string(&text[range.clone()]));
    obj.insert(
        "start".into(),
        Value::Number(utf16_offset(text, range.start)),
    );
    obj.insert("end".into(), Value::Number(utf16_offset(text, range.end)));
    obj.insert("groups".into(), Value::array(groups, 0));
    obj.insert(
        "next".into(),
        Value::function(Function::RegexNext {
            re: re.clone(),
            text: text.clone(),
            last: range.end,
        }),
    );
    Ok(Value::object(obj))
}

/// The `next()` of a match object: the following match, or undefined at the end. A regex that
/// matches the empty string would never progress, so that raises D1004.
pub(crate) fn next(ev: &Evaluator, re: &Rc<Regex>, text: &Rc<str>, last: usize) -> Result<Value> {
    if last >= text.len() {
        return Ok(Value::Undefined);
    }
    let result = exec(ev, re, text, last)?;
    if let Value::Object(o) = &result
        && o.borrow().get("match").and_then(Value::as_str) == Some("")
    {
        return Err(Error::new("D1004").with("value", re.pattern.as_str()));
    }
    Ok(result)
}
