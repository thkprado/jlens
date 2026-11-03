//! Regular expressions with JavaScript semantics, backed by `regress` (see ADR 0002).
//!
//! A regex literal evaluates to a function. Applied to a string it returns a match object
//! `{match, start, end, groups, next}` (or undefined), where `start`/`end` are UTF-16 offsets
//! and `next()` continues the global search, exactly like the closure jsonata-js creates.
//! `$match`, `$replace`, `$split` and `$contains` run a literal's searches themselves
//! ([`Matches`]), with no match object where nothing can see one.
//! Matches run through the evaluator, so its timeout and cancellation token stop them (ADR 0009).

use std::ops::Range;
use std::rc::Rc;

use crate::error::{Error, Result};
use crate::eval::Evaluator;
use crate::value::{Function, Map, Value};

/// A compiled regular expression literal.
pub(crate) struct Regex {
    pub re: regress::Regex,
    pub pattern: String,
    /// The keys of the objects made for its matches, made once for each evaluation.
    pub keys: Keys,
}

/// The keys of a match object, `{match, start, end, groups, next}`, and `index`, which `$match`
/// gives its matches in place of `start`.
pub(crate) struct Keys {
    pub match_: Rc<str>,
    pub start: Rc<str>,
    pub end: Rc<str>,
    pub groups: Rc<str>,
    pub next: Rc<str>,
    pub index: Rc<str>,
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
            keys: Keys {
                match_: "match".into(),
                start: "start".into(),
                end: "end".into(),
                groups: "groups".into(),
                next: "next".into(),
                index: "index".into(),
            },
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

/// A match: where it starts and ends, in both units, and the byte ranges of its groups.
pub(crate) struct Found {
    pub start: Offset,
    pub end: Offset,
    pub groups: Vec<Option<Range<usize>>>,
}

impl Found {
    /// The text matched.
    pub fn matched<'t>(&self, text: &'t str) -> &'t str {
        &text[self.start.byte..self.end.byte]
    }

    /// Group `i`, counted from 0 (the first parenthesis), when it took part in the match.
    pub fn group<'t>(&self, text: &'t str, i: usize) -> Option<&'t str> {
        self.groups.get(i)?.clone().map(|r| &text[r])
    }

    /// The groups as a match object holds them: a string each, undefined where a group took no
    /// part.
    pub fn group_values<'a>(&self, text: &str) -> Vec<Value<'a>> {
        (0..self.groups.len())
            .map(|i| self.group(text, i).map_or(Value::Undefined, Value::string))
            .collect()
    }
}

/// Finds the first match at or after `from`.
pub(crate) fn find(
    ev: &Evaluator<'_, '_>,
    re: &Regex,
    text: &str,
    from: Offset,
) -> Result<Option<Found>> {
    let Some(m) = ev.find_regex(&re.re, text, from.byte)? else {
        return Ok(None);
    };
    let range = m.range();
    let start = from.advanced_to(text, range.start);
    let end = start.advanced_to(text, range.end);
    Ok(Some(Found {
        start,
        end,
        groups: m.captures,
    }))
}

/// The match after the one that ended at `last`, as `next()` finds it: none at the end of the
/// text, and D1004 for an empty match, as a regex that matches the empty string would never
/// progress.
fn find_next(
    ev: &Evaluator<'_, '_>,
    re: &Regex,
    text: &str,
    last: Offset,
) -> Result<Option<Found>> {
    if last.byte >= text.len() {
        return Ok(None);
    }
    let found = find(ev, re, text, last)?;
    if found.as_ref().is_some_and(|f| f.start.byte == f.end.byte) {
        return Err(Error::new("D1004").with("value", re.pattern.as_str()));
    }
    Ok(found)
}

/// The match object of a match, `{match, start, end, groups, next}`.
fn object<'a>(re: &Rc<Regex>, text: &Rc<str>, found: Option<Found>) -> Value<'a> {
    let Some(found) = found else {
        return Value::Undefined;
    };
    let keys = &re.keys;
    let mut obj = Map::with_capacity_and_hasher(5, Default::default());
    obj.insert(keys.match_.clone(), Value::string(found.matched(text)));
    obj.insert(keys.start.clone(), Value::Number(found.start.utf16 as f64));
    obj.insert(keys.end.clone(), Value::Number(found.end.utf16 as f64));
    obj.insert(
        keys.groups.clone(),
        Value::array(found.group_values(text), 0),
    );
    obj.insert(
        keys.next.clone(),
        Value::function(Function::RegexNext {
            re: re.clone(),
            text: text.clone(),
            last: found.end,
        }),
    );
    Value::object(obj)
}

/// A regex literal applied to a text: the match object of the first match at or after `from`.
pub(crate) fn exec<'a>(
    ev: &Evaluator<'_, 'a>,
    re: &Rc<Regex>,
    text: &Rc<str>,
    from: Offset,
) -> Result<Value<'a>> {
    Ok(object(re, text, find(ev, re, text, from)?))
}

/// The `next()` of a match object: the following match, or undefined at the end.
pub(crate) fn next<'a>(
    ev: &Evaluator<'_, 'a>,
    re: &Rc<Regex>,
    text: &Rc<str>,
    last: Offset,
) -> Result<Value<'a>> {
    Ok(object(re, text, find_next(ev, re, text, last)?))
}

/// The matches of a regex literal over a text, for `$match`, `$replace`, `$split` and
/// `$contains` to run without a match object each. Each match costs what the matcher called
/// back costs, its step and its checks included ([`Evaluator::callback_step`]), and is found as
/// the regex and the `next()` of its match objects find it, with the same errors; so a function
/// gives the same results, errors and step count as it does through match objects.
pub(crate) struct Matches<'r> {
    re: &'r Regex,
    text: &'r str,
    /// Where the last match ended; `None` before the first.
    last: Option<Offset>,
}

impl<'r> Matches<'r> {
    pub fn new(re: &'r Regex, text: &'r str) -> Self {
        Matches {
            re,
            text,
            last: None,
        }
    }

    /// The next match, or none after the last.
    pub fn next(&mut self, ev: &mut Evaluator<'_, '_>) -> Result<Option<Found>> {
        ev.callback_step()?;
        let found = match self.last {
            None => find(ev, self.re, self.text, Offset::START)?,
            Some(last) => find_next(ev, self.re, self.text, last)?,
        };
        if let Some(f) = &found {
            self.last = Some(f.end);
        }
        Ok(found)
    }
}

/// The regex of a regex literal, which `$match` and the others can run themselves.
pub(crate) fn literal<'v>(v: &'v Value<'_>) -> Option<&'v Regex> {
    match v {
        Value::Function(f) => match &**f {
            Function::Regex(re) => Some(re),
            _ => None,
        },
        _ => None,
    }
}
