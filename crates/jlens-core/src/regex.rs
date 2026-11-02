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

/// A place in a text, in both of its units. Each match counts its UTF-16 offsets from the place
/// the search started, so the matches of a text together count it once.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Offset {
    pub byte: usize,
    pub utf16: usize,
}

impl Offset {
    pub const START: Offset = Offset { byte: 0, utf16: 0 };

    /// The first character boundary at or after a UTF-16 offset (the end of the text past it).
    pub fn from_utf16(text: &str, utf16: usize) -> Offset {
        let mut place = Offset::START;
        for c in text.chars() {
            if place.utf16 >= utf16 {
                break;
            }
            place = place.after(c);
        }
        place
    }

    /// The place after `c`, which starts at this one.
    pub fn after(self, c: char) -> Offset {
        Offset {
            byte: self.byte + c.len_utf8(),
            utf16: self.utf16 + c.len_utf16(),
        }
    }

    /// The place `byte`, at or after this one.
    fn advanced_to(self, text: &str, byte: usize) -> Offset {
        Offset {
            byte,
            utf16: self.utf16 + text[self.byte..byte].encode_utf16().count(),
        }
    }
}

/// Finds the first match at or after `from`.
pub(crate) fn exec<'a>(
    ev: &Evaluator<'_, 'a>,
    re: &Rc<Regex>,
    text: &Rc<str>,
    from: Offset,
) -> Result<Value<'a>> {
    let Some(m) = ev.find_regex(&re.re, text, from.byte)? else {
        return Ok(Value::Undefined);
    };
    let range = m.range();
    let start = from.advanced_to(text, range.start);
    let end = start.advanced_to(text, range.end);
    let groups: Vec<Value<'a>> = m
        .captures
        .iter()
        .map(|g| match g {
            Some(r) => Value::string(&text[r.clone()]),
            None => Value::Undefined,
        })
        .collect();
    let mut obj = Map::default();
    obj.insert("match".into(), Value::string(&text[range]));
    obj.insert("start".into(), Value::Number(start.utf16 as f64));
    obj.insert("end".into(), Value::Number(end.utf16 as f64));
    obj.insert("groups".into(), Value::array(groups, 0));
    obj.insert(
        "next".into(),
        Value::function(Function::RegexNext {
            re: re.clone(),
            text: text.clone(),
            last: end,
        }),
    );
    Ok(Value::object(obj))
}

/// The `next()` of a match object: the following match, or undefined at the end. A regex that
/// matches the empty string would never progress, so that raises D1004.
pub(crate) fn next<'a>(
    ev: &Evaluator<'_, 'a>,
    re: &Rc<Regex>,
    text: &Rc<str>,
    last: Offset,
) -> Result<Value<'a>> {
    if last.byte >= text.len() {
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
