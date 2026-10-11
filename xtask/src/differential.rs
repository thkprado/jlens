//! `cargo xtask differential`: generated expressions run in jlens and in jsonata-js 2.2.2,
//! which must agree on the result or on the error code.
//!
//! The generator is seeded, so a run is reproduced by its seed. The cases go to
//! `target/differential/cases.json`; `benches/compare/differential.mjs` runs them in jsonata-js
//! and writes its outcomes next to them; the divergences are written to
//! `target/differential/divergences.json`.
//!
//! Three kinds of disagreement are not divergences, and are counted apart (see "Differential
//! testing" in `docs/src/architecture/conformance.md`): a JavaScript exception inside jsonata-js,
//! which has no error code to compare; different error codes from an expression whose array or
//! object constructors jsonata-js evaluates concurrently; and an evaluation in which jsonata-js
//! changed its input document.

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
    let (mut agreed, mut limits, mut crashed, mut raced, mut mutated) = (0, 0, 0, 0, 0);
    let mut divergences = Vec::new();
    for ((expr, input), expected) in cases.iter().zip(&mut reference) {
        let changed_input = expected
            .as_object_mut()
            .and_then(|o| o.remove("mutated"))
            .is_some();
        let expected = &*expected;
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
        } else if actual["error"].is_string() && expected["error"].is_string() && concurrent(expr) {
            raced += 1;
        } else if changed_input {
            mutated += 1;
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
        "skipped: {limits} (limits), {crashed} (jsonata-js crashed), {raced} (concurrent \
         errors), {mutated} (jsonata-js changed its input)"
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

fn flag(args: &[String], name: &str) -> Result<Option<u64>> {
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
    "$single", "$base64encode", "$encodeUrlComponent", "$assert", "$error",
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
    match rng.below(18) {
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
        14 => match rng.below(4) {
            0 => format!("$map({}, function($e){{ {} }})", e(rng), e(rng)),
            1 => format!("$filter({}, function($e){{ {} }})", e(rng), e(rng)),
            2 => format!("$reduce({}, function($l, $r){{ {} }})", e(rng), e(rng)),
            _ => format!("$sort({}, function($l, $r){{ {} }})", e(rng), e(rng)),
        },
        15 => format!("items#$i[$i < 2].{}", step(rng)),
        16 => format!("items@$o.{}", step(rng)),
        _ => format!("{} ~> |items|{{'p': {}}}|", e(rng), e(rng)),
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
