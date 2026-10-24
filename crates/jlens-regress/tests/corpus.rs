//! jlens-regress: the corpus of V8's answers (`tests/corpus/v8.json`), built by
//! `cargo xtask regex-corpus` from test262 and from regress's own tests.
//!
//! Each case either compiles a pattern, and V8 accepted or rejected it, or runs one `exec`:
//! the pattern, the flags, the input and `lastIndex`, and V8's match as UTF-16 ranges of the
//! match and of each group. The search runs as jlens runs it, with `find_from_interruptible`
//! on a `&str`. A case this API cannot express is counted, not compared:
//!
//! - a pattern or an input holding a lone surrogate, which a `&str` cannot;
//! - a `lastIndex` or a range of V8's answer that falls between the two halves of a surrogate
//!   pair: without the `u` or `v` flag, JavaScript matches UTF-16 code units, and regress
//!   matches the characters of a `&str`.
//!
//! Every other difference fails the test, unless `tests/corpus/differences.txt` lists it with
//! its reason; a listed case that no longer differs fails too, so that the list stays true.

use regress::Regex;
use serde_json::Value;
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

const CORPUS: &str = include_str!("corpus/v8.json");
const DIFFERENCES: &str = include_str!("corpus/differences.txt");

/// Interrupt calls after which a search counts as a hang (each call is 1,024 steps).
const MAX_INTERRUPTS: u32 = 100_000;

/// A UTF-16 range, as V8 gives it.
type Range16 = Option<[usize; 2]>;

/// What V8 answered, or what regress answers, to one case.
#[derive(Debug, PartialEq)]
enum Answer {
    Compiles,
    SyntaxError,
    NoMatch,
    Match {
        ranges: Vec<Range16>,
        groups: BTreeMap<String, Range16>,
    },
    Hangs,
}

/// Why a case is counted instead of compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Skip {
    LoneSurrogate,
    InsideSurrogatePair,
}

fn range16(v: &Value) -> Range16 {
    v.as_array().map(|r| {
        [
            r[0].as_u64().unwrap() as usize,
            r[1].as_u64().unwrap() as usize,
        ]
    })
}

/// V8's answer, read from the case.
fn expected(case: &Value) -> Answer {
    if case.get("compiles").is_some() {
        return Answer::Compiles;
    }
    if case.get("error").is_some() {
        return Answer::SyntaxError;
    }
    match &case["match"] {
        Value::Null => Answer::NoMatch,
        Value::Array(ranges) => Answer::Match {
            ranges: ranges.iter().map(range16).collect(),
            groups: case
                .get("groups")
                .and_then(Value::as_object)
                .map(|g| g.iter().map(|(k, v)| (k.clone(), range16(v))).collect())
                .unwrap_or_default(),
        },
        other => panic!("unexpected match {other}"),
    }
}

/// UTF-16 offsets of every byte offset of `text` that starts a character, and of its end.
fn utf16_offsets(text: &str) -> BTreeMap<usize, usize> {
    let mut offsets = BTreeMap::new();
    let mut u16 = 0;
    for (byte, c) in text.char_indices() {
        offsets.insert(byte, u16);
        u16 += c.len_utf16();
    }
    offsets.insert(text.len(), u16);
    offsets
}

/// Whether a UTF-16 offset falls between the two halves of a surrogate pair of `text`.
fn splits_pair(offsets: &BTreeMap<usize, usize>, at: usize) -> bool {
    let len16 = *offsets.values().last().unwrap();
    at <= len16 && !offsets.values().any(|&o| o == at)
}

/// Runs one case through regress, or says why it cannot.
fn actual(case: &Value) -> Result<Answer, Skip> {
    let Some(pattern) = case["pattern"].as_str() else {
        return Err(Skip::LoneSurrogate);
    };
    let flags = case["flags"].as_str().unwrap();
    let compiled = Regex::with_flags(pattern, flags);
    let input = match case.get("input") {
        None => {
            return Ok(if compiled.is_ok() {
                Answer::Compiles
            } else {
                Answer::SyntaxError
            });
        }
        Some(Value::String(input)) => input.as_str(),
        Some(_) => return Err(Skip::LoneSurrogate),
    };
    let offsets = utf16_offsets(input);
    if let Answer::Match { ranges, groups } = expected(case) {
        let bounds = ranges.iter().chain(groups.values()).flatten().flatten();
        if bounds.copied().any(|at| splits_pair(&offsets, at)) {
            return Err(Skip::InsideSurrogatePair);
        }
    }
    let Ok(re) = compiled else {
        return Ok(Answer::SyntaxError);
    };
    let sticky = flags.contains('y');
    let last_index = case
        .get("lastIndex")
        .map_or(0, |v| v.as_u64().unwrap() as usize);
    let len16 = offsets[&input.len()];
    if last_index > len16 {
        return Ok(Answer::NoMatch);
    }
    let Some((&start, _)) = offsets.iter().find(|&(_, &o)| o == last_index) else {
        return Err(Skip::InsideSurrogatePair);
    };
    let calls = Cell::new(0);
    let found = re.find_from_interruptible(input, start, || {
        calls.set(calls.get() + 1);
        calls.get() > MAX_INTERRUPTS
    });
    let Ok(found) = found else {
        return Ok(Answer::Hangs);
    };
    let Some(m) = found.filter(|m| !sticky || m.start() == start) else {
        return Ok(Answer::NoMatch);
    };
    let to16 = |r: regress::Range| [offsets[&r.start], offsets[&r.end]];
    Ok(Answer::Match {
        ranges: m.groups().map(|r| r.map(to16)).collect(),
        groups: m
            .named_groups()
            .map(|(name, r)| (name.to_string(), r.map(to16)))
            .collect(),
    })
}

/// A case as one line, to name it in `differences.txt` and in failures.
fn key(case: &Value) -> String {
    let mut fields = serde_json::Map::new();
    for field in ["pattern", "flags", "input", "lastIndex"] {
        if let Some(v) = case.get(field) {
            fields.insert(field.to_string(), v.clone());
        }
    }
    Value::Object(fields).to_string()
}

#[test]
fn corpus_matches_v8() {
    let corpus: Value = serde_json::from_str(CORPUS).expect("v8.json is JSON");
    let listed: BTreeSet<&str> = DIFFERENCES
        .lines()
        .filter(|l| l.starts_with('{'))
        .map(|l| l.split("\t#").next().unwrap())
        .collect();
    let cases = corpus["cases"].as_array().unwrap();
    let mut skipped: BTreeMap<Skip, usize> = BTreeMap::new();
    let mut differ = Vec::new();
    let mut still_differ = BTreeSet::new();
    let (mut compiles, mut execs, mut without_uv) = (0, 0, 0);
    for case in cases {
        let flags = case["flags"].as_str().unwrap();
        if !flags.contains(['u', 'v']) {
            without_uv += 1;
        }
        if case.get("input").is_some() {
            execs += 1;
        } else {
            compiles += 1;
        }
        let answer = match actual(case) {
            Ok(answer) => answer,
            Err(skip) => {
                *skipped.entry(skip).or_default() += 1;
                continue;
            }
        };
        let want = expected(case);
        if answer != want {
            let key = key(case);
            if listed.contains(key.as_str()) {
                still_differ.insert(key);
            } else {
                differ.push(format!(
                    "{key}\n    from {}: V8 {want:?}, regress {answer:?}",
                    case["from"].as_str().unwrap()
                ));
            }
        }
    }
    eprintln!(
        "corpus: {} cases ({compiles} compile, {execs} exec), {without_uv} without u or v \
         ({:.0}%); test262 {}; skipped {skipped:?}; {} listed differences",
        cases.len(),
        100.0 * without_uv as f64 / cases.len() as f64,
        corpus["test262"],
        listed.len(),
    );
    let fixed: Vec<_> = listed
        .iter()
        .filter(|k| !still_differ.contains(**k))
        .collect();
    assert!(
        differ.is_empty() && fixed.is_empty(),
        "{} cases differ from V8:\n{}\n\n{} listed cases no longer differ:\n{}",
        differ.len(),
        differ.join("\n"),
        fixed.len(),
        fixed
            .iter()
            .map(|k| k.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    );
}
