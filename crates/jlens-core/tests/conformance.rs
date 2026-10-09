//! Runs the official JSONata test suite (`tests/jsonata-suite`, pinned to jsonata-js 2.2.2).
//!
//! Every case becomes one test named `<group>/<file>` (or `<group>/<file>#<index>` for files
//! holding an array of cases). The semantics mirror `test/run-test-suite.js` from jsonata-js.
//!
//! * Cases listed in `tests/conformance/baseline.txt` must pass; all others are reported as
//!   ignored, so `cargo test` only fails on regressions. Run with `--include-ignored` to try
//!   every case.
//! * `tests/conformance/overrides.json` documents intentional deviations (`skip`,
//!   `alternateResult`, `alternateCode`), each with a reason.
//! * With `JLENS_CONFORMANCE_REPORT=1` the run writes `tests/conformance/REPORT.md` and never
//!   fails; adding `JLENS_UPDATE_BASELINE=1` also rewrites the baseline with the passing cases.
//!   `cargo xtask conformance` wraps both.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jlens_core::{Bindings, Error, Expression, Options};
use libtest_mimic::{Arguments, Failed, Trial};
use serde_json::{Map, Value};

#[derive(Debug, Clone)]
enum Expect {
    Result(Value),
    Undefined,
    /// Expected error fields (`code`, and optionally `token`, `position`, `value`).
    Error(Map<String, Value>),
    /// The case file could not be parsed as JSON by serde_json.
    Unloadable(String),
}

#[derive(Debug, Clone)]
struct Case {
    id: String,
    group: String,
    expr: String,
    input: Option<Value>,
    bindings: Bindings,
    options: Options,
    expect: Expect,
}

#[derive(Debug, Clone, Default)]
struct Override {
    skip: bool,
    alternate_result: Option<Value>,
    alternate_code: Option<String>,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parsing {path:?}: {e}"))
}

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("listing {dir:?}: {e}"))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    entries
}

fn load_cases(suite: &Path) -> Vec<Case> {
    let mut datasets = BTreeMap::new();
    for path in sorted_entries(&suite.join("datasets")) {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        datasets.insert(name, read_json(&path));
    }

    let mut cases = Vec::new();
    for group_dir in sorted_entries(&suite.join("groups")) {
        if !group_dir.is_dir() {
            continue;
        }
        let group = group_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        for file in sorted_entries(&group_dir) {
            if file.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let stem = file.file_stem().unwrap().to_string_lossy().into_owned();
            let text = fs::read_to_string(&file).unwrap();
            let spec = match serde_json::from_str(&text) {
                Ok(spec) => spec,
                Err(e) => {
                    // e.g. lone UTF-16 surrogates such as `\uD800`, which a Rust String cannot hold
                    cases.push(unloadable(format!("{group}/{stem}"), &group, e.to_string()));
                    continue;
                }
            };
            match spec {
                Value::Array(items) => {
                    for (i, item) in items.into_iter().enumerate() {
                        let id = format!("{group}/{stem}#{i}");
                        cases.push(to_case(id, &group, &group_dir, item, &datasets));
                    }
                }
                item => {
                    let id = format!("{group}/{stem}");
                    cases.push(to_case(id, &group, &group_dir, item, &datasets));
                }
            }
        }
    }
    cases
}

fn unloadable(id: String, group: &str, reason: String) -> Case {
    Case {
        id,
        group: group.to_owned(),
        expr: String::new(),
        input: None,
        bindings: Bindings::new(),
        options: Options::default(),
        expect: Expect::Unloadable(reason),
    }
}

fn to_case(
    id: String,
    group: &str,
    group_dir: &Path,
    spec: Value,
    datasets: &BTreeMap<String, Value>,
) -> Case {
    let Value::Object(spec) = spec else {
        panic!("{id}: test case is not an object");
    };

    let expr = match (spec.get("expr"), spec.get("expr-file")) {
        (Some(Value::String(e)), _) => e.clone(),
        (_, Some(Value::String(file))) => fs::read_to_string(group_dir.join(file))
            .unwrap_or_else(|e| panic!("{id}: reading {file}: {e}")),
        _ => panic!("{id}: no expr"),
    };

    let input = if let Some(data) = spec.get("data") {
        Some(data.clone())
    } else {
        match spec.get("dataset") {
            None | Some(Value::Null) => None,
            Some(Value::String(name)) => Some(
                datasets
                    .get(name)
                    .unwrap_or_else(|| panic!("{id}: unknown dataset {name}"))
                    .clone(),
            ),
            Some(other) => panic!("{id}: invalid dataset {other}"),
        }
    };

    let bindings = match spec.get("bindings") {
        Some(Value::Object(map)) => map.clone().into_iter().collect(),
        _ => Bindings::new(),
    };

    let mut options = Options::default();
    if let (Some(limit), Some(depth)) = (spec.get("timelimit"), spec.get("depth")) {
        options.timeout = limit.as_u64().map(Duration::from_millis);
        options.max_depth = depth.as_u64().map(|d| d as usize);
    }

    let expect = if spec.contains_key("undefinedResult") {
        Expect::Undefined
    } else if let Some(result) = spec.get("result") {
        Expect::Result(result.clone())
    } else if let Some(Value::Object(err)) = spec.get("error") {
        Expect::Error(err.clone())
    } else if let Some(code) = spec.get("code") {
        let mut err = Map::new();
        err.insert("code".into(), code.clone());
        if let Some(token) = spec.get("token") {
            err.insert("token".into(), token.clone());
        }
        Expect::Error(err)
    } else {
        panic!("{id}: nothing to test in this test case");
    };

    Case {
        id,
        group: group.to_owned(),
        expr,
        input,
        bindings,
        options,
        expect,
    }
}

fn load_overrides(path: &Path) -> BTreeMap<String, Override> {
    let Value::Object(map) = read_json(path) else {
        panic!("{path:?} must be an object");
    };
    map.into_iter()
        .filter(|(id, _)| !id.starts_with('$'))
        .map(|(id, spec)| {
            assert!(
                spec.get("reason").and_then(Value::as_str).is_some(),
                "override {id} must give a reason"
            );
            let o = Override {
                skip: spec.get("skip").and_then(Value::as_bool).unwrap_or(false),
                alternate_result: spec.get("alternateResult").cloned(),
                alternate_code: spec
                    .get("alternateCode")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            };
            (id, o)
        })
        .collect()
}

fn load_baseline(path: &Path) -> BTreeSet<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// Deep equality with JavaScript number semantics (`1` equals `1.0`).
fn json_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| json_equal(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| json_equal(v, w)))
        }
        _ => a == b,
    }
}

fn check_error(err: &Error, expected: &Map<String, Value>) -> Result<(), Failed> {
    for (field, want) in expected {
        let got = match field.as_str() {
            // The suite runner raises U1001 from its own timeout/depth guard; jlens enforces the
            // same limits through `Options`, which report D1011 (depth) or D1012 (timeout).
            "code" if want == "U1001" && matches!(err.code(), "D1011" | "D1012") => continue,
            "code" => Some(Value::String(err.code().to_owned())),
            "token" => err.token().map(|t| Value::String(t.to_owned())),
            "position" => err.position().map(Value::from),
            "value" => err.value().cloned(),
            _ => continue,
        };
        if !got.as_ref().is_some_and(|g| json_equal(g, want)) {
            return Err(
                format!("error field `{field}`: expected {want}, got {got:?} ({err})").into(),
            );
        }
    }
    Ok(())
}

fn run_case(case: &Case, ov: Option<&Override>) -> Result<(), Failed> {
    if let Expect::Unloadable(reason) = &case.expect {
        return Err(format!("case file cannot be loaded: {reason}").into());
    }
    let mut expect = case.expect.clone();
    if let Some(ov) = ov {
        if let Some(result) = &ov.alternate_result {
            expect = Expect::Result(result.clone());
        }
        if let Some(code) = &ov.alternate_code {
            let mut err = Map::new();
            err.insert("code".into(), Value::String(code.clone()));
            expect = Expect::Error(err);
        }
    }

    let outcome = Expression::compile(&case.expr)
        .and_then(|expr| expr.evaluate_with(case.input.as_ref(), &case.bindings, &case.options));

    match (outcome, &expect) {
        (Ok(None), Expect::Undefined) => Ok(()),
        (Ok(Some(got)), Expect::Result(want)) if json_equal(&got, want) => Ok(()),
        (Err(err), Expect::Error(want)) => check_error(&err, want),
        (Ok(got), _) => Err(format!("expected {expect:?}, got {got:?}").into()),
        (Err(err), _) => Err(format!("expected {expect:?}, got error {err}").into()),
    }
}

fn write_report(path: &Path, cases: &[Case], passed: &BTreeSet<String>) {
    let mut per_group: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for case in cases {
        let entry = per_group.entry(&case.group).or_default();
        entry.1 += 1;
        if passed.contains(&case.id) {
            entry.0 += 1;
        }
    }
    let total = cases.len();
    let ok = passed.len();
    let pct = |p: usize, t: usize| {
        if t == 0 {
            0.0
        } else {
            100.0 * p as f64 / t as f64
        }
    };

    let mut out = String::new();
    writeln!(out, "# JSONata conformance report\n").unwrap();
    writeln!(
        out,
        "Suite: jsonata-js 2.2.2 (`tests/jsonata-suite`). Generated by `cargo xtask conformance`.\n"
    )
    .unwrap();
    writeln!(out, "**Total: {ok} / {total} ({:.1}%)**\n", pct(ok, total)).unwrap();
    writeln!(out, "| Group | Passed | Total | % |\n|---|---:|---:|---:|").unwrap();
    for (group, (p, t)) in per_group {
        writeln!(out, "| {group} | {p} | {t} | {:.1} |", pct(p, t)).unwrap();
    }
    fs::write(path, out).unwrap();
    println!(
        "conformance: {ok} / {total} ({:.1}%) — report at {}",
        pct(ok, total),
        path.display()
    );
}

fn main() {
    let root = workspace_root();
    let suite = root.join("tests/jsonata-suite");
    let conformance = root.join("tests/conformance");

    let cases = load_cases(&suite);
    let overrides = Arc::new(load_overrides(&conformance.join("overrides.json")));
    let baseline = load_baseline(&conformance.join("baseline.txt"));
    let report_mode = std::env::var_os("JLENS_CONFORMANCE_REPORT").is_some();
    let passed = Arc::new(Mutex::new(BTreeSet::new()));

    let trials = cases
        .iter()
        .map(|case| {
            let ov = overrides.get(&case.id).cloned();
            let skip = ov.as_ref().is_some_and(|o| o.skip);
            let expected_to_pass = baseline.contains(&case.id);
            let (id, group) = (case.id.clone(), case.group.clone());
            let case = case.clone();
            let passed = Arc::clone(&passed);
            Trial::test(id, move || {
                if skip {
                    return Ok(());
                }
                let result = run_case(&case, ov.as_ref());
                if result.is_ok() {
                    passed.lock().unwrap().insert(case.id.clone());
                }
                result
            })
            .with_kind(group)
            .with_ignored_flag(skip || (!expected_to_pass && !report_mode))
        })
        .collect();

    let mut args = Arguments::from_args();
    if report_mode {
        args.include_ignored = true;
        args.quiet = true;
    }
    let conclusion = libtest_mimic::run(&args, trials);

    if report_mode {
        let passed = passed.lock().unwrap();
        write_report(&conformance.join("REPORT.md"), &cases, &passed);
        if std::env::var_os("JLENS_UPDATE_BASELINE").is_some() {
            let mut text = String::from(
                "# Conformance cases that must keep passing. Regenerate with\n\
                 # `cargo xtask conformance --update-baseline`.\n",
            );
            for id in passed.iter() {
                text.push_str(id);
                text.push('\n');
            }
            fs::write(conformance.join("baseline.txt"), text).unwrap();
        }
        return;
    }
    conclusion.exit();
}
