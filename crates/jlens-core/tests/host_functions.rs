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
fn fallbacks_call_their_left_side_once() {
    // jsonata-js evaluates the left side of `?:` and `??` twice: once to test it, once for the
    // result. jlens keeps the value it tested.
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let b = Bindings::new().function(
        "next",
        HostFunction::new(0, move |_, _| {
            Ok(Some(json!(counter.fetch_add(1, Ordering::SeqCst) + 1)))
        }),
    );
    assert_eq!(eval("$next() ?: 0", None, &b), Some(json!(1)));
    assert_eq!(eval("$next() ?? 0", None, &b), Some(json!(2)));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
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

/// `$apply($f, args...)`: calls its first argument with the others (ADR 0011).
fn apply() -> HostFunction {
    HostFunction::new(2, |args, call| match call.function(0) {
        Some(f) => f.call(args.get(1..).unwrap_or_default()),
        None => Err(Error::new("ACME02").with("message", "Not a function")),
    })
}

fn callback_bindings() -> Bindings {
    Bindings::new()
        .function("apply", apply())
        .function("add", add())
        .function(
            "arity",
            HostFunction::new(1, |_, call| Ok(call.function(0).map(|f| json!(f.arity())))),
        )
}

#[test]
fn host_functions_call_the_functions_they_receive() {
    let b = callback_bindings();
    let input = json!({"a": {"b": 5}, "items": [{"n": 1}, {"n": 2}]});
    for (expr, expected) in [
        ("$apply(function($x) { $x * 2 }, 21)", json!(42)),
        ("$apply($uppercase, 'a')", json!("A")),
        ("$apply($add, 1, 2)", json!(3)),
        ("$apply($substring(?, 1), 'abc')", json!("bc")),
        ("$apply($uppercase ~> $trim, ' a ')", json!("A")),
        // A closure keeps the variables and the context of its definition.
        ("($k := 10; $apply(function($x) { $x + $k }, 1))", json!(11)),
        ("a.$apply(function() { b })", json!(5)),
        // The context value of the call is null, as in jsonata-js.
        ("$apply($string)", json!("null")),
        // A sequence comes back as an array.
        ("$apply(function() { items.n })", json!([1, 2])),
        // A partial application of the host function.
        (
            "($p := $apply(function($x) { $x + 1 }, ?); $p(1))",
            json!(2),
        ),
    ] {
        assert_eq!(
            eval(expr, Some(input.clone()), &b),
            Some(expected),
            "{expr}"
        );
    }
    // A function result crosses as undefined.
    assert_eq!(eval("$apply(function() { $sum })", None, &b), None);
    // Arguments that are not functions give no handle, and a function passed to a callable
    // crosses as undefined.
    assert_eq!(
        eval_with("$apply($apply, $add, 1, 2)", None, &b)
            .unwrap_err()
            .code(),
        "ACME02"
    );
    assert_eq!(
        eval_with("$apply(1)", None, &b).unwrap_err().code(),
        "ACME02"
    );
}

#[test]
fn callables_have_the_arity_of_their_function() {
    let b = callback_bindings();
    for (expr, expected) in [
        ("$arity(function($x, $y) { $x })", json!(2)),
        ("$arity($substring)", json!(3)),
        ("$arity($substring(?, 1))", json!(1)),
        ("$arity($add)", json!(2)),
        ("$arity(1)", Value::Null),
    ] {
        let result = eval(expr, None, &b).unwrap_or(Value::Null);
        assert_eq!(result, expected, "{expr}");
    }
}

#[test]
fn arguments_are_indexed_after_the_signature_check() {
    // The `-` modifier puts the context value first.
    let b = Bindings::new().function(
        "next",
        HostFunction::with_signature("<n-f:n>", |args, call| {
            call.function(1).unwrap().call(&args[..1])
        })
        .unwrap(),
    );
    assert_eq!(
        eval(
            "n.$next(function($x) { $x + 1 })",
            Some(json!({"n": 5})),
            &b
        ),
        Some(json!(6))
    );
}

#[test]
fn errors_of_a_callable_reach_the_host_function_with_their_codes() {
    let b = callback_bindings().function(
        "code",
        HostFunction::new(1, |_, call| match call.function(0).unwrap().call(&[]) {
            Ok(_) => Ok(None),
            Err(e) => Ok(Some(json!(e.code()))),
        }),
    );
    assert_eq!(
        eval("$code(function() { 'a' + 1 })", None, &b),
        Some(json!("T2001"))
    );
    assert_eq!(
        eval("$code(function() { $error('boom') })", None, &b),
        Some(json!("D3137"))
    );
    // Returned, the error ends the evaluation, with its own position.
    let err = eval_with("$apply(function() { 'a' + 1 })", None, &b).unwrap_err();
    let alone = eval_with("'a' + 1", None, &b).unwrap_err();
    assert_eq!(err.code(), "T2001");
    assert_eq!(err.position(), alone.position().map(|p| p + 20));
}

#[test]
fn a_recursion_through_a_host_function_ends_with_d1011() {
    let b = callback_bindings();
    let expr = "($f := function($n) { $apply($f, $n + 1) }; $f(0))";
    assert_eq!(eval_with(expr, None, &b).unwrap_err().code(), "D1011");
    let options = Options::new().max_depth(100);
    let err = Expression::compile(expr)
        .unwrap()
        .evaluate_with(None, &b, &options)
        .unwrap_err();
    assert_eq!(err.code(), "D1011");
}

#[test]
fn callables_check_the_timeout_and_the_cancellation() {
    // `$forever($f)` calls `$f` until it fails.
    let forever = HostFunction::new(1, |_, call| {
        let f = call.function(0).unwrap();
        loop {
            f.call(&[])?;
        }
    });
    let b = Bindings::new().function("forever", forever);
    let expr = Expression::compile("$forever(function() { 1 })").unwrap();
    let options = Options::new().timeout(std::time::Duration::from_millis(20));
    let err = expr.evaluate_with(None, &b, &options).unwrap_err();
    assert_eq!(err.code(), "D1012");

    let token = jlens_core::CancelToken::new();
    let cancel = token.clone();
    let b = Bindings::new().function(
        "cancel",
        HostFunction::new(1, move |_, call| {
            cancel.cancel();
            call.function(0).unwrap().call(&[])
        }),
    );
    let expr = Expression::compile("$cancel(function() { 1 })").unwrap();
    let options = Options::new().cancel_token(token);
    let err = expr.evaluate_with(None, &b, &options).unwrap_err();
    assert_eq!(err.code(), "U1003");
}

#[test]
fn the_result_of_a_callable_is_limited_in_size() {
    let b = callback_bindings();
    let expr = Expression::compile("$apply(function() { [1..100] })").unwrap();
    let options = Options::new().max_result_size(64);
    let err = expr.evaluate_with(None, &b, &options).unwrap_err();
    assert_eq!(err.code(), "U1001");
}
