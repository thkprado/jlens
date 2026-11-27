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
use std::sync::{Arc, OnceLock};

use crate::ast::{Ast, Node, NodeId, NodeKind};
use crate::error::{Error, Result};
use crate::eval::Evaluator;
use crate::value::{Function, Map, Value};

/// A regular expression literal, compiled: shared by the evaluations of an expression
/// ([`Patterns`]).
pub(crate) struct Pattern {
    pub re: regress::Regex,
    pub source: String,
    pub flags: String,
}

/// A regular expression literal as one evaluation holds it: its compiled pattern, and the keys
/// of the objects made for its matches, which the evaluation's literals share.
pub(crate) struct Regex {
    pub pattern: Arc<Pattern>,
    pub keys: Rc<Keys>,
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

impl Keys {
    pub fn new() -> Keys {
        Keys {
            match_: "match".into(),
            start: "start".into(),
            end: "end".into(),
            groups: "groups".into(),
            next: "next".into(),
            index: "index".into(),
        }
    }
}

impl Pattern {
    /// Compiles `/pattern/flags`. Flags are `i`, `m`, and the extensions `u` and `v` (Unicode
    /// mode, and Unicode mode with set operations on classes); matching is always global, as in
    /// jsonata-js. A flag given twice, or `u` with `v`, is a syntax error, as in JavaScript.
    pub fn new(pattern: &str, flags: &str) -> Result<Pattern> {
        if let Some(message) = invalid_flags(flags) {
            return Err(Error::new("S0302")
                .with("value", pattern)
                .with("flags", flags)
                .with("message", message));
        }
        let re = regress::Regex::with_flags(pattern, flags).map_err(|e| {
            // jsonata-js surfaces the JavaScript SyntaxError; jlens reports it as a regex
            // syntax error.
            Error::new("S0302")
                .with("value", pattern)
                .with("flags", flags)
                .with("message", e.to_string())
        })?;
        Ok(Pattern {
            re,
            source: pattern.to_owned(),
            flags: flags.to_owned(),
        })
    }
}

/// The regex literals of a compiled expression, each compiled the first time an evaluation
/// meets it and shared by every evaluation after it, on any thread. Listed by node id the first
/// time one is sought; a node is found by its address, so that a node of another tree (one
/// `$eval` parsed) with the same id is never taken for it.
#[derive(Default)]
pub(crate) struct Patterns(OnceLock<Box<[Slot]>>);

struct Slot {
    id: NodeId,
    /// The node's address in the expression's tree, which an `Expression` never moves or
    /// changes; only compared.
    node: usize,
    pattern: OnceLock<Arc<Pattern>>,
}

impl Patterns {
    /// The compiled pattern of `node`, a regex literal of `ast`, compiled now if no evaluation
    /// has. `None` for a node of another tree.
    pub fn get(
        &self,
        ast: &Ast,
        node: &Node,
        pattern: &str,
        flags: &str,
    ) -> Option<Result<Arc<Pattern>>> {
        let slots = self.0.get_or_init(|| {
            let mut slots = Vec::new();
            ast.root.walk(&mut |n| {
                if let NodeKind::Regex { .. } = n.kind {
                    slots.push(Slot {
                        id: n.id,
                        node: std::ptr::from_ref(n).addr(),
                        pattern: OnceLock::new(),
                    });
                }
            });
            slots.sort_unstable_by_key(|s| s.id);
            slots.into_boxed_slice()
        });
        let slot = &slots[slots.binary_search_by_key(&node.id, |s| s.id).ok()?];
        if slot.node != std::ptr::from_ref(node).addr() {
            return None;
        }
        if let Some(compiled) = slot.pattern.get() {
            return Some(Ok(compiled.clone()));
        }
        // Two threads may both compile it; the first to finish is kept. An invalid pattern is
        // not kept: its error is the rare path.
        Some(
            Pattern::new(pattern, flags)
                .map(|compiled| slot.pattern.get_or_init(|| Arc::new(compiled)).clone()),
        )
    }
}

impl std::fmt::Debug for Patterns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let compiled = self.0.get().map_or(0, |slots| {
            slots.iter().filter(|s| s.pattern.get().is_some()).count()
        });
        f.debug_struct("Patterns")
            .field("compiled", &compiled)
            .finish()
    }
}

/// Why a regex literal's flags are invalid, if they are. The lexer only takes `i`, `m`, `u` and
/// `v`; `regress` would take a repeated flag, and `u` with `v`, which JavaScript rejects.
fn invalid_flags(flags: &str) -> Option<String> {
    let bytes = flags.as_bytes();
    if let Some(i) = (1..bytes.len()).find(|&i| bytes[..i].contains(&bytes[i])) {
        return Some(format!("the flag '{}' is given twice", bytes[i] as char));
    }
    (bytes.contains(&b'u') && bytes.contains(&b'v'))
        .then(|| "the flags 'u' and 'v' cannot be given together".to_owned())
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

/// Finds the first match at or after `from` in the searcher's text, `text`.
fn find(
    ev: &Evaluator<'_, '_>,
    searcher: &mut regress::Searcher<'_, '_>,
    text: &str,
    from: Offset,
) -> Result<Option<Found>> {
    let Some(m) = ev.find_regex(searcher, from.byte)? else {
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
    searcher: &mut regress::Searcher<'_, '_>,
    text: &str,
    last: Offset,
) -> Result<Option<Found>> {
    if last.byte >= text.len() {
        return Ok(None);
    }
    let found = find(ev, searcher, text, last)?;
    if found.as_ref().is_some_and(|f| f.start.byte == f.end.byte) {
        return Err(Error::new("D1004").with("value", re.pattern.source.as_str()));
    }
    Ok(found)
}

/// The match object of a match, `{match, start, end, groups, next}`.
fn object<'a>(re: &Rc<Regex>, text: &Rc<str>, found: Option<Found>) -> Value<'a> {
    let Some(found) = found else {
        return Value::Undefined;
    };
    let keys = &re.keys;
    let mut obj = Map::with_capacity(5);
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
    let found = find(ev, &mut re.pattern.re.searcher(text), text, from)?;
    Ok(object(re, text, found))
}

/// The `next()` of a match object: the following match, or undefined at the end.
pub(crate) fn next<'a>(
    ev: &Evaluator<'_, 'a>,
    re: &Rc<Regex>,
    text: &Rc<str>,
    last: Offset,
) -> Result<Value<'a>> {
    let found = find_next(ev, re, &mut re.pattern.re.searcher(text), text, last)?;
    Ok(object(re, text, found))
}

/// The matches of a regex literal over a text, for `$match`, `$replace`, `$split` and
/// `$contains` to run without a match object each. Each match costs what the matcher called
/// back costs, its step and its checks included ([`Evaluator::callback_step`]), and is found as
/// the regex and the `next()` of its match objects find it, with the same errors; so a function
/// gives the same results, errors and step count as it does through match objects.
pub(crate) struct Matches<'r> {
    re: &'r Regex,
    text: &'r str,
    /// Keeps the backtracker's storage from one search to the next.
    searcher: regress::Searcher<'r, 'r>,
    /// Where the last match ended; `None` before the first.
    last: Option<Offset>,
}

impl<'r> Matches<'r> {
    pub fn new(re: &'r Regex, text: &'r str) -> Self {
        Matches {
            re,
            text,
            searcher: re.pattern.re.searcher(text),
            last: None,
        }
    }

    /// The next match, or none after the last.
    pub fn next(&mut self, ev: &mut Evaluator<'_, '_>) -> Result<Option<Found>> {
        ev.callback_step()?;
        let found = match self.last {
            None => find(ev, &mut self.searcher, self.text, Offset::START)?,
            Some(last) => find_next(ev, self.re, &mut self.searcher, self.text, last)?,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn compiled(patterns: &Patterns) -> usize {
        patterns.0.get().map_or(0, |slots| {
            slots.iter().filter(|s| s.pattern.get().is_some()).count()
        })
    }

    #[test]
    fn a_literal_is_compiled_once_per_expression() {
        let expr = crate::Expression::compile(
            "[$contains(x, /a/), $contains(x, /b/i), $match(x, /a/).index]",
        )
        .unwrap();
        let input = serde_json::json!({"x": "xAb"});
        let first = expr
            .evaluate(Some(&input), &crate::Bindings::new())
            .unwrap();
        let patterns = &expr.compiled.patterns;
        assert_eq!(compiled(patterns), 3);
        let shared: Vec<_> = patterns
            .0
            .get()
            .unwrap()
            .iter()
            .map(|s| Arc::as_ptr(s.pattern.get().unwrap()))
            .collect();
        let second = expr
            .evaluate(Some(&input), &crate::Bindings::new())
            .unwrap();
        assert_eq!(first, second);
        let again: Vec<_> = patterns
            .0
            .get()
            .unwrap()
            .iter()
            .map(|s| Arc::as_ptr(s.pattern.get().unwrap()))
            .collect();
        assert_eq!(shared, again);
    }

    #[test]
    fn threads_share_the_compiled_literals() {
        let expr = crate::Expression::compile("$match(x, /\\d+/).match").unwrap();
        let input = serde_json::json!({"x": "a1b22"});
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..50 {
                        let out = expr
                            .evaluate(Some(&input), &crate::Bindings::new())
                            .unwrap();
                        assert_eq!(out, Some(serde_json::json!(["1", "22"])));
                    }
                });
            }
        });
        assert_eq!(compiled(&expr.compiled.patterns), 1);
    }

    #[test]
    fn a_node_of_another_tree_is_not_taken() {
        let a = crate::parse("/a/").unwrap();
        let b = crate::parse("/b/").unwrap();
        assert_eq!(a.root.id, b.root.id);
        let patterns = Patterns::default();
        assert!(patterns.get(&a, &b.root, "b", "").is_none());
        let found = patterns.get(&a, &a.root, "a", "").unwrap().unwrap();
        assert_eq!(found.source, "a");
    }

    #[test]
    fn an_invalid_pattern_fails_each_time() {
        let expr = crate::Expression::compile("$contains('a', /+/)").unwrap();
        for _ in 0..2 {
            let err = expr.evaluate(None, &crate::Bindings::new()).unwrap_err();
            assert_eq!(err.code(), "S0302");
        }
        assert_eq!(compiled(&expr.compiled.patterns), 0);
    }

    #[test]
    fn eval_of_the_same_ids_keeps_its_own_patterns() {
        let expr =
            crate::Expression::compile("/a/('xa').match & $eval('/b/')('xb').match").unwrap();
        for _ in 0..2 {
            let out = expr.evaluate(None, &crate::Bindings::new()).unwrap();
            assert_eq!(out, Some(serde_json::json!("ab")));
        }
    }
}
