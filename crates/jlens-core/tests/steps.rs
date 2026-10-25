//! The step limit (ADR 0018): exact counts that change only on purpose.
//!
//! The table is `steps.json`: a change to a count shows there, and goes in the changelog under
//! its own heading.

use jlens_core::audit::AuditOptions;
use jlens_core::{Bindings, Expression, HostFunction, Options};
use serde_json::{Value, json};

/// The table, shared with the WebAssembly package's JS suite and the Python package's pytest.
const TABLE: &str = include_str!("steps.json");

fn table() -> Value {
    serde_json::from_str(TABLE).expect("steps.json is JSON")
}

/// The input the table's expressions are evaluated against.
fn input() -> Value {
    table()["input"].clone()
}

/// Each expression of the table and the steps it takes.
fn counts() -> Vec<(String, u64)> {
    let table = table();
    let groups = table["groups"].as_array().expect("groups");
    groups
        .iter()
        .flat_map(|group| group["counts"].as_array().expect("counts"))
        .map(|entry| {
            let source = entry[0].as_str().expect("an expression");
            (source.to_owned(), entry[1].as_u64().expect("a count"))
        })
        .collect()
}

fn evaluate(
    expr: &Expression,
    input: &Value,
    bindings: &Bindings,
    steps: u64,
) -> Result<Option<Value>, jlens_core::Error> {
    expr.evaluate_with(Some(input), bindings, &Options::new().max_steps(steps))
}

/// Checks that `source` takes exactly `steps` steps: it succeeds with that budget, fails with
/// `U1006` with one less, and an audit reports that many.
fn check(source: &str, input: &Value, bindings: &Bindings, steps: u64) -> Vec<String> {
    let expr = Expression::compile(source).unwrap();
    let mut failures = Vec::new();
    let (_, audit) = expr.evaluate_audited(
        Some(input),
        bindings,
        &Options::new(),
        &AuditOptions::decisions(),
    );
    if audit.header.steps != steps {
        failures.push(format!(
            "{source}: the audit counts {} steps, not {steps}",
            audit.header.steps
        ));
    }
    if let Err(e) = evaluate(&expr, input, bindings, steps) {
        failures.push(format!("{source}: fails with {steps} steps: {e}"));
    }
    if let Some(fewer) = steps.checked_sub(1) {
        match evaluate(&expr, input, bindings, fewer) {
            Err(e) if e.code() == "U1006" => {
                assert_eq!(e.value(), Some(&json!(fewer)), "{source}");
            }
            outcome => failures.push(format!("{source}: {outcome:?} with {fewer} steps")),
        }
    }
    failures
}

#[test]
fn exact_counts() {
    let input = input();
    let failures: Vec<String> = counts()
        .iter()
        .flat_map(|(source, steps)| check(source, &input, &Bindings::new(), *steps))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn host_callbacks_spend_the_budget() {
    let twice = HostFunction::new(2, |args, call| {
        let f = call.function(0).expect("a function");
        let once = f.call(args.get(1..).unwrap_or_default())?;
        f.call(&[once])
    });
    let bindings = Bindings::new().function("twice", twice);
    // The call, its name and its arguments, then each call back: a step, and the body's
    // 3 nodes.
    let failures = check("$twice(function($x){ $x + 1 }, 1)", &input(), &bindings, 12);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn an_exhausted_budget_fails_every_evaluation() {
    let expr = Expression::compile("1").unwrap();
    let err = evaluate(&expr, &input(), &Bindings::new(), 0).unwrap_err();
    assert_eq!(err.code(), "U1006");
    assert_eq!(err.value(), Some(&json!(0)));
}

/// `$eval` reports other errors of its text as `D3121`, but not the budget's.
#[test]
fn eval_does_not_hide_an_exhausted_budget() {
    let expr = Expression::compile(r#"$eval("$sum([1..10].($ * 2))")"#).unwrap();
    let err = evaluate(&expr, &input(), &Bindings::new(), 10).unwrap_err();
    assert_eq!(err.code(), "U1006");
}

/// A regular expression that backtracks catastrophically stops under a step limit alone.
#[test]
fn catastrophic_regexes_stop_on_steps() {
    let source = format!("$contains('{}!', /^(a+)+$/)", "a".repeat(40));
    let expr = Expression::compile(&source).unwrap();
    let started = std::time::Instant::now();
    let err = evaluate(&expr, &input(), &Bindings::new(), 1_000).unwrap_err();
    assert_eq!(err.code(), "U1006");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

/// A predicate evaluated once for all the items spends the steps of each, also when a timeout's
/// read of the clock falls inside it (found with `PROPTEST_CASES=100000`).
#[test]
fn a_constant_predicate_counts_its_steps_across_a_clock_read() {
    // More steps than the clock is read every, so one read falls inside the predicate.
    let source = format!("($i := 0; [1, 2, 3][{}])", vec!["$i"; 300].join(" + "));
    let expr = Expression::compile(&source).unwrap();
    let (_, audit) = expr.evaluate_audited(
        Some(&input()),
        &Bindings::new(),
        &Options::new(),
        &AuditOptions::decisions(),
    );
    let steps = audit.header.steps;
    let options = Options::new().timeout(std::time::Duration::from_secs(60));
    let within = expr.evaluate_with(
        Some(&input()),
        &Bindings::new(),
        &options.clone().max_steps(steps),
    );
    assert_eq!(within.unwrap(), Some(json!(1)));
    let fewer = expr.evaluate_with(
        Some(&input()),
        &Bindings::new(),
        &options.max_steps(steps - 1),
    );
    assert_eq!(fewer.unwrap_err().code(), "U1006");
}
