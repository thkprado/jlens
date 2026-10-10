//! Functions provided by the host through `Bindings::function`. The expected results are those
//! of jsonata-js 2.2.2 with the same functions passed in its bindings or registered with
//! `registerFunction`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use jlens_core::audit::{AuditOptions, OutcomeKind};
use jlens_core::{Bindings, Document, Error, Expression, HostFunction, Options};
use serde_json::{Value, json};

fn number(args: &[Option<Value>], i: usize) -> f64 {
    args.get(i)
        .and_then(Option::as_ref)
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

/// `$add(a, b)`, without a signature.
fn add() -> HostFunction {
    HostFunction::new(2, |args, _| {
        Ok(Some(json!(number(args, 0) + number(args, 1))))
    })
}

/// `$args(...)`: the arguments it receives, as an array (`null` for undefined).
fn args_of(arity: usize) -> HostFunction {
    HostFunction::new(arity, |args, _| {
        Ok(Some(Value::Array(
            args.iter()
                .map(|a| a.clone().unwrap_or(Value::Null))
                .collect(),
        )))
    })
}

fn eval_with(
    expr: &str,
    input: Option<Value>,
    bindings: &Bindings,
) -> Result<Option<Value>, Error> {
    Expression::compile(expr)
        .unwrap()
        .evaluate(input.as_ref(), bindings)
}

fn eval(expr: &str, input: Option<Value>, bindings: &Bindings) -> Option<Value> {
    eval_with(expr, input, bindings).unwrap_or_else(|e| panic!("{expr}: {e}"))
}

#[test]
fn calls_with_the_arguments_given() {
    let b = Bindings::new()
        .function("add", add())
        .function("args", args_of(1));
    assert_eq!(eval("$add(1, 2)", None, &b), Some(json!(3)));
    assert_eq!(
        eval("$add(price, 0.5)", Some(json!({"price": 2})), &b),
        Some(json!(2.5))
    );
    // Without a signature, arguments are passed as given, however many there are.
    assert_eq!(eval("$args()", None, &b), Some(json!([])));
    assert_eq!(
        eval("$args(1, 'a', missing)", Some(json!({})), &b),
        Some(json!([1, "a", null]))
    );
    // Arrays, objects and sequences arrive as JSON.
    assert_eq!(
        eval(
            "$args(items.n, {'k': items[0]})",
            Some(json!({"items": [{"n": 1}, {"n": 2}]})),
            &b
        ),
        Some(json!([[1, 2], {"k": {"n": 1}}]))
    );
}

#[test]
fn results_are_values_of_the_expression() {
    let b = Bindings::new()
        .function(
            "pair",
            HostFunction::new(0, |_, _| Ok(Some(json!([{"a": 1}, {"a": 2}])))),
        )
        .function("nothing", HostFunction::new(0, |_, _| Ok(None)));
    assert_eq!(eval("$pair().a", None, &b), Some(json!([1, 2])));
    assert_eq!(eval("$count($pair())", None, &b), Some(json!(2)));
    assert_eq!(eval("$nothing()", None, &b), None);
    assert_eq!(eval("$exists($nothing())", None, &b), Some(json!(false)));
}

#[test]
fn signatures_check_and_complete_the_arguments() {
    let tax = HostFunction::with_signature("<n-n?:n>", |args, _| {
        let rate = args[1].as_ref().and_then(Value::as_f64).unwrap_or(0.25);
        Ok(Some(json!(number(args, 0) * rate)))
    })
    .unwrap();
    assert_eq!(tax.arity(), 2);
    assert_eq!(tax.signature().unwrap().definition, "<n-n?:n>");
    let b = Bindings::new().function("tax", tax);
    assert_eq!(eval("$tax(100)", None, &b), Some(json!(25)));
    assert_eq!(eval("$tax(100, 0.5)", None, &b), Some(json!(50)));
    // The `-` modifier takes the context value.
    assert_eq!(
        eval("price.$tax()", Some(json!({"price": 8})), &b),
        Some(json!(2))
    );
    assert_eq!(eval("$tax()", Some(json!(21)), &b), Some(json!(5.25)));

    let err = eval_with("$tax('x')", None, &b).unwrap_err();
    assert_eq!(err.code(), "T0410");
    assert_eq!(err.position(), Some(5));
    assert_eq!(err.token().as_deref(), Some("tax"));

    // An array parameter wraps a single value.
    let b = Bindings::new().function(
        "first",
        HostFunction::with_signature("<a<n>:n>", |args, _| {
            Ok(args[0].as_ref().map(|a| a[0].clone()))
        })
        .unwrap(),
    );
    assert_eq!(eval("$first(7)", None, &b), Some(json!(7)));
    assert_eq!(
        eval_with("$first(['x'])", None, &b).unwrap_err().code(),
        "T0412"
    );
}

#[test]
fn invalid_signatures_are_rejected() {
    let err = HostFunction::with_signature("<n<s>:n>", |_, _| Ok(None)).unwrap_err();
    assert!(err.code().starts_with("S04"), "{err}");
}

#[test]
fn errors_point_at_the_call() {
    let fail = HostFunction::new(0, |_, _| {
        Err(Error::new("ACME01").with("message", "The rate service is down"))
    });
    let b = Bindings::new().function("rate", fail);
    let err = eval_with("1 + $rate()", None, &b).unwrap_err();
    assert_eq!(err.code(), "ACME01");
    assert_eq!(err.message(), "The rate service is down");
    assert_eq!(err.position(), Some(10));
    assert_eq!(err.token().as_deref(), Some("rate"));
}

#[test]
fn the_call_gives_the_context_value() {
    let input_of = HostFunction::new(0, |_, call| Ok(call.input()));
    let b = Bindings::new().function("input", input_of);
    assert_eq!(
        eval("$input()", Some(json!({"a": 1})), &b),
        Some(json!({"a": 1}))
    );
    assert_eq!(
        eval("a.$input()", Some(json!({"a": {"b": 2}})), &b),
        Some(json!({"b": 2}))
    );
    assert_eq!(
        eval("items.$input()", Some(json!({"items": [1, 2]})), &b),
        Some(json!([1, 2]))
    );
    // An input array is the context value as a whole.
    assert_eq!(
        eval("$input()", Some(json!([1, 2])), &b),
        Some(json!([1, 2]))
    );
    assert_eq!(eval("$input()", None, &b), None);
}

#[test]
fn the_context_value_is_converted_only_when_asked_for() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let b = Bindings::new().function(
        "tick",
        HostFunction::new(0, move |_, _| {
            counter.fetch_add(1, Ordering::Relaxed);
            Ok(Some(json!(1)))
        }),
    );
    assert_eq!(
        eval("$sum(items.$tick())", Some(json!({"items": [1, 2, 3]})), &b),
        Some(json!(3))
    );
    assert_eq!(calls.load(Ordering::Relaxed), 3);
}

#[test]
fn higher_order_functions_pass_as_many_arguments_as_the_arity() {
    let b = Bindings::new()
        .function("one", args_of(1))
        .function("two", args_of(2))
        .function("three", args_of(3));
    assert_eq!(
        eval("$map([5, 6], $one)", None, &b),
        Some(json!([[5], [6]]))
    );
    assert_eq!(
        eval("$map([5, 6], $two)", None, &b),
        Some(json!([[5, 0], [6, 1]]))
    );
    assert_eq!(
        eval("$map([5, 6], $three)", None, &b),
        Some(json!([[5, 0, [5, 6]], [6, 1, [5, 6]]]))
    );
    let positive = HostFunction::new(1, |args, _| Ok(Some(json!(number(args, 0) > 0.0))));
    let b = Bindings::new()
        .function("positive", positive)
        .function("add", add());
    assert_eq!(
        eval("$filter([-1, 2, 3], $positive)", None, &b),
        Some(json!([2, 3]))
    );
    assert_eq!(eval("$reduce([1, 2, 3], $add)", None, &b), Some(json!(6)));
    assert_eq!(
        eval("[1, 2, 3][$positive($ - 2)]", None, &b),
        Some(json!(3))
    );
}

#[test]
fn partial_application_and_chaining() {
    let b = Bindings::new()
        .function("args", args_of(3))
        .function("add", add());
    // `$f(?, 5)` has one parameter left: the others are bound, or undefined.
    assert_eq!(
        eval("($h := $args(?, 5); $h(1))", None, &b),
        Some(json!([1, 5, null]))
    );
    assert_eq!(
        eval("($h := $args(?, 5); $h(2, 3))", None, &b),
        Some(json!([2, 5, null]))
    );
    assert_eq!(eval("$add(?, 1)(41)", None, &b), Some(json!(42)));
    assert_eq!(
        eval("($h := $args(?, 5, ?); $h(?, 9)(1))", None, &b),
        Some(json!([1, 5, 9]))
    );
    let triple = HostFunction::new(1, |args, _| Ok(Some(json!(number(args, 0) * 3.0))));
    let b = b.function("triple", triple);
    assert_eq!(eval("$add(1, 1) ~> $triple", None, &b), Some(json!(6)));
    assert_eq!(
        eval("($c := $add(?, 1) ~> $triple; $c(1))", None, &b),
        Some(json!(6))
    );
}

#[test]
fn host_functions_are_function_values() {
    let b = Bindings::new().function("add", add());
    assert_eq!(eval("$type($add)", None, &b), Some(json!("function")));
    assert_eq!(eval("$string($add)", None, &b), Some(json!("")));
    assert_eq!(eval("$add = $add", None, &b), Some(json!(true)));
    assert_eq!(eval("($f := $add; $f(1, 2))", None, &b), Some(json!(3)));
    assert_eq!(eval("$eval('$add(1, 2)')", None, &b), Some(json!(3)));
    // A function as the result is undefined, and a function argument arrives as undefined.
    assert_eq!(eval("$add", None, &b), None);
    let b = b.function("args", args_of(1));
    assert_eq!(
        eval("$args($add, function($x) { $x })", None, &b),
        Some(json!([null, null]))
    );
}

#[test]
fn bindings_shadow_built_in_functions() {
    let b = Bindings::new().function("sum", HostFunction::new(1, |_, _| Ok(Some(json!("mine")))));
    assert_eq!(eval("$sum([1, 2])", None, &b), Some(json!("mine")));
    // An expression's own bindings shadow host functions.
    assert_eq!(
        eval("($sum := function($a) { 0 }; $sum([1]))", None, &b),
        Some(json!(0))
    );
}

#[test]
fn a_name_holds_a_value_or_a_function() {
    let b = Bindings::new().var("f", 1).function("f", add());
    assert!(b.get("f").is_none());
    assert_eq!(eval("$f(1, 2)", None, &b), Some(json!(3)));
    let b = b.var("f", 1);
    assert!(b.get_function("f").is_none());
    assert_eq!(eval("$f", None, &b), Some(json!(1)));
    let b = Bindings::new()
        .var("x", 1)
        .function("add", add())
        .function("args", args_of(0));
    let names: Vec<&str> = b.functions().map(|(name, _)| name).collect();
    assert_eq!(names, ["add", "args"]);
    assert_eq!(b.iter().count(), 1);
}

#[test]
fn documents_and_audits_see_host_functions() {
    let b = Bindings::new().function("add", add());
    let doc = Document::new(&json!({"price": 2}));
    let expr = Expression::compile("$add(price, 1) > 2").unwrap();
    assert_eq!(
        expr.evaluate_document(&doc, &b, &Options::default())
            .unwrap(),
        Some(json!(true))
    );
    let input = json!({"price": 2});
    let (result, audit) = expr.evaluate_audited(
        Some(&input),
        &b,
        &Options::default(),
        &AuditOptions::decisions(),
    );
    assert_eq!(result.unwrap(), Some(json!(true)));
    let root = audit.root.unwrap();
    assert_eq!(root.outcome, OutcomeKind::True);
    assert_eq!(root.operands, Some([Some(json!(3)), Some(json!(2))]));
}

#[test]
fn bindings_with_functions_can_be_shared_between_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Bindings>();
    assert_send_sync::<HostFunction>();
    let b = Arc::new(Bindings::new().function("add", add()));
    let expr = Arc::new(Expression::compile("$add(1, 2)").unwrap());
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let (b, expr) = (b.clone(), expr.clone());
            std::thread::spawn(move || expr.evaluate(None, &b).unwrap())
        })
        .collect();
    for h in handles {
        assert_eq!(h.join().unwrap(), Some(json!(3)));
    }
}
