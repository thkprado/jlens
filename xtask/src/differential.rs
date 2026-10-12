//! `cargo xtask differential`: generated expressions run in jlens and in jsonata-js 2.2.2,
//! which must agree on the result or on the error code.
//!
//! The generator is seeded, so a run is reproduced by its seed. The cases go to
//! `target/differential/cases.json`; `benches/compare/differential.mjs` runs them in jsonata-js
//! and writes its outcomes next to them; the divergences are written to
//! `target/differential/divergences.json`.
//!
//! Ten kinds of disagreement are not divergences, and are counted apart (see "Differential
//! testing" in `docs/src/architecture/conformance.md`): a JavaScript exception inside jsonata-js,
//! which has no error code to compare; a case jsonata-js never finished; a jsonata-js result
//! with half of a character, which a jlens string cannot hold; an expression with a
//! regular expression literal on the left of `??` or `?:`, which the jsonata-js parser mangles;
//! different error codes from an expression whose array or object constructors jsonata-js
//! evaluates concurrently; an evaluation in which jsonata-js changed its input document; a
//! wildcard that gives jsonata-js's internal flag `sequence` as a `true` item; an expression
//! that may meet one of the jsonata-js quirks that jlens dropped; a number that overflows to
//! infinity, where jlens fails and jsonata-js goes on; and a large number whose digits jlens
//! writes exactly.

use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use jlens_core::{Bindings, Expression, Node, NodeKind, Options};
use serde_json::{Value as JsonValue, json};

use crate::root;

/// Limits of an evaluation, the same in both engines.
const TIMEOUT_MS: u64 = 1000;
const MAX_DEPTH: usize = 500;

/// Error codes of the limits above (jsonata-js: depth, time; jlens: its own guardrails). A case
/// that hits one in either engine is skipped: the engines count depth differently.
const LIMIT_CODES: &[&str] = &["D1011", "D1012", "U1001"];

/// How many divergences are printed; all of them are written to the file.
const PRINTED: usize = 10;

pub fn run(args: &[String]) -> Result<()> {
    let count = flag(args, "--cases")?.unwrap_or(1000);
    let seed = match flag(args, "--seed")? {
        Some(seed) => seed,
        None => SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() as u64,
    };
    let compare = root().join("benches/compare");
    if !compare.join("node_modules/jsonata").is_dir() {
        bail!("jsonata-js is not installed: run `npm install` in benches/compare");
    }

    let mut rng = Rng(seed);
    let inputs = inputs();
    let cases: Vec<(String, Option<usize>)> = (0..count)
        .map(|_| {
            let expr = expression(&mut rng, 4);
            let input = rng.below(inputs.len() + 1);
            (expr, (input < inputs.len()).then_some(input))
        })
        .collect();

    let dir = root().join("target/differential");
    fs::create_dir_all(&dir)?;
    // A file left by an earlier run would be mistaken for this run's.
    let _ = fs::remove_file(dir.join("divergences.json"));
    let cases_path = dir.join("cases.json");
    let outcomes_path = dir.join("outcomes.json");
    let file = json!({
        "seed": seed,
        "inputs": inputs,
        "bindings": variables(),
        "timeout": TIMEOUT_MS,
        "depth": MAX_DEPTH,
        "cases": cases
            .iter()
            .map(|(expr, input)| json!({"expr": expr, "input": input}))
            .collect::<Vec<_>>(),
    });
    fs::write(&cases_path, serde_json::to_string_pretty(&file)?)?;

    let status = Command::new("node")
        .current_dir(&compare)
        .arg("differential.mjs")
        .arg(&cases_path)
        .arg(&outcomes_path)
        .status()
        .context("running node (Node.js 20+ is needed)")?;
    if !status.success() {
        bail!("the jsonata-js runner failed: {status}");
    }
    let mut reference: Vec<JsonValue> = serde_json::from_str(&fs::read_to_string(&outcomes_path)?)?;
    if reference.len() != cases.len() {
        bail!(
            "the jsonata-js runner wrote {} outcomes for {} cases",
            reference.len(),
            cases.len()
        );
    }

    let mut bindings = Bindings::new();
    for (name, value) in variables().as_object().into_iter().flatten() {
        bindings = bindings.var(name.clone(), value.clone());
    }
    let options = Options::new()
        .timeout(std::time::Duration::from_millis(TIMEOUT_MS))
        .max_depth(MAX_DEPTH);
    let (mut agreed, mut limits, mut crashed, mut hung, mut split) = (0, 0, 0, 0, 0);
    let (mut mangled, mut raced, mut mutated, mut flagged, mut dropped) = (0, 0, 0, 0, 0);
    let (mut overflowed, mut exact, mut timezone) = (0, 0, 0);
    let mut divergences = Vec::new();
    for ((expr, input), expected) in cases.iter().zip(&mut reference) {
        let changed_input = expected
            .as_object_mut()
            .and_then(|o| o.remove("mutated"))
            .is_some();
        let expected = &*expected;
        if expected["hung"] == true {
            hung += 1;
            continue;
        }
        if expected["split"] == true {
            split += 1;
            continue;
        }
        let actual = outcome(expr, input.map(|i| &inputs[i]), &bindings, &options);
        if [&actual, expected].iter().any(|o| is_limit(o)) {
            limits += 1;
        } else if expected["error"]
            .as_str()
            .is_some_and(|e| e.starts_with("js:"))
        {
            crashed += 1;
        } else if same(&actual, expected) {
            agreed += 1;
        } else if regex_before_default(expr) {
            mangled += 1;
        } else if actual["error"].is_string() && expected["error"].is_string() && concurrent(expr) {
            raced += 1;
        } else if changed_input {
            mutated += 1;
        } else if sequence_flag(expr, input.is_none(), &actual, expected) {
            flagged += 1;
        } else if dropped_quirk(expr) {
            dropped += 1;
        } else if exact_digits(expr, &actual, expected) {
            exact += 1;
        } else if actual["error"] == "D1001" {
            // jlens raises D1001 where a number overflows to infinity (`1 / 0`), which
            // jsonata-js passes on: to its result, where JSON writes `null`, or to an operator
            // that raises D1001 or another error later. `$fromMillis` raises it for a date
            // beyond JavaScript's range, which jsonata-js writes as "NaN".
            overflowed += 1;
        } else if actual["error"] == "D3150" {
            // jlens raises D3150 where `$fromMillis` or `$now` is given a timezone that is not a
            // number, which jsonata-js reads as NaN and writes as "NaN" in every component.
            timezone += 1;
        } else {
            divergences.push(json!({
                "expr": expr,
                "input": input,
                "jlens": actual,
                "jsonata-js": expected,
            }));
        }
    }

    for d in divergences.iter().take(PRINTED) {
        println!("{}", d["expr"].as_str().unwrap_or_default());
        println!("  input:      {}", d["input"]);
        println!("  jlens:      {}", d["jlens"]);
        println!("  jsonata-js: {}\n", d["jsonata-js"]);
    }
    println!(
        "{count} cases (seed {seed}): {agreed} agree, {} differ",
        divergences.len()
    );
    println!(
        "skipped: {limits} (limits), {crashed} (jsonata-js crashed), {hung} (jsonata-js hung), \
         {split} (jsonata-js split a character), {mangled} (regex left of ?? or ?:), \
         {raced} (concurrent errors), \
         {mutated} (jsonata-js changed its input), {flagged} (wildcard over a sequence), \
         {dropped} (dropped quirk), {overflowed} (number out of range), \
         {exact} (exact digits of a large number), {timezone} (timezone not a number)"
    );
    if !divergences.is_empty() {
        let path = dir.join("divergences.json");
        fs::write(&path, serde_json::to_string_pretty(&divergences)?)?;
        bail!(
            "jlens and jsonata-js differ; all divergences are in {}",
            path.display()
        );
    }
    Ok(())
}

pub(crate) fn flag(args: &[String], name: &str) -> Result<Option<u64>> {
    match args.iter().position(|a| a == name) {
        None => Ok(None),
        Some(i) => {
            let value = args
                .get(i + 1)
                .with_context(|| format!("{name} needs a value"))?;
            let n = value
                .parse()
                .with_context(|| format!("{name} takes a non-negative integer, not {value:?}"))?;
            Ok(Some(n))
        }
    }
}

/// The outcome of a case in jlens, in the form the jsonata-js runner writes.
fn outcome(
    expr: &str,
    input: Option<&JsonValue>,
    bindings: &Bindings,
    options: &Options,
) -> JsonValue {
    match Expression::compile(expr).and_then(|e| e.evaluate_with(input, bindings, options)) {
        Ok(Some(value)) => json!({ "value": value }),
        Ok(None) => json!({ "undefined": true }),
        Err(err) => json!({ "error": err.code() }),
    }
}

fn is_limit(outcome: &JsonValue) -> bool {
    outcome["error"]
        .as_str()
        .is_some_and(|code| LIMIT_CODES.contains(&code))
}

/// Whether `expr` has a regular expression literal on the left of `??` or `?:`. The jsonata-js
/// parser copies that side through JSON, which turns the regex into `{}`: the expression then
/// hangs (counted as such) or gives another result.
fn regex_before_default(expr: &str) -> bool {
    let Ok(compiled) = Expression::compile(expr) else {
        return false;
    };
    let mut found = false;
    compiled.ast().root.walk(&mut |node: &Node| {
        if let NodeKind::Fallback { lhs, .. } = &node.kind {
            lhs.walk(&mut |n: &Node| found |= matches!(n.kind, NodeKind::Regex { .. }));
        }
    });
    found
}

/// Whether jsonata-js evaluates parts of `expr` concurrently: the items of an array constructor
/// and the values of an object constructor (`Promise.all`). When two of them fail, the error it
/// reports is the one raised first in its asynchronous schedule, not necessarily the first in
/// order, which jlens reports.
fn concurrent(expr: &str) -> bool {
    let Ok(compiled) = Expression::compile(expr) else {
        return false;
    };
    let mut found = false;
    compiled.ast().root.walk(&mut |node: &Node| {
        found |= match &node.kind {
            NodeKind::Array { items } => items.len() > 1,
            // One pair makes several groups when the input is an array.
            NodeKind::Object { .. } => true,
            _ => node.attrs.group.is_some(),
        };
    });
    found
}

/// Whether jsonata-js's result is jlens's with `true` items added, and `expr` has a wildcard. An
/// array that jsonata-js builds from a path carries the own property `sequence: true`, and its
/// wildcard enumerates the properties of an array item, flag included
/// (`$map([1, 2], function($e){ items.$ }).*`).
///
/// Without an input, the flag need not reach the result: a path whose first step is an array
/// constructor evaluates it on the sequence `[undefined]`, so a wildcard in it gives
/// `[undefined, true]`, and the path goes on where jlens's stops at `[]` (`[*].price` is
/// `undefined` in jsonata-js, `[]` in jlens). Such a case counts too, whatever the outcomes.
fn sequence_flag(expr: &str, no_input: bool, actual: &JsonValue, expected: &JsonValue) -> bool {
    if no_input && wildcard_first(expr) {
        return true;
    }
    let (Some(actual), Some(expected)) = (actual["value"].as_array(), expected["value"].as_array())
    else {
        return false;
    };
    let unflagged: Vec<JsonValue> = expected.iter().filter(|v| **v != true).cloned().collect();
    if unflagged.len() == expected.len()
        || !same(&JsonValue::Array(unflagged), &actual.clone().into())
    {
        return false;
    }
    let Ok(compiled) = Expression::compile(expr) else {
        return false;
    };
    let mut found = false;
    compiled
        .ast()
        .root
        .walk(&mut |node: &Node| found |= matches!(node.kind, NodeKind::Wildcard));
    found
}

/// Whether `expr` has a path whose first step is an array constructor with a wildcard in it.
fn wildcard_first(expr: &str) -> bool {
    let Ok(compiled) = Expression::compile(expr) else {
        return false;
    };
    let mut found = false;
    compiled.ast().root.walk(&mut |node: &Node| {
        if let NodeKind::Path { steps, .. } = &node.kind
            && steps.first().is_some_and(|s| s.attrs.cons_array)
        {
            steps[0].walk(&mut |n: &Node| found |= matches!(n.kind, NodeKind::Wildcard));
        }
    });
    found
}

/// Whether `expr` may meet a jsonata-js quirk that jlens dropped (see "Differences" in
/// `docs/src/language/compatibility.md`): a regex literal applied with `~>` to anything but a
/// string literal, which jsonata-js matches as JavaScript's `String(value)`; `$formatNumber` with
/// a literal picture whose integer part has irregular grouping (`'#,##,##0'`), where jsonata-js
/// places separators with no digit on their left; and `$substringBefore` or `$substringAfter`
/// whose `chars` is not a string literal, which jsonata-js searches as "undefined" when missing.
fn dropped_quirk(expr: &str) -> bool {
    let Ok(compiled) = Expression::compile(expr) else {
        return false;
    };
    let mut found = false;
    compiled.ast().root.walk(&mut |node: &Node| {
        found |= match &node.kind {
            NodeKind::Apply { lhs, rhs } => {
                matches!(rhs.kind, NodeKind::Regex { .. })
                    && !matches!(lhs.kind, NodeKind::String { .. })
            }
            NodeKind::Function { procedure, args } => match &procedure.kind {
                NodeKind::Variable { name } if name == "formatNumber" => args.get(1).is_some_and(
                    |a| matches!(&a.kind, NodeKind::String { value } if irregular_grouping(value)),
                ),
                NodeKind::Variable { name }
                    if name == "substringBefore" || name == "substringAfter" =>
                {
                    !args
                        .get(1)
                        .is_some_and(|a| matches!(a.kind, NodeKind::String { .. }))
                }
                _ => false,
            },
            _ => false,
        };
    });
    found
}

/// Whether jlens wrote the digits of a large number where jsonata-js did not. `$formatNumber`
/// from 1e21 writes JavaScript's exponent notation through the picture, so jsonata-js's result
/// has an `e` and a `+` with at most a separator between them (`1e,+21.00`). `$formatBase` from
/// 2^53 writes zeros below the number's precision in a radix that is not a power of two, so the
/// results differ only where jsonata-js has a zero.
fn exact_digits(expr: &str, actual: &JsonValue, expected: &JsonValue) -> bool {
    let (jlens, js) = (actual.to_string(), expected.to_string());
    let calls = |name: &str| expr.contains(&format!("${name}("));
    let exponent = js
        .as_bytes()
        .windows(3)
        .any(|w| w[0] == b'e' && (w[1] == b'+' || w[2] == b'+'));
    let zeros = jlens.len() == js.len()
        && jlens
            .bytes()
            .zip(js.bytes())
            .all(|(a, b)| a == b || b == b'0');
    (calls("formatNumber") && exponent) || (calls("formatBase") && zeros)
}

/// Whether the integer part of a sub-picture of `picture` has groups of different sizes after
/// its first separator, as `#,##,##0` has.
fn irregular_grouping(picture: &str) -> bool {
    picture.split(';').any(|sub| {
        let integer = sub.split('.').next().unwrap_or_default();
        let sizes: Vec<usize> = integer.split(',').skip(1).map(str::len).collect();
        sizes.windows(2).any(|w| w[0] != w[1])
    })
}

/// JSON equality, where numbers are compared as floating-point values (jsonata-js writes `1`
/// for the `1.0` jlens may hold) and the keys of an object must come in the same order.
fn same(a: &JsonValue, b: &JsonValue) -> bool {
    match (a, b) {
        (JsonValue::Number(x), JsonValue::Number(y)) => x.as_f64() == y.as_f64(),
        (JsonValue::Array(x), JsonValue::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same(x, y))
        }
        (JsonValue::Object(x), JsonValue::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y)
                    .all(|((kx, vx), (ky, vy))| kx == ky && same(vx, vy))
        }
        _ => a == b,
    }
}

/// The input documents; a case uses one of them, or none.
fn inputs() -> Vec<JsonValue> {
    vec![
        json!({
            "a": {"b": [1, 2, {"b": "x"}]},
            "items": [{"price": 2, "a": "p"}, {"price": 3.5, "a": null}, {"price": "4"}],
            "s": "hello 𝄞",
            "n": 7
        }),
        json!([1, 2.5, "3", null, true, {"a": 1}, [4, [5]]]),
        json!("text"),
        json!({"a": [{"b": 1}, {"b": [2, 3]}], "price": 10, "items": []}),
    ]
}

fn variables() -> JsonValue {
    json!({"x": [1, "two", null], "n": 0.1})
}

#[rustfmt::skip]
const LEAVES: &[&str] = &[
    "a", "b", "items", "price", "s", "n", "$", "$$", "$x", "$n", "1", "0", "3", "-2.5", "0.1",
    "1e3", "'s'", "''", "'12'", "true", "false", "null", "[]", "{}", "items.price", "a.b",
    "$x[1]", "$[0]", "*", "**",
];

const STEPS: &[&str] = &["a", "b", "items", "price", "s", "n", "$", "*", "$string()"];

#[rustfmt::skip]
const OPERATORS: &[&str] = &[
    "+", "-", "*", "/", "%", "=", "!=", "<", "<=", ">", ">=", "and", "or", "in", "&", "..", "??",
    "?:",
];

/// Built-in functions whose results do not depend on the clock or on chance.
#[rustfmt::skip]
const FUNCTIONS: &[&str] = &[
    "$string", "$number", "$sum", "$count", "$max", "$min", "$average", "$keys", "$values",
    "$reverse", "$sort", "$distinct", "$uppercase", "$lowercase", "$trim", "$pad", "$length",
    "$boolean", "$not", "$exists", "$type", "$abs", "$floor", "$ceil", "$round", "$power",
    "$sqrt", "$append", "$spread", "$merge", "$lookup", "$substring", "$substringBefore",
    "$substringAfter", "$split", "$join", "$contains", "$replace", "$formatNumber",
    "$formatBase", "$formatInteger", "$parseInteger", "$fromMillis", "$toMillis", "$zip",
    "$single", "$base64encode", "$encodeUrlComponent", "$assert", "$error", "$match",
    "$base64decode", "$encodeUrl", "$decodeUrl", "$decodeUrlComponent",
];

/// Strings for the functions that read text: matched by the patterns below, or decoded.
#[rustfmt::skip]
const TEXTS: &[&str] = &[
    "s", "'banana'", "'Hello World'", "'a1 b22 c333'", "'aabbcc'", "'tic-tac-toe'",
    r"'line1\nline2'", "''", "'%E2%82%AC%20x'", "'%zz'", "'aGVsbG8='", "'4pyTIOKCrA=='", "'x y'",
];

/// Regular expression literals, none of them catastrophic. `/x?/` matches the empty string,
/// which `$replace` and `$split` treat apart.
#[rustfmt::skip]
const REGEXES: &[&str] = &[
    "/l+/", "/o/i", "/[aeiou]/", r"/(\w)(\w)/", r"/\d+/", "/^h/im", r"/\s/", r"/(.)\1/",
    "/[^a-z ]/", "/b(an)*a/", "/x?/", "/𝄞/", "/e|o/", r"/\bt/", "/a.c/", "/^line/m",
];

/// Replacements for `$replace`: group references, a literal `$`, a group that does not exist.
#[rustfmt::skip]
const REPLACEMENTS: &[&str] = &[
    "'$0'", "'<$1>'", "'$$'", "'$2$1'", "'x'", "''", "'$9'", "'$12'",
    "function($m){ $uppercase($m.match) }", "function($m){ $m.groups[0] }",
    "function($m){ $string($m.index) }",
];

#[rustfmt::skip]
const NUMBERS: &[&str] = &[
    "0", "7", "21", "-42", "3.7", "1999", "1234.5678", "-0.5", "0.000123", "12345678", "1e21",
];

#[rustfmt::skip]
const NUMBER_PICTURES: &[&str] = &[
    "'#,##0.00'", "'0.###'", "'0.0e0'", "'#,##0.0#;(#,##0.0#)'", "'00%'", "'#‰'", "'#,###'",
    "'000.000'", "'#0.0##'", "'#,##,##0'", "'#'", "'.00'",
];

#[rustfmt::skip]
const INTEGER_PICTURES: &[&str] = &[
    "'0'", "'000'", "'#,##0'", "'w'", "'W'", "'Ww'", "'I'", "'i'", "'a'", "'A'", "'1;o'",
    "'w;o'", "'#0'", "'01'",
];

/// Milliseconds since the epoch: the epoch, a leap day, before 1970, the last year 9999 moment.
#[rustfmt::skip]
const MILLIS: &[&str] = &[
    "0", "1700000000000", "951782400000", "-86400000", "1e12", "253402300799999", "1234",
];

#[rustfmt::skip]
const DATE_PICTURES: &[&str] = &[
    "'[Y0001]-[M01]-[D01]'", "'[H01]:[m01]:[s01].[f001]'", "'[FNn], [D1o] [MNn] [Y]'",
    "'[Y,2]-[M,2]'", "'[h]:[m01] [P]'", "'[Z01:01]'", "'[z]'", "'[d] [F1] [W] [w]'",
    "'[X0001]-W[W01]-[F1]'", "'[MNn,3-3] [Dwo] [YI]'", "'[D01]/[M01]/[Y0001] [H01]:[m01]'",
    "'[Y0001][M01][D01]'",
];

/// The pictures above that give the date: `$toMillis` takes the parts a picture leaves out from
/// the clock.
#[rustfmt::skip]
const FULL_DATE_PICTURES: &[&str] = &[
    "'[Y0001]-[M01]-[D01]'", "'[FNn], [D1o] [MNn] [Y]'", "'[Y,2]-[M,2]'",
    "'[X0001]-W[W01]-[F1]'", "'[MNn,3-3] [Dwo] [YI]'", "'[D01]/[M01]/[Y0001] [H01]:[m01]'",
    "'[Y0001][M01][D01]'",
];

#[rustfmt::skip]
const TIMEZONES: &[&str] = &["'+0530'", "'-0800'", "'0000'", "'+01:00'"];

#[rustfmt::skip]
const DATES: &[&str] = &[
    "'2024-02-29'", "'2024-02-29T12:34:56Z'", "'2024-02-29T12:34:56.789+05:30'", "'1999'",
    "'2024-13-01'", "'not a date'",
];

/// An expression from a small grammar, at most `depth` levels deep.
fn expression(rng: &mut Rng, depth: u32) -> String {
    if depth == 0 || rng.below(4) == 0 {
        return rng.pick(LEAVES).to_owned();
    }
    let e = |rng: &mut Rng| expression(rng, depth - 1);
    let list = |rng: &mut Rng| {
        let items: Vec<String> = (0..rng.below(4)).map(|_| e(rng)).collect();
        items.join(", ")
    };
    // A path step: a literal there is a static error (`S0213`), so a name or a block.
    let step = |rng: &mut Rng| match rng.below(2) {
        0 => rng.pick(STEPS).to_owned(),
        _ => format!("({})", e(rng)),
    };
    match rng.below(21) {
        0..=2 => {
            let l = e(rng);
            let op = rng.pick(OPERATORS);
            format!("({l} {op} {})", e(rng))
        }
        3 => format!("{}[{}]", e(rng), e(rng)),
        4 => format!("{}.{}", e(rng), step(rng)),
        5 | 6 => {
            let f = rng.pick(FUNCTIONS);
            format!("{f}({})", list(rng))
        }
        7 => format!("({} ? {} : {})", e(rng), e(rng), e(rng)),
        8 => format!("[{}]", list(rng)),
        9 => format!("{{{}: {}}}", e(rng), e(rng)),
        10 => format!("($y := {}; {})", e(rng), e(rng)),
        11 => format!("($f := function($y){{ {} }}; $f({}))", e(rng), e(rng)),
        12 => format!("{}^({})", e(rng), e(rng)),
        13 => format!("{}{{{}: {}}}", e(rng), e(rng), e(rng)),
        14 => match rng.below(6) {
            0 => format!("$map({}, function($e){{ {} }})", e(rng), e(rng)),
            1 => format!("$filter({}, function($e){{ {} }})", e(rng), e(rng)),
            2 => format!("$reduce({}, function($l, $r){{ {} }})", e(rng), e(rng)),
            3 => format!("$sort({}, function($l, $r){{ {} }})", e(rng), e(rng)),
            // An object's values and keys: a body that reads them, or any expression.
            f => {
                let f = if f == 4 { "$sift" } else { "$each" };
                let body = match rng.below(5) {
                    0 => "$v".to_owned(),
                    1 => "$k".to_owned(),
                    2 => "$k & '=' & $string($v)".to_owned(),
                    3 => "$type($v) = 'number'".to_owned(),
                    _ => e(rng),
                };
                format!("{f}({}, function($v, $k){{ {body} }})", e(rng))
            }
        },
        15 => format!("items#$i[$i < 2].{}", step(rng)),
        16 => format!("items@$o.{}", step(rng)),
        17 => format!("{} ~> |items|{{'p': {}}}|", e(rng), e(rng)),
        18 => regex_call(rng, depth),
        19 => picture_call(rng, depth),
        _ => codec_call(rng, depth),
    }
}

/// One of the given strings, or now and then any expression.
fn either(rng: &mut Rng, items: &[&'static str], depth: u32) -> String {
    if rng.below(5) == 0 {
        expression(rng, depth - 1)
    } else {
        rng.pick(items).to_owned()
    }
}

/// A function that takes a regular expression, applied to a text.
fn regex_call(rng: &mut Rng, depth: u32) -> String {
    let text = either(rng, TEXTS, depth);
    let regex = rng.pick(REGEXES);
    let limit = |rng: &mut Rng| match rng.below(3) {
        0 => String::new(),
        _ => format!(", {}", either(rng, &["0", "1", "2", "-1"], depth)),
    };
    match rng.below(6) {
        0 => format!("$match({text}, {regex}{})", limit(rng)),
        1 => {
            let replacement = either(rng, REPLACEMENTS, depth);
            format!("$replace({text}, {regex}, {replacement}{})", limit(rng))
        }
        2 => format!("$split({text}, {regex}{})", limit(rng)),
        3 => format!("$contains({text}, {regex})"),
        // A regex is a function: applied, it gives the first match.
        4 => format!("({text} ~> {regex})"),
        _ => format!("$filter($split({text}, ' '), function($e){{ $contains($e, {regex}) }})"),
    }
}

/// A formatting function with a valid picture, or a parser given what its formatter wrote.
fn picture_call(rng: &mut Rng, depth: u32) -> String {
    match rng.below(7) {
        0 => {
            let number = either(rng, NUMBERS, depth);
            let picture = rng.pick(NUMBER_PICTURES);
            let options = match rng.below(4) {
                0 => ", {'decimal-separator': ',', 'grouping-separator': '.'}",
                1 => ", {'percent': 'pc', 'zero-digit': '٠'}",
                _ => "",
            };
            format!("$formatNumber({number}, {picture}{options})")
        }
        1 => {
            let number = either(rng, NUMBERS, depth);
            format!("$formatInteger({number}, {})", rng.pick(INTEGER_PICTURES))
        }
        2 => {
            let number = either(rng, NUMBERS, depth);
            let picture = rng.pick(INTEGER_PICTURES);
            format!("$parseInteger($formatInteger({number}, {picture}), {picture})")
        }
        3 | 4 => {
            let millis = either(rng, MILLIS, depth);
            let picture = rng.pick(DATE_PICTURES);
            let timezone = match rng.below(3) {
                0 => format!(", {}", rng.pick(TIMEZONES)),
                _ => String::new(),
            };
            format!("$fromMillis({millis}, {picture}{timezone})")
        }
        5 => {
            let millis = either(rng, MILLIS, depth);
            let picture = rng.pick(FULL_DATE_PICTURES);
            format!("$toMillis($fromMillis({millis}, {picture}), {picture})")
        }
        _ => match rng.below(2) {
            0 => format!("$toMillis({})", either(rng, DATES, depth)),
            _ => format!("$fromMillis({})", either(rng, MILLIS, depth)),
        },
    }
}

/// A decoder, given a text or what its encoder wrote.
fn codec_call(rng: &mut Rng, depth: u32) -> String {
    let (encode, decode) = match rng.below(3) {
        0 => ("$base64encode", "$base64decode"),
        1 => ("$encodeUrl", "$decodeUrl"),
        _ => ("$encodeUrlComponent", "$decodeUrlComponent"),
    };
    let text = either(rng, TEXTS, depth);
    match rng.below(3) {
        0 => format!("{decode}({encode}({text}))"),
        1 => format!("{decode}({text})"),
        _ => format!("{encode}({text})"),
    }
}

/// SplitMix64: small, fast, and the same on every platform.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick(&mut self, items: &[&'static str]) -> &'static str {
        items[self.below(items.len())]
    }
}
