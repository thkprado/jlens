//! `jlens test`: runs files of test cases.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;
use jlens_core::{Bindings, Expression, Options};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

use crate::{eval, output, schema};

#[derive(Args)]
pub struct TestArgs {
    /// Case files: several, or glob patterns such as `tests/*.json`.
    #[arg(required = true, value_name = "FILE")]
    files: Vec<String>,
    /// Print the outcome as one compact JSON object on stdout:
    /// {"passed":N,"failed":N,"failures":[…]}.
    #[arg(long)]
    json: bool,
    /// Stop an evaluation after this many milliseconds (error D1012).
    #[arg(long, value_name = "MS")]
    timeout: Option<u64>,
    #[command(flatten)]
    caps: eval::SizeCaps,
}

/// A case file: one expression and the cases it is evaluated with.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseFile {
    /// The expression.
    expr: Option<String>,
    /// Or the file that holds it, relative to the case file.
    file: Option<PathBuf>,
    /// Variables bound in every case.
    #[serde(default)]
    bindings: Map<String, Value>,
    /// The time seen by `$now()` and `$millis()`, as in `eval --now`.
    now: Option<Value>,
    /// The seed of `$random()` and `$shuffle()`.
    seed: Option<u64>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    name: Option<String>,
    /// The input document; absent for none (`null` is a document).
    #[serde(default, deserialize_with = "present")]
    input: Option<Value>,
    /// Variables bound in this case, over those of the file.
    #[serde(default)]
    bindings: Map<String, Value>,
    #[serde(default, deserialize_with = "present")]
    result: Option<Value>,
    #[serde(default)]
    undefined: bool,
    /// The code of the expected error.
    error: Option<String>,
}

/// Deserializes a field that may be `null` as `Some(Value::Null)`, so that only an absent field
/// is `None`.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// What a case expects.
enum Expected {
    Result(Value),
    Undefined,
    Error(String),
}

/// A case file, read and validated.
struct Suite {
    path: PathBuf,
    source: String,
    options: Options,
    cases: Vec<(Option<String>, Option<Value>, Bindings, Expected)>,
}

pub fn run(args: TestArgs) -> Result<ExitCode> {
    // Read every file first, so that an invalid one prints nothing on stdout.
    let suites = schema::expand(&args.files)?
        .into_iter()
        .map(|path| load(path, &args))
        .collect::<Result<Vec<_>>>()?;
    let (mut passed, mut failed) = (0, 0);
    let mut failures = Vec::new();
    for suite in &suites {
        let name = suite.path.display().to_string();
        let expr = Expression::compile(&suite.source);
        let (mut file_passed, mut file_failed) = (0, 0);
        for (index, (case_name, input, bindings, expected)) in suite.cases.iter().enumerate() {
            let actual = match &expr {
                Ok(expr) => expr.evaluate_with(input.as_ref(), bindings, &suite.options),
                Err(err) => Err(err.clone()),
            };
            if matches(&actual, expected) {
                file_passed += 1;
                continue;
            }
            file_failed += 1;
            let case = index + 1;
            if args.json {
                failures.push(failure_json(
                    &name,
                    case,
                    case_name.as_deref(),
                    expected,
                    &actual,
                    &suite.source,
                ));
                continue;
            }
            match case_name {
                Some(case_name) => eprintln!("FAIL {name} case {case}: {case_name}"),
                None => eprintln!("FAIL {name} case {case}"),
            }
            eprintln!("  expected: {}", expected_text(expected));
            eprintln!("  actual:   {}", actual_text(&actual));
        }
        passed += file_passed;
        failed += file_failed;
        if !args.json {
            eprintln!("{name}: {file_passed} passed, {file_failed} failed");
        }
    }
    if args.json {
        output::line(&format!(
            r#"{{"passed":{passed},"failed":{failed},"failures":[{}]}}"#,
            failures.join(",")
        ))?;
    } else if suites.len() > 1 {
        eprintln!("{} file(s): {passed} passed, {failed} failed", suites.len());
    }
    Ok(if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// Reads a case file, its expression, and checks that every case expects one outcome.
fn load(path: PathBuf, args: &TestArgs) -> Result<Suite> {
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let file: CaseFile =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let source = match (file.expr, &file.file) {
        (Some(expr), None) => expr,
        (None, Some(expr_file)) => {
            let expr_path = path.parent().unwrap_or(Path::new("")).join(expr_file);
            fs::read_to_string(&expr_path)
                .with_context(|| format!("reading {}", expr_path.display()))?
        }
        _ => bail!("{}: give either expr or file", path.display()),
    };
    let mut options = Options::new();
    options.timeout = args.timeout.map(Duration::from_millis);
    args.caps.apply(&mut options);
    options.seed = file.seed;
    options.now = match &file.now {
        None => None,
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => eval::parse_now(s).ok(),
        Some(_) => None,
    };
    if file.now.is_some() && options.now.is_none() {
        bail!(
            "{}: now must be milliseconds since the epoch or an ISO 8601 timestamp",
            path.display()
        );
    }
    let mut cases = Vec::new();
    for (index, case) in file.cases.into_iter().enumerate() {
        let expected = match (case.result, case.undefined, case.error) {
            (Some(value), false, None) => Expected::Result(value),
            (None, true, None) => Expected::Undefined,
            (None, false, Some(code)) => Expected::Error(code),
            _ => bail!(
                "{}: case {}: give exactly one of result, \"undefined\": true or error",
                path.display(),
                index + 1
            ),
        };
        let mut bindings = file.bindings.clone();
        bindings.extend(case.bindings);
        cases.push((
            case.name,
            case.input,
            bindings.into_iter().collect(),
            expected,
        ));
    }
    Ok(Suite {
        path,
        source,
        options,
        cases,
    })
}

fn matches(actual: &jlens_core::Result<Option<Value>>, expected: &Expected) -> bool {
    match (actual, expected) {
        (Ok(Some(value)), Expected::Result(expected)) => same(value, expected),
        (Ok(None), Expected::Undefined) => true,
        (Err(err), Expected::Error(code)) => err.code().eq_ignore_ascii_case(code),
        _ => false,
    }
}

/// JSON equality, with numbers compared by value (`1` and `1.0` are the same). The recursion is
/// bounded by the depth of the expected value, which `serde_json` caps when parsing.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same(x, y))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        _ => a == b,
    }
}

fn expected_text(expected: &Expected) -> String {
    match expected {
        Expected::Result(value) => jlens_core::format_json(value, false),
        Expected::Undefined => "undefined".to_owned(),
        Expected::Error(code) => format!("error {code}"),
    }
}

fn actual_text(actual: &jlens_core::Result<Option<Value>>) -> String {
    match actual {
        Ok(Some(value)) => jlens_core::format_json(value, false),
        Ok(None) => "undefined".to_owned(),
        Err(err) => format!("error {}: {}", err.code(), err.message()),
    }
}

/// A failed case as `--json` prints it. `expected` and `actual` have the shape of the
/// `eval --json` envelope (ADR 0008); an expected error has only its `code`.
fn failure_json(
    file: &str,
    case: usize,
    name: Option<&str>,
    expected: &Expected,
    actual: &jlens_core::Result<Option<Value>>,
    source: &str,
) -> String {
    let expected = match expected {
        Expected::Result(value) => output::ok_envelope(Some(value), None),
        Expected::Undefined => output::ok_envelope(None, None),
        Expected::Error(code) => format!(r#"{{"ok":false,"error":{{"code":{}}}}}"#, json(code)),
    };
    let actual = match actual {
        Ok(value) => output::ok_envelope(value.as_ref(), None),
        Err(err) => output::error_envelope(err, source, None),
    };
    let name = name.map_or_else(String::new, |n| format!(r#""name":{},"#, json(n)));
    format!(
        r#"{{"file":{},"case":{case},{name}"expected":{expected},"actual":{actual}}}"#,
        json(file)
    )
}

fn json(text: &str) -> String {
    Value::from(text).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_compare_by_value() {
        assert!(same(&json!({"a": [1, 2.5]}), &json!({"a": [1.0, 2.5]})));
        assert!(!same(&json!([1, 2]), &json!([2, 1])));
        assert!(!same(&json!({"a": 1}), &json!({"a": 1, "b": null})));
        assert!(!same(&json!("1"), &json!(1)));
    }
}
