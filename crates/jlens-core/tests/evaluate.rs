//! End-to-end evaluation through the public API.

use std::time::Duration;

use jlens_core::{Bindings, Document, Expression, Options};
use serde_json::{Value, json};

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

fn eval(expr: &str, input: Value) -> Option<Value> {
    Expression::compile(expr)
        .unwrap()
        .evaluate(Some(&input), &Bindings::new())
        .unwrap_or_else(|e| panic!("{expr}: {e}"))
}

#[test]
fn shipping_rule_fixture_reaches_the_expected_decision() {
    let rule = Expression::compile(&fixture("shipping-rule.jsonata")).unwrap();
    let doc: Value = serde_json::from_str(&fixture("online-store-order.json")).unwrap();
    let result = rule.evaluate(Some(&doc), &Bindings::new()).unwrap();
    assert_eq!(
        result,
        Some(json!({"action": "FREE_SHIPPING", "code": "OK_200"}))
    );

    // Blocking the customer flips the decision.
    let mut blocked = doc.clone();
    blocked["customer"]["login"] = json!("mallory.cheat1");
    let result = rule.evaluate(Some(&blocked), &Bindings::new()).unwrap();
    assert_eq!(result.unwrap()["action"], "REGULAR_FEE");
}

#[test]
fn filters_aggregates_and_construction() {
    let doc: Value = serde_json::from_str(&fixture("online-store-order.json")).unwrap();
    assert_eq!(
        eval(
            "items[type = 'electronic'].(unitPrice * quantity) ~> $sum()",
            doc.clone()
        ),
        Some(json!(37000))
    );
    assert_eq!(
        eval("items{type: $sum($.(unitPrice * quantity))}", doc.clone()),
        Some(json!({"apparel": 20000, "electronic": 37000}))
    );
    assert_eq!(eval("items[0].code", doc.clone()), Some(json!("1")));
    assert_eq!(eval("items[-1].color", doc.clone()), Some(json!("white")));
    assert_eq!(
        eval("items^(>tax).code", doc.clone()),
        Some(json!(["2", "3", "1"]))
    );
    assert_eq!(eval("missing.path", doc), None);
}

#[test]
fn sequences_and_singletons() {
    let data = json!({"a": [{"b": 1}], "c": [1, 2]});
    assert_eq!(eval("a.b", data.clone()), Some(json!(1)));
    assert_eq!(eval("a.b[]", data.clone()), Some(json!([1])));
    assert_eq!(eval("c", data.clone()), Some(json!([1, 2])));
    assert_eq!(eval("$", json!([1, 2])), Some(json!([1, 2])));
}

#[test]
fn a_frame_keeps_its_variables_however_many_it_holds() {
    // Up to, at and past the number of variables a frame lists before it moves them to a table.
    for count in [1, 7, 8, 9, 10, 40] {
        let names: Vec<String> = (0..count).map(|i| format!("v{i}")).collect();
        let binds: String = names
            .iter()
            .enumerate()
            .map(|(i, n)| format!("${n} := {i}; "))
            .collect();
        let sum = names
            .iter()
            .map(|n| format!("${n}"))
            .collect::<Vec<_>>()
            .join(" + ");
        let total: i64 = (0..count).sum();

        // Each one found, and each one bound again in the same frame, before and after a move.
        let rebound = format!(
            "({binds}$v0 := 100; ${} := 200; {sum})",
            names[count as usize - 1]
        );
        let expected = if count == 1 {
            200
        } else {
            total + 100 + 200 - (count - 1)
        };
        assert_eq!(
            eval(&rebound, json!(null)),
            Some(json!(expected)),
            "{rebound}"
        );

        // A closure sees them, a function's name is kept with them, and an inner frame hides one.
        let closure = format!(
            "({binds}$f := function($v0){{ {sum} }}; $g := function(){{ $sum([1, {sum}]) }}; [$f(1000), $g()])"
        );
        assert_eq!(
            eval(&closure, json!(null)),
            Some(json!([total + 1000, total + 1])),
            "{closure}"
        );

        // As many variables from the host, in the root frame, with the built-ins it keeps.
        let bindings = names
            .iter()
            .enumerate()
            .fold(Bindings::new(), |b, (i, n)| b.var(n.clone(), i));
        let bound = format!("$sum([{}]) + $count([$v0]) + $max([$v0])", sum);
        assert_eq!(
            Expression::compile(&bound)
                .unwrap()
                .evaluate(None, &bindings)
                .unwrap(),
            Some(json!(total + 1)),
            "{bound}"
        );
    }
}

#[test]
fn a_frame_keeps_names_of_any_length() {
    // Around the bytes kept in place, by characters of one and of two bytes.
    let names = [
        "a".to_owned(),
        "n".repeat(13),
        "n".repeat(14),
        "n".repeat(15),
        "n".repeat(16),
        "n".repeat(100),
        "é".repeat(7),
        "é".repeat(8),
    ];
    for name in &names {
        // A name that differs in its last byte, one that is one byte longer and one shorter.
        let other = format!(
            "{}x",
            &name[..name.len() - name.chars().last().unwrap().len_utf8()]
        );
        let longer = format!("{name}x");
        let cases = [
            (format!("(${name} := 1; ${name})"), json!(1)),
            (format!("(${name} := 1; ${name} := 2; ${name})"), json!(2)),
            (
                format!(
                    "(${name} := 1; ${other} := 2; ${longer} := 3; [${name}, ${other}, ${longer}])"
                ),
                json!([1, 2, 3]),
            ),
            (
                format!("(${other} := 2; ${name} := 1; ${other} := 4; [${name}, ${other}])"),
                json!([1, 4]),
            ),
            // A name held that starts with the one looked up, in the frame's one place and list.
            (
                format!("(${longer} := 3; [${name}, ${longer}])"),
                json!([3]),
            ),
            (
                format!("(${longer} := 3; ${name} := 1; [${name}, ${longer}])"),
                json!([1, 3]),
            ),
            // In the list and in the table of a frame, bound again.
            (
                format!(
                    "($v0 := 0; $v1 := 1; $v2 := 2; $v3 := 3; $v4 := 4; $v5 := 5; $v6 := 6;                      $v7 := 7; ${name} := 8; ${name} := 9; ${longer} := 10; [${name}, ${longer}, $v0])"
                ),
                json!([9, 10, 0]),
            ),
            // An inner frame hides it, and the outer one still holds it.
            (
                format!("(${name} := 1; [(${name} := 2; ${name}), ${name}])"),
                json!([2, 1]),
            ),
            // The frames of a tuple's step.
            (format!("v@${name}.${name}"), json!([3, 4])),
            (format!("v#${name}.${name}"), json!([0, 1])),
            // In a lambda's frame, a name of its body.
            (
                format!("function($p){{ $append(${name} := 5, ${name} + $p) }}(1)"),
                json!([5, 6]),
            ),
        ];
        for (expr, expected) in cases {
            assert_eq!(eval(&expr, json!({"v": [3, 4]})), Some(expected), "{expr}");
        }

        // From the host, in the root frame, where a name shorter or longer finds nothing.
        let bindings = Bindings::new().var(name.clone(), 1);
        let bound = format!("[${name}, ${other}, ${longer}]");
        assert_eq!(
            Expression::compile(&bound)
                .unwrap()
                .evaluate(None, &bindings)
                .unwrap(),
            Some(json!([1])),
            "{bound}"
        );
    }
}

#[test]
fn a_lambda_frame_names_its_arguments_as_if_each_were_bound() {
    // A call's frame holds the arguments it was given and takes their names from the function.
    // The results are those of jsonata-js.
    let cases = [
        // Of two parameters of one name the last holds, given or not.
        ("function($x, $x){ $x }(1, 2)", Some(json!(2))),
        ("function($x, $x){ $x }(1)", None),
        // Arguments too many are dropped; one missing hides the variable outside.
        ("function($x){ $x }(1, 2, 3)", Some(json!(1))),
        ("($x := 1; function($x){ $x }())", None),
        // The body binds in the frame, a parameter or a name of its own, with no block around.
        ("function($x){ $x := 2 }(1)", Some(json!(2))),
        ("function($x){ $y := $x + 1 }(1)", Some(json!(2))),
        (
            "function($x){ $append($x := 5, $x) }(1)",
            Some(json!([5, 5])),
        ),
        (
            "function($x){ $append($y := $x + 1, $append($y, $x)) }(1)",
            Some(json!([2, 2, 1])),
        ),
        (
            "function($x, $x){ $append($y := 5, $x) }(1, 2)",
            Some(json!([5, 2])),
        ),
        // Past the number of variables a list holds.
        (
            "function($a, $b, $c, $d, $e, $f, $g, $h, $i, $j){ \
             $append($k := 11, $append($a, $append($j, $k))) }(1, 2, 3, 4, 5, 6, 7, 8, 9, 10)",
            Some(json!([11, 1, 10, 11])),
        ),
        // A closure keeps the frame after the call, and sees what the body binds after it.
        (
            "($f := function($x){ function(){ $x } }; $f(3)())",
            Some(json!(3)),
        ),
        (
            "function($x){ $append($string($h := function(){ $y + $x }), \
             $append($y := 1, $h())) }(2)",
            Some(json!(["", 1, 3])),
        ),
        // A partial application names what is left to supply.
        (
            "($f := function($a, $b, $c){ [$a, $b, $c] }; \
             $g := $f(?, 2, ?); $h := $g(1, ?); $h(3))",
            Some(json!([1, 2, 3])),
        ),
    ];
    for (expr, expected) in cases {
        assert_eq!(eval(expr, json!(null)), expected, "{expr}");
    }
}

#[test]
fn a_constructor_evaluates_its_items_in_order() {
    // The first two differ from jsonata-js on purpose (compatibility.md): its items run
    // concurrently, so that a plain `$x` reads the value from before the binding.
    let cases = [
        ("($x := 1; [$x := 5, $x])", json!([5, 5])),
        (
            r#"($x := 1; {"a": $x := 5, "b": $x})"#,
            json!({"a": 5, "b": 5}),
        ),
        ("($x := 1; [$x := 5, 0 + $x])", json!([5, 5])),
        ("($x := 1; [$x, $x := 5])", json!([1, 5])),
    ];
    for (expr, expected) in cases {
        assert_eq!(eval(expr, json!(null)), Some(expected), "{expr}");
    }
}

#[test]
fn bindings_and_closures() {
    let expr = Expression::compile("$limit < $sum(values) ? 'over' : 'under'").unwrap();
    let bindings = Bindings::new().var("limit", 10);
    let input = json!({"values": [4, 5, 6]});
    assert_eq!(
        expr.evaluate(Some(&input), &bindings).unwrap(),
        Some(json!("over"))
    );

    assert_eq!(
        eval(
            "($add := function($x){ function($y){ $x + $y } }; $add(2)(3))",
            json!(null)
        ),
        Some(json!(5))
    );
    // Tail calls run in constant stack depth.
    assert_eq!(
        eval(
            "($loop := function($n, $acc){ $n = 0 ? $acc : $loop($n - 1, $acc + 1) }; $loop(100000, 0))",
            json!(null)
        ),
        Some(json!(100000))
    );
}

#[test]
fn a_group_by_on_a_tail_call_is_applied() {
    // jsonata-js runs a call in tail position without its group-by; jlens applies it, as
    // outside a lambda (`compatibility.md`).
    let input = json!({"items": [{"a": 1, "k": "x"}, {"a": 2, "k": "y"}, {"a": 3, "k": "x"}]});
    let grouped = json!({"x": 4, "y": 2});
    for body in [
        "$append(items, []){k: $sum(a)}",
        "true ? $append(items, []){k: $sum(a)}",
        "false ? 0 : $append(items, []){k: $sum(a)}",
        "($x := 1; $append(items, []){k: $sum(a)})",
        "nothing ?? $append(items, []){k: $sum(a)}",
        "$append(items, []){k: $sum(a)} ?: 0",
    ] {
        let source = format!("($f := function() {{ {body} }}; $f())");
        assert_eq!(
            eval(&source, input.clone()),
            Some(grouped.clone()),
            "{body}"
        );
    }
    assert_eq!(
        eval(
            "($f := function() { $zip(items){'k': $} }; $f())",
            input.clone()
        ),
        Some(json!({"k": input["items"]}))
    );
    // The calls without a group-by are still tail calls: the recursion runs in constant stack
    // depth.
    assert_eq!(
        eval(
            "($f := function($n){ $n = 0 ? items{k: $sum(a)} : $f($n - 1) }; $f(100000))",
            input
        ),
        Some(grouped)
    );
}

#[test]
fn errors_carry_code_position_and_token() {
    let err = Expression::compile("1 + 'a'")
        .unwrap()
        .evaluate(None, &Bindings::new())
        .unwrap_err();
    assert_eq!(err.code(), "T2002");
    assert_eq!(err.position(), Some(3));
    assert_eq!(err.token().as_deref(), Some("+"));
    assert_eq!(
        err.message(),
        r#"The right side of the "+" operator must evaluate to a number"#
    );
}

#[test]
fn guardrails_stop_runaway_evaluations() {
    let deep = Expression::compile("($f := function($n){ 1 + $f($n + 1) }; $f(0))").unwrap();
    let options = Options::new().max_depth(500);
    let err = deep
        .evaluate_with(None, &Bindings::new(), &options)
        .unwrap_err();
    assert_eq!(err.code(), "D1011");

    let forever = Expression::compile("($f := function($n){ $f($n + 1) }; $f(0))").unwrap();
    let options = Options::new().timeout(Duration::from_millis(50));
    let err = forever
        .evaluate_with(None, &Bindings::new(), &options)
        .unwrap_err();
    assert_eq!(err.code(), "D1012");
}

#[test]
fn recursion_through_native_callbacks_is_bounded() {
    // The lambda's body is a tail call, so no expression stays on the stack between levels:
    // only the calls back from the native function count toward the depth.
    for callback in [
        "$map([$n], function($v){ $f($v) })",
        "$filter([$n], function($v){ $f($v) })",
        "$reduce([1, 2], function($a, $v){ $f($v) })",
        "$sort([1, 2], function($a, $b){ $f($n) })",
        "$sift({'a': 1}, function($v){ $f($v) })",
        "$each({'a': 1}, function($v){ $f($v) })",
        "$single([$n], function($v){ $f($v) })",
        "$replace('a', /a/, function($m){ $f($n) })",
        "$replace('a', function($s){ $f($n) }, 'b')",
    ] {
        let source = format!("($f := function($n){{ {callback} }}; $f(0))");
        let options = Options::new().max_depth(2_000);
        let err = Expression::compile(&source)
            .unwrap()
            .evaluate_with(None, &Bindings::new(), &options)
            .unwrap_err();
        assert_eq!(err.code(), "D1011", "{source}");
    }
}

#[test]
fn guardrails_stop_catastrophic_regexes() {
    // Without a limit this match would backtrack for years (ADR 0009).
    let source = format!("$contains('{}!', /^(a+)+$/)", "a".repeat(40));
    let expr = Expression::compile(&source).unwrap();

    let options = Options::new().timeout(Duration::from_millis(50));
    let started = std::time::Instant::now();
    let err = expr
        .evaluate_with(None, &Bindings::new(), &options)
        .unwrap_err();
    assert_eq!(err.code(), "D1012");
    assert!(started.elapsed() < Duration::from_secs(2));

    let token = jlens_core::CancelToken::new();
    let options = Options::new().cancel_token(token.clone());
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        token.cancel();
    });
    let err = expr
        .evaluate_with(None, &Bindings::new(), &options)
        .unwrap_err();
    assert_eq!(err.code(), "U1003");
    canceller.join().unwrap();
}

/// `$eval` reports an error of its text as `D3121`, but a limit of the evaluation passes
/// through, located at the call of `$eval` as `D3121` is.
#[test]
fn eval_lets_the_limits_of_the_evaluation_through() {
    let run = |text: &str, options: &Options| {
        let source = format!("1 + $eval({})", serde_json::to_string(text).unwrap());
        Expression::compile(&source)
            .unwrap()
            .evaluate_with(None, &Bindings::new(), options)
            .unwrap_err()
    };
    let wrapped = run("1 + 'a'", &Options::new());
    assert_eq!(wrapped.code(), "D3121");

    let forever = "($f := function($n){ $f($n + 1) }; $f(0))";
    let token = jlens_core::CancelToken::new();
    let cancel = || {
        let token = token.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            token.cancel();
        })
    };
    let cancelled = Options::new().cancel_token(token.clone());
    let mut canceller = None;
    for (text, options, code) in [
        (
            "($f := function($n){ 1 + $f($n + 1) }; $f(0))",
            Options::new().max_depth(500),
            "D1011",
        ),
        (
            forever,
            Options::new().timeout(Duration::from_millis(50)),
            "D1012",
        ),
        ("[1..1000]", Options::new().max_array_length(100), "U1001"),
        (forever, cancelled, "U1003"),
        (
            "$sum([1..10].($ * 2))",
            Options::new().max_steps(10),
            "U1006",
        ),
    ] {
        // The token is cancelled while `$eval` runs, not before the evaluation starts.
        if code == "U1003" {
            canceller = Some(cancel());
        }
        let err = run(text, &options);
        assert_eq!(err.code(), code, "{text}");
        assert_eq!(err.position(), wrapped.position(), "{text}");
        assert_eq!(err.token(), wrapped.token(), "{text}");
    }
    canceller.unwrap().join().unwrap();
}

#[test]
fn deeply_nested_regexes_are_syntax_errors() {
    // regress 0.10.5 overflowed the stack parsing this; 0.12.0 caps the nesting.
    let source = format!("/{}a{}/", "(".repeat(50_000), ")".repeat(50_000));
    let err = Expression::compile(&source)
        .and_then(|e| e.evaluate(None, &Bindings::new()))
        .unwrap_err();
    assert_eq!(err.code(), "S0302");
}

#[test]
fn jsonata_js_quirks_that_expose_javascript_are_dropped() {
    let input = json!({"o": {"a": 1}, "list": [1, [null, []], true]});
    // A regex applied to a value that is not a string gives undefined, not a match of
    // JavaScript's `String(value)`.
    for expr in [
        r"0 ~> /\d+/",
        r"missing ~> /\w+/",
        r"o ~> /\[.*\]/",
        "list ~> /.+/",
    ] {
        assert_eq!(eval(expr, input.clone()), None, "{expr}");
    }
    assert_eq!(eval("('abc' ~> /b/).match", json!(null)), Some(json!("b")));
    // A grouping separator needs a digit on its left.
    let formatted =
        |n: i32, picture: &str| eval(&format!("$formatNumber({n}, '{picture}')"), json!(null));
    for (n, expected) in [
        (0, "0"),
        (12, "12"),
        (123, "123"),
        (1234, "1,234"),
        (12345, "12,345"),
        (123456, "1,23,456"),
        (1234567, "12,34,567"),
        (-1234, "-1,234"),
    ] {
        assert_eq!(formatted(n, "#,##,##0"), Some(json!(expected)), "{n}");
    }
    assert_eq!(formatted(1234, "#,##,##0.00"), Some(json!("1,234.00")));
    assert_eq!(formatted(1, "0,000"), Some(json!("0,001")));
    // `$substringBefore` and `$substringAfter` without `chars` give the string unchanged.
    let text = json!("a-undefined-b");
    assert_eq!(
        eval("$substringBefore($, missing)", text.clone()),
        Some(text.clone())
    );
    assert_eq!(
        eval("$substringAfter($, missing)", text.clone()),
        Some(text.clone())
    );
    assert_eq!(
        eval("$substringAfter($, '-')", text),
        Some(json!("undefined-b"))
    );
}

#[test]
fn quirks_found_by_the_differential_tests_match_jsonata_js() {
    // `$base64decode` reads each UTF-16 code unit as its low byte, as Node's `Buffer` does:
    // `𝄞` is U+D834 U+DD1E, read as the digit `4` and an ignored byte.
    let decoded = |s: &str| eval(&format!("$base64decode('{s}')"), json!(null));
    assert_eq!(decoded("hello 𝄞"), decoded("hello4"));
    assert_eq!(decoded("helloŁ"), decoded("helloA"));
    assert_eq!(decoded("aĀb"), Some(json!("i")));
}

#[test]
fn order_by_evaluates_each_key_once_per_item() {
    use jlens_core::HostFunction;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let bindings = Bindings::new().function(
        "key",
        HostFunction::new(1, move |args, _| {
            counted.fetch_add(1, Ordering::Relaxed);
            Ok(args[0].clone())
        }),
    );
    let run = |expr: &str, input: Value| {
        calls.store(0, Ordering::Relaxed);
        let result = Expression::compile(expr)
            .unwrap()
            .evaluate(Some(&input), &bindings)
            .unwrap();
        (result, calls.load(Ordering::Relaxed))
    };

    let input = json!([5, 3, 8, 1, 9, 2, 7, 4, 6, 0]);
    assert_eq!(
        run("$^($key($))", input),
        (Some(json!([0, 1, 2, 3, 4, 5, 6, 7, 8, 9])), 10)
    );
    // A second term is evaluated only for the items that tie on the first.
    let input = json!([
        {"a": 2, "b": "x"}, {"a": 1, "b": "z"}, {"a": 2, "b": "w"}, {"a": 3, "b": "y"}
    ]);
    assert_eq!(
        run("$^($key(a), $key(b)).b", input),
        (Some(json!(["z", "w", "x", "y"])), 6)
    );
    // Tuple streams bind each item's variables for its keys.
    let input = json!({"items": [{"n": 2}, {"n": 1}, {"n": 3}]});
    assert_eq!(
        run("items@$i^($key($i.n)).$i.n", input),
        (Some(json!([1, 2, 3])), 3)
    );
}

#[test]
fn order_by_raises_the_first_error_jsonata_js_raises() {
    // The halves of the merge sort are sorted first, so the last two items are compared before
    // the first one is: these are the codes jsonata-js 2.2.2 raises.
    for (input, code) in [
        (json!([{"a": 1}, {"a": "x"}]), "T2007"),
        (json!([{"a": 1}, {"a": "x"}, {"a": true}]), "T2008"),
        (json!([{"a": true}, {"a": "x"}, {"a": 1}]), "T2007"),
        (json!([{"a": "x"}, {"a": 1}, {"a": [1]}]), "T2008"),
    ] {
        let err = Expression::compile("$^(a)")
            .unwrap()
            .evaluate(Some(&input), &Bindings::new())
            .unwrap_err();
        assert_eq!(err.code(), code, "{input}");
    }
}

#[test]
fn order_by_swaps_nan_keys_both_ways_as_jsonata_js() {
    // jsonata-js compares keys with `===` and `<`: a NaN key is neither equal to nor less than
    // another, so the comparison asks for a swap either way. These are its 2.2.2 results.
    let digits = json!([5, 3, 8, 1, 9, 2, 7, 4, 6, 0]);
    for (expr, input, expected) in [
        ("$^(0 / 0)", json!([1, 2, 3, 4, 5]), json!([5, 4, 3, 2, 1])),
        (
            "$^(>(0 / 0))",
            json!([1, 2, 3, 4, 5]),
            json!([1, 2, 3, 4, 5]),
        ),
        (
            "$^($parseInteger('x', 'w'))",
            json!([1, 2, 3]),
            json!([3, 2, 1]),
        ),
        (
            "$^($ = 3 ? 0 / 0 : $)",
            digits.clone(),
            json!([0, 1, 2, 4, 6, 7, 8, 9, 3, 5]),
        ),
        (
            "$^(>($ = 3 ? 0 / 0 : $))",
            digits.clone(),
            json!([9, 8, 7, 6, 5, 3, 4, 2, 1, 0]),
        ),
        (
            "$^($ % 2 = 0 ? 0 / 0 : $)",
            digits,
            json!([0, 6, 4, 1, 3, 5, 7, 2, 9, 8]),
        ),
        // The next term is never reached: the first one never ties.
        (
            "$^(0 / 0, >$)",
            json!([1, 2, 3, 4, 5]),
            json!([5, 4, 3, 2, 1]),
        ),
        // Undefined keys still go last.
        (
            "$^($ = 2 ? undefined : 0 / 0)",
            json!([1, 2, 3, 4]),
            json!([4, 3, 1, 2]),
        ),
    ] {
        assert_eq!(eval(expr, input), Some(expected), "{expr}");
    }
}

#[test]
fn group_by_joins_each_group_as_jsonata_js_appends_it() {
    // The results of jsonata-js 2.2.2: a group of one item keeps it as it is, and the arrays
    // of a larger group are flattened one level.
    let tuples = json!({"items": [{"t": "a", "c": 1}, {"t": "b", "c": 2}, {"t": "a", "c": 3}]});
    for (expr, input, expected) in [
        (
            r#"${"x": $}"#,
            json!([[1, 2], [3]]),
            json!({"x": [1, 2, 3]}),
        ),
        (r#"${"x": $}"#, json!([[1, 2]]), json!({"x": [1, 2]})),
        (
            r#"${"x": $count($)}"#,
            json!([[1, 2], [3], 4]),
            json!({"x": 4}),
        ),
        (
            "items@$i{$i.t: $i.c}",
            tuples.clone(),
            json!({"a": [1, 3], "b": 2}),
        ),
        ("items#$p{t: $p}", tuples, json!({"a": [0, 2], "b": 1})),
        (
            "items@$i{$i.t: $i}",
            json!({"items": [{"t": "a", "c": [1, 2]}, {"t": "a", "c": 3}]}),
            json!({"a": [{"t": "a", "c": [1, 2]}, {"t": "a", "c": 3}]}),
        ),
    ] {
        assert_eq!(eval(expr, input), Some(expected), "{expr}");
    }
}

#[test]
fn a_tuple_binds_each_of_its_fields() {
    // A tuple's frame is made at the number of its fields: one name, a list up to eight, a
    // table past it. Each step binds one more, as jsonata-js 2.2.2 does.
    let input = json!({"v": [3, 4]});
    for k in 1..=10 {
        let names: Vec<String> = (1..=k).map(|i| format!("$x{i}")).collect();
        let steps = names
            .iter()
            .map(|n| format!("v@{n}"))
            .collect::<Vec<_>>()
            .join(".");
        let last = &names[k - 1];
        let all = names.join(", ");
        // Each tuple, in order, the first name the outermost; the filter binds them too.
        let expected: Vec<i64> = (0..1 << k)
            .filter(|t| t & 1 == 1)
            .flat_map(|t| (0..k).rev().map(move |i| 3 + (t >> i & 1)))
            .collect();
        let expr = format!("[{steps}[{last} = 4].[{all}]]");
        assert_eq!(eval(&expr, input.clone()), Some(json!(expected)), "{expr}");
        // A group's frame binds each field of the tuples it merges.
        let expr = format!("{steps}{{$string($x1): $count({last})}}");
        let half = 1 << (k - 1);
        assert_eq!(
            eval(&expr, input.clone()),
            Some(json!({"3": half, "4": half})),
            "{expr}"
        );
    }
}

#[test]
fn an_order_by_keeps_the_tuple_stream() {
    // jsonata-js gives a filter or group-by after the sort its internal tuples, unless the
    // stream has a single item (compatibility.md); jlens keeps the tuple stream.
    let input = json!({"items": [{"p": 2, "t": "a"}, {"p": 1, "t": "b"}, {"p": 3, "t": "a"}]});
    for (expr, expected) in [
        ("items@$o^($o.p){'s': $o.p}", json!({"s": [1, 2, 3]})),
        ("items@$o^($o.p){$o.t: $o.p}", json!({"b": 1, "a": [2, 3]})),
        ("items@$o.{}^(1){'s': $}", json!({"s": [{}, {}, {}]})),
        ("items@$o^(>$o.p)[$o.p > 1].$o.t", json!(["a", "a"])),
    ] {
        assert_eq!(eval(expr, input.clone()), Some(expected), "{expr}");
    }
}

#[test]
fn blocks_scope_the_variables_they_bind() {
    // Results from jsonata-js 2.2.2. A block binds in a frame of its own only when it may
    // bind; these check that the others still see, and never leak, the variables around them.
    for (expr, expected) in [
        ("($x := 1; ($x := 2); $x)", Some(json!(1))),
        ("($x := 1; ($y := 2; $x + $y))", Some(json!(3))),
        ("($x := 1; (($x := 2)); $x)", Some(json!(1))),
        ("($x := 1; ($x > 0 ? ($x := 5) : 0); $x)", Some(json!(1))),
        ("($x := 1; [1,2].($y := $); $y)", None),
        ("[1,2].($x := $; $x * 2)", Some(json!([2, 4]))),
        // Closures defined in a block.
        (
            "($f := ($n := 10; function($v){$v + $n}); [$f(1), $n])",
            Some(json!([11])),
        ),
        ("($g := (function(){ $y }); $y := 5; $g())", Some(json!(5))),
        // `$eval` binds in the frame of its caller, under any name.
        ("( $eval('$x := 5'); $x )", Some(json!(5))),
        ("( ($eval('$x := 5')); $x )", None),
        ("($e := $eval; ($e('$x := 5')); $x)", None),
        ("($e := $eval; ('$x := 5' ~> $e); $x)", None),
        ("($e := $eval; $e('($e(\"$z := 3\"); $z)'))", Some(json!(3))),
        ("($e := $eval; $e('(($e(\"$z := 3\")); $z)'))", None),
    ] {
        assert_eq!(eval(expr, json!({})), expected, "{expr}");
    }
}

#[test]
fn a_field_of_one_value_keeps_the_sequence_rules() {
    // Results from jsonata-js 2.2.2. A name evaluated on a value that is not an array is a
    // lookup; these check the sequence and singleton rules of the general path it skips.
    for (expr, input, expected) in [
        ("a", json!({"a": 1}), Some(json!(1))),
        ("a", json!({"a": null}), Some(json!(null))),
        ("a", json!({"a": [1]}), Some(json!([1]))),
        ("a", json!({"a": [1, [2, 3]]}), Some(json!([1, [2, 3]]))),
        ("a", json!({"a": []}), Some(json!([]))),
        ("a", json!({}), None),
        ("a", json!("text"), None),
        ("a[]", json!({"a": 1}), Some(json!([1]))),
        ("a[]", json!({"a": [1]}), Some(json!([1]))),
        ("[a]", json!({"a": 1}), Some(json!([1]))),
        ("[a]", json!({"a": [1]}), Some(json!([1]))),
        ("a[0]", json!({"a": 5}), Some(json!(5))),
        ("a[0]", json!({"a": [5, 6]}), Some(json!(5))),
        ("a{\"k\": $}", json!({"a": 1}), Some(json!({"k": 1}))),
        ("$count(a)", json!({"a": [1]}), Some(json!(1))),
        ("$type(a)", json!({"a": [1]}), Some(json!("array"))),
        (
            "a.(b)",
            json!({"a": [{"b": [1]}, {"b": [[2]]}]}),
            Some(json!([1, [2]])),
        ),
        (
            "items.(p)",
            json!({"items": [{"p": [1, 2]}, {"p": 3}, {}]}),
            Some(json!([1, 2, 3])),
        ),
        // Fields of constructed objects: a sequence, an array of arrays, a function.
        (
            "($o := {\"s\": [1,2].($ * 2)}; $o.(s))",
            json!({}),
            Some(json!([2, 4])),
        ),
        (
            "($o := {\"s\": [[1]]}; $o.(s))",
            json!({}),
            Some(json!([[1]])),
        ),
        (
            "($f := function(){1}; {\"f\": $f}.(f)())",
            json!({}),
            Some(json!(1)),
        ),
    ] {
        assert_eq!(eval(expr, input), expected, "{expr}");
    }
    let err = Expression::compile("items.(p * q)")
        .unwrap()
        .evaluate(
            Some(&json!({"items": [{"p": 2, "q": [5]}]})),
            &Bindings::new(),
        )
        .unwrap_err();
    assert_eq!(err.code(), "T2002");
}

#[test]
fn equality_with_a_string_literal_keeps_the_rules_of_equality() {
    // Results from jsonata-js 2.2.2. `=` and `!=` compare with a string literal without
    // building its value; these check the undefined, type and sequence rules.
    for (expr, input, expected) in [
        ("a = 'x'", json!({"a": "x"}), true),
        ("a != 'x'", json!({"a": "x"}), false),
        ("'x' = a", json!({"a": "x"}), true),
        ("'x' != a", json!({"a": "y"}), true),
        ("a = 'x'", json!({}), false),
        ("a != 'x'", json!({}), false),
        ("'x' != a", json!({}), false),
        ("a = 'x'", json!({"a": ["x"]}), false),
        ("a != 'x'", json!({"a": ["x"]}), true),
        ("a = '1'", json!({"a": 1}), false),
        ("a != '1'", json!({"a": 1}), true),
        ("a != 'x'", json!({"a": null}), true),
        ("'x' = 'x'", json!({}), true),
        ("a = 'x'[0]", json!({"a": "x"}), true),
        ("a.b = 'x'", json!({"a": [{"b": "x"}]}), true),
        ("a.b = 'x'", json!({"a": [{"b": "x"}, {"b": "x"}]}), false),
        ("a.b != 'x'", json!({"a": [{"b": "x"}, {"b": "x"}]}), true),
        ("a = ''", json!({"a": ""}), true),
        ("a = 'é'", json!({"a": "e\u{301}"}), false),
    ] {
        assert_eq!(eval(expr, input), Some(json!(expected)), "{expr}");
    }
    for source in ["$error('m') = 'x'", "'x' = $error('m')"] {
        let err = Expression::compile(source)
            .unwrap()
            .evaluate(None, &Bindings::new())
            .unwrap_err();
        assert_eq!(err.code(), "D3137", "{source}");
    }
    // The literal's depth counts as if it were evaluated: an audit takes the general path.
    for source in ["((a = 'x'))", "(('x' = a))"] {
        let expr = Expression::compile(source).unwrap();
        let input = json!({"a": "x"});
        for depth in 1..6 {
            let options = Options::new().max_depth(depth);
            let plain = expr.evaluate_with(Some(&input), &Bindings::new(), &options);
            let (audited, _) = expr.evaluate_audited(
                Some(&input),
                &Bindings::new(),
                &options,
                &jlens_core::audit::AuditOptions::full(),
            );
            assert_eq!(
                format!("{plain:?}"),
                format!("{audited:?}"),
                "{source} {depth}"
            );
        }
    }
}

/// The detail of `D2016` is the padding's length as an integer, as jsonata-js prints it
/// (jsonata-js #832, unreleased after 2.2.2); before 0.28 it was `19999999.0`.
#[test]
fn a_pad_too_long_reports_its_length_as_an_integer() {
    let err = Expression::compile(r#"$pad("x", 20000000)"#)
        .unwrap()
        .evaluate(None, &Bindings::new())
        .unwrap_err();
    assert_eq!(err.code(), "D2016");
    assert_eq!(err.detail("value"), Some(&json!(19999999)));
    assert_eq!(
        eval(r#"$length($pad("x", -10000001))"#, json!(null)),
        Some(json!(10000001))
    );
}

/// A regex bound to a variable gives each call the same matches (jsonata-js #845, unreleased
/// after 2.2.2: its calls shared one `lastIndex`).
#[test]
fn a_regex_in_a_variable_serves_each_call_alike() {
    let expr = r#"( $input := "a,b,c"; $pattern := /[,-]/;
        [$replace($input, $pattern, "|"), $replace($input, $pattern, "|"), $replace($input, $pattern, "|")] )"#;
    assert_eq!(
        eval(expr, json!(null)),
        Some(json!(["a|b|c", "a|b|c", "a|b|c"]))
    );
}

/// The flags `u` and `v`, an extension (jsonata-js rejects both with `S0202`): Unicode
/// properties for text in any script, `\w` still ASCII, and with `v` set operations on classes
/// and emoji sequences. A repeated flag, or `u` with `v`, is `S0302`, as JavaScript rejects them.
#[test]
fn the_flags_u_and_v_read_text_in_any_script() {
    let text = json!("Café Ωμέγα Москва 東京 🇧🇷 👍🏽");
    let cases = [
        (
            r"$match($, /\p{L}+/u).match",
            json!(["Café", "Ωμέγα", "Москва", "東京"]),
        ),
        (
            r"$match($, /\p{L}+/v).match",
            json!(["Café", "Ωμέγα", "Москва", "東京"]),
        ),
        (r"$match($, /\w+/u).match", json!("Caf")),
        (r"$match($, /\p{Script=Cyrillic}+/u).match", json!("Москва")),
        (
            r"$match($, /[\p{L}--\p{Script=Latin}]+/v).match",
            json!(["Ωμέγα", "Москва", "東京"]),
        ),
        (
            r"$match($, /[\p{L}&&\p{Script=Greek}]+/v).match",
            json!("Ωμέγα"),
        ),
        (
            r"$match($, /\p{RGI_Emoji}/v).(match & index)",
            json!(["🇧🇷21", "👍🏽26"]),
        ),
        (r"$match('ΣΊΣΥΦΟΣ', /σίσυφος/iu).match", json!("ΣΊΣΥΦΟΣ")),
        (
            r"$replace($, /[\p{L}--[a-z]]/v, '_')",
            json!("_af_ _____ ______ __ 🇧🇷 👍🏽"),
        ),
    ];
    for (expr, expected) in cases {
        assert_eq!(eval(expr, text.clone()), Some(expected), "{expr}");
    }
    for expr in [
        "$match('a', /a/ii)",
        "$match('a', /a/uv)",
        "$match('a', /a/vmu)",
        "$match('a', /[a-]/v)",
        r"$match('a', /\p{RGI_Emoji}/u)",
    ] {
        let err = Expression::compile(expr)
            .and_then(|e| e.evaluate(None, &Bindings::new()))
            .unwrap_err();
        assert_eq!(err.code(), "S0302", "{expr}");
    }
    assert_eq!(
        eval("$match('a-', /[a-]+/u).match", json!(null)),
        Some(json!("a-"))
    );
}

#[test]
fn size_caps_stop_huge_values() {
    let code = |source: &str, options: &Options| {
        Expression::compile(source)
            .unwrap()
            .evaluate_with(None, &Bindings::new(), options)
            .err()
            .map(|e| e.code().to_owned())
    };
    // 30 times a string of 10^7 bytes is over the default 256 MiB.
    let huge = r#"($s := $pad("", 10000000, "x"); $join([1..30].$s))"#;
    assert_eq!(code(huge, &Options::new()).as_deref(), Some("U1001"));
    // An `M` per thousand: this Roman numeral would take 10^18 bytes.
    assert_eq!(
        code("$formatInteger(1e21, 'I')", &Options::new()).as_deref(),
        Some("U1001")
    );

    let strings = Options::new().max_string_length(10);
    assert_eq!(code(r#""abcde" & "fghij""#, &strings), None);
    assert_eq!(
        code(r#""abcde" & "fghijk""#, &strings).as_deref(),
        Some("U1001")
    );
    assert_eq!(
        code(r#"$uppercase("abcdefghijk")"#, &strings).as_deref(),
        Some("U1001")
    );

    let arrays = Options::new().max_array_length(3);
    for ok in ["[1, 2, 3]", "[[1, 2], [3, 4]]", "[1..3]"] {
        assert_eq!(code(ok, &arrays), None, "{ok}");
    }
    for huge in [
        "[1, 2, 3, 4]",
        "[[1], [2], [3], [4]]",
        "[1..4]",
        "($x := [1, 2]; $x.$x)",
        "($x := [1, 2]; $x@$a.$x@$b)",
        "$append([1, 2], [3, 4])",
    ] {
        assert_eq!(code(huge, &arrays).as_deref(), Some("U1001"), "{huge}");
    }
}

#[test]
fn the_result_size_cap_counts_shared_values() {
    let code = |source: &str, options: &Options| {
        Expression::compile(source)
            .unwrap()
            .evaluate_with(None, &Bindings::new(), options)
            .err()
            .map(|e| e.code().to_owned())
    };
    // 2^30 copies of a 1,000-item array, built from 30 objects of two fields.
    let nest = "$d := function($x, $n) { $n = 0 ? $x : $d({'l': $x, 'r': $x}, $n - 1) }";
    let huge = format!("({nest}; $d([1..1000], 30))");
    assert_eq!(code(&huge, &Options::new()).as_deref(), Some("U1001"));

    let caps = Options::new()
        .max_result_size(1 << 20)
        .max_string_length(1 << 20)
        .max_array_length(1 << 20);
    for huge in [
        format!("({nest}; [$d([1..1000], 30)])"),
        format!("({nest}; $string($d([1..1000], 30)))"),
        format!("({nest}; $d([1..1000], 30) & '')"),
        format!("({nest}; $d([1..1000], 30) ~> |$| {{'b': 2}}|)"),
        format!("({nest}; $count($d([1..1000], 30).**))"),
        "($a := [1..2000]; $count({'a': $map([1..2000], function() { $a })}.*))".into(),
    ] {
        assert_eq!(code(&huge, &caps).as_deref(), Some("U1001"), "{huge}");
    }
    // A huge value that is not part of the result is fine, and an error that shows one leaves
    // it out.
    let inner = format!("({nest}; $d([1..1000], 30) ~> $type)");
    assert_eq!(code(&inner, &caps), None);
    let shown = format!("({nest}; $d([1..1000], 30) + 1)");
    assert_eq!(code(&shown, &caps).as_deref(), Some("T2001"));

    // The size is that of the compact JSON text: `"abcdefgh"` is 10 bytes.
    let small = Options::new().max_result_size(10);
    for ok in [r#""abcdefgh""#, "[1, 2, 3, 4]", "{'a': [1]}", "[1, $x, 3]"] {
        assert_eq!(code(ok, &small), None, "{ok}");
    }
    for huge in [
        r#""abcdefghi""#,
        "[1, 2, 3, 4, 5]",
        "{'abc': [1]}",
        "[12, null, 3]",
    ] {
        assert_eq!(code(huge, &small).as_deref(), Some("U1001"), "{huge}");
    }
}

#[test]
fn the_result_size_cap_limits_host_function_arguments() {
    use jlens_core::HostFunction;
    let bindings = Bindings::new()
        .function(
            "count",
            HostFunction::new(2, |args, _| Ok(Some(json!(args.len())))),
        )
        .function(
            "context",
            HostFunction::new(0, |_, call| Ok(Some(json!(call.input().is_some())))),
        );
    let small = Options::new().max_result_size(10);
    let code = |source: &str| {
        Expression::compile(source)
            .unwrap()
            .evaluate_with(None, &bindings, &small)
            .err()
            .map(|e| e.code().to_owned())
    };
    assert_eq!(code("$count([1, 2], [3, 4])"), None);
    assert_eq!(code("$count([1, 2], [3, 4, 5])").as_deref(), Some("U1001"));
    assert_eq!(code("($s := 'abcdefgh'; $s.$context())"), None);
    assert_eq!(
        code("($s := 'abcdefghi'; $s.$context())").as_deref(),
        Some("U1001")
    );
}

#[test]
fn documents_are_reusable_and_unchanged_by_evaluation() {
    let text = r#"{"items": [{"code": "a", "n": 1}, {"code": "b", "n": 2}], "k": 1, "k": 2}"#;
    let parsed = Document::parse(text).unwrap();
    let converted = Document::new(&serde_json::from_str(text).unwrap());
    assert_eq!(parsed.to_json(), converted.to_json());
    // A repeated key keeps the last value, as JSON.parse does.
    assert_eq!(parsed.to_json()["k"], json!(2));
    assert!(Document::parse("{} trailing").is_err());

    let before = parsed.to_json();
    let bindings = Bindings::new();
    let options = Options::default();
    for source in [
        "$ ~> |items|{'n': n * 10}, ['code']|",
        "items#$i.{'i': $i, 'code': code}",
        "items@$it.$it.code",
        "$merge([items[0], {'extra': true}])",
    ] {
        let expr = Expression::compile(source).unwrap();
        let first = expr
            .evaluate_document(&parsed, &bindings, &options)
            .unwrap();
        let again = expr
            .evaluate_document(&parsed, &bindings, &options)
            .unwrap();
        assert_eq!(first, again, "{source}");
        assert_eq!(
            first,
            expr.evaluate(Some(&before), &bindings).unwrap(),
            "{source}"
        );
        assert_eq!(parsed.to_json(), before, "{source} modified the document");
    }
}

#[test]
fn documents_serialized_from_rust_match_serde_json() {
    use jlens_core::Document;
    use serde::Serialize;
    use serde::ser::{SerializeMap, Serializer};
    use std::collections::BTreeMap;

    #[derive(Serialize)]
    enum Shape {
        Empty,
        Circle(f64),
        Point(i32, i32),
        Rect { w: u8, h: u8 },
    }
    #[derive(Serialize)]
    struct Id(u32);
    #[derive(Serialize)]
    struct Unit;
    #[derive(Serialize)]
    struct All<'a> {
        name: &'a str,
        letter: char,
        small: f32,
        nan: f64,
        big: u64,
        wide: i128,
        id: Id,
        unit: Unit,
        nothing: Option<u8>,
        pair: (bool, &'a str),
        shapes: Vec<Shape>,
        by_number: BTreeMap<i32, &'a str>,
        by_flag: BTreeMap<bool, u8>,
        by_shape: BTreeMap<&'a str, Shape>,
        json: Value,
    }
    /// A map that repeats a key, and one keyed by floats.
    struct Entries<K>(Vec<(K, u8)>);
    impl<K: Serialize> Serialize for Entries<K> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            let mut map = s.serialize_map(None)?;
            for (k, v) in &self.0 {
                map.serialize_entry(k, v)?;
            }
            map.end()
        }
    }

    let same = |value: &dyn erased::Erased| {
        let json = value.to_value();
        let document = value.serialized();
        match (json, document) {
            (Ok(json), Ok(document)) => {
                assert_eq!(document.to_json(), Document::new(&json).to_json())
            }
            (Err(_), Err(_)) => {}
            (json, document) => panic!("{json:?} against {document:?}"),
        }
    };
    let all = All {
        name: "pen",
        letter: '𝄞',
        small: 0.1,
        nan: f64::NAN,
        big: u64::MAX,
        wide: -(1 << 60),
        id: Id(7),
        unit: Unit,
        nothing: None,
        pair: (true, "x"),
        shapes: vec![
            Shape::Empty,
            Shape::Circle(1.5),
            Shape::Point(1, -2),
            Shape::Rect { w: 2, h: 3 },
        ],
        by_number: BTreeMap::from([(-1, "a"), (10, "b")]),
        by_flag: BTreeMap::from([(false, 0), (true, 1)]),
        by_shape: BTreeMap::from([("c", Shape::Circle(2.0))]),
        json: json!({"z": 1, "a": [null, {"k": "v"}]}),
    };
    same(&all);
    same(&Entries(vec![("k", 1), ("j", 2), ("k", 3)]));
    same(&Entries(vec![(1.5, 1), (1e21, 2), (-0.0, 3)]));
    // Errors: a number out of range, keys that JSON has no string for.
    same(&(1u128 << 64));
    same(&Entries(vec![(f64::NAN, 1)]));
    same(&Entries(vec![((), 1)]));
    same(&Entries(vec![(vec![1], 1)]));
    // Past 1,000 levels, the document fails where `serde_json` does not.
    let mut deep = json!(1);
    for _ in 0..1001 {
        deep = json!([deep]);
    }
    assert!(Document::from_serialize(&deep).is_err());

    mod erased {
        use jlens_core::Document;
        pub trait Erased {
            fn to_value(&self) -> serde_json::Result<serde_json::Value>;
            fn serialized(&self) -> serde_json::Result<Document>;
        }
        impl<T: serde::Serialize> Erased for T {
            fn to_value(&self) -> serde_json::Result<serde_json::Value> {
                serde_json::to_value(self)
            }
            fn serialized(&self) -> serde_json::Result<Document> {
                Document::from_serialize(self)
            }
        }
    }
}

#[test]
fn variables_bind_to_documents() {
    use jlens_core::{Document, HostFunction};
    let doc = Document::parse(r#"{"items": [{"n": 1}, {"n": 2}], "a/b": {"~": true}}"#).unwrap();
    // JSON Pointers, as `serde_json` reads them.
    let part = |pointer: &str| doc.pointer(pointer).map(|d| d.to_json());
    assert_eq!(part(""), Some(doc.to_json()));
    assert_eq!(part("/items/1"), Some(json!({"n": 2})));
    assert_eq!(part("/items/1/n"), Some(json!(2)));
    assert_eq!(part("/a~1b/~0"), Some(json!(true)));
    for missing in [
        "items",
        "/item",
        "/items/2",
        "/items/01",
        "/items/+1",
        "/items/-",
        "/items/1/n/0",
    ] {
        assert_eq!(part(missing), None, "{missing}");
    }

    let options = Options::default();
    let second = doc.pointer("/items/1").unwrap();
    let first = doc.pointer("/items/0").unwrap();
    let bindings = Bindings::new()
        .var("self", 10)
        .function("other", HostFunction::new(0, |_, _| Ok(Some(json!(0)))));
    let eval = |source: &str, variables: &[(&str, &Document)]| {
        Expression::compile(source)
            .unwrap()
            .evaluate_document_with(&doc, variables, &bindings, &options)
            .unwrap()
    };
    assert_eq!(
        eval("$self.n + $count(items)", &[("self", &second)]),
        Some(json!(4))
    );
    // A document takes the place of a variable or a function of `bindings`, and a later pair
    // the place of an earlier one.
    assert_eq!(eval("$self", &[]), Some(json!(10)));
    assert_eq!(eval("$other.n", &[("other", &first)]), Some(json!(1)));
    assert_eq!(
        eval("$self.n", &[("self", &first), ("self", &second)]),
        Some(json!(2))
    );
    // A transform copies a bound document before it changes it.
    assert_eq!(
        eval("$self ~> |$|{'n': 5}|", &[("self", &second)]),
        Some(json!({"n": 5}))
    );
    assert_eq!(second.to_json(), json!({"n": 2}));
    assert_eq!(part("/items/1"), Some(json!({"n": 2})));
}

/// Against a document, a variable of the bindings is converted as far as the expression reads
/// it, and an expression cannot tell: what it gives is what `evaluate_with` gives.
#[test]
fn variables_against_a_document_give_what_they_give_elsewhere() {
    use jlens_core::{Document, Error, HostFunction};
    let unit = json!({
        "z": 1,
        "hp": 40,
        "tags": ["a", "b"],
        "stats": [{"str": 3, "dex": 1}, [{"str": 5}], 7],
        "none": {},
        "deep": {"a": {"b": null, "c": 2}},
    });
    let values = Bindings::new()
        .var("self", unit.clone())
        .var("other", 5)
        .var("list", json!([{"a": 1, "b": 2}, {"a": 3}]));
    let echo = HostFunction::new(1, |args, _| Ok(Some(json!({"host": args}))));
    let hidden = |name: &str, value: Value| values.clone().var(name, value);
    let host = |name: &str| values.clone().function(name, echo.clone());
    let cases = [
        // Fields, items, and what needs no field.
        ("$self.hp", values.clone()),
        ("$self.missing", values.clone()),
        ("$self.stats.str", values.clone()),
        ("$self.deep.a.b", values.clone()),
        ("$list[a = 3].a", values.clone()),
        ("$list[1]", values.clone()),
        ("$count($self.stats)", values.clone()),
        ("$exists($self.none)", values.clone()),
        ("$type($self.deep)", values.clone()),
        ("$self.none ? 1 : 2", values.clone()),
        ("$boolean($self.deep)", values.clone()),
        ("$self.none ?? 1", values.clone()),
        // What reads the variable whole, in the order of its fields.
        ("$self", values.clone()),
        ("$keys($self)", values.clone()),
        ("$string($self.deep)", values.clone()),
        ("$self.*", values.clone()),
        ("$self.**.b", values.clone()),
        ("{'a': $self.deep}", values.clone()),
        ("$merge([$self, {'hp': 1}]).hp", values.clone()),
        // Functions reach what they close over, however they are called.
        (
            "($f := function($k){ $self.hp + $k }; $f($other))",
            values.clone(),
        ),
        (
            "$map([1, 2], function($v){ $self.deep.a.c + $v })",
            values.clone(),
        ),
        ("($g := $string; $g($self.deep))", values.clone()),
        // A variable that only some branch reads, and one that an inner block hides.
        ("$other > 9 ? $self.hp : 0", values.clone()),
        ("($self := {'hp': 1}; $self.hp)", values.clone()),
        (
            "($f := function(){ $other }; $other := 1; $f())",
            values.clone(),
        ),
        ("($other > 9 ? $self := 1 : 0; $self.hp)", values.clone()),
        (
            "($f := function(){ $other > 9 ? $self := 1 : $self.hp }; $f())",
            values.clone(),
        ),
        // A transform and the variable it copies.
        ("$self ~> |deep.a|{'c': 3}|", values.clone()),
        // Errors, with the value they show.
        ("$self.tags + 1", values.clone()),
        ("$self.deep.a.c.d()", values.clone()),
        ("$sum($self.deep)", values.clone()),
        // `$eval` can read any variable.
        ("$eval('$self.hp + $other')", values.clone()),
        ("($e := $eval; $e('$self.hp'))", values.clone()),
        // A binding that hides a built-in the expression names.
        ("$string.a", hidden("string", json!({"a": 1}))),
        ("$count($self)", host("count")),
        ("($f := $count; $f($self))", host("count")),
        ("$self.hp ~> $string", host("string")),
        ("$count($self)", hidden("count", json!(1))),
        ("count()", hidden("count", json!(true))),
        ("$self ~> |deep|{'x': 1}|", hidden("clone", json!(1))),
    ];
    let input = json!({"hp": 1, "other": 2});
    let outcome = |result: Result<Option<Value>, Error>| {
        result.map_err(|e| (e.code().to_owned(), e.to_string()))
    };
    let options = Options::default();
    for (source, bindings) in &cases {
        let expr = Expression::compile(source).unwrap();
        let elsewhere = expr.evaluate_with(Some(&input), bindings, &options);
        let document = expr.evaluate_document(&Document::new(&input), bindings, &options);
        assert_eq!(outcome(document), outcome(elsewhere), "{source}");
        // Twice, as the first evaluation runs the analysis.
        let none = expr.evaluate_with(None, bindings, &options);
        for _ in 0..2 {
            let document = expr.evaluate_document(&Document::undefined(), bindings, &options);
            assert_eq!(outcome(document), outcome(none.clone()), "{source}");
        }
    }

    // A name of `variables` takes the place of a binding that hides nothing, or hides a built-in.
    let number = Document::new(&json!({"a": 1}));
    let evaluate = |source: &str, name: &str| {
        Expression::compile(source)
            .unwrap()
            .evaluate_document_with(
                &Document::undefined(),
                &[(name, &number)],
                &values,
                &options,
            )
            .map_err(|e| e.code().to_owned())
    };
    assert_eq!(evaluate("$self.a", "self"), Ok(Some(json!(1))));
    assert_eq!(evaluate("$string.a", "string"), Ok(Some(json!(1))));
    assert_eq!(
        evaluate("$string($self.hp)", "string"),
        Err("T1006".to_owned())
    );
}

#[test]
fn an_undefined_document_stands_for_no_input() {
    let options = Options::default();
    let undefined = Document::undefined();
    let eval = |source: &str| {
        Expression::compile(source)
            .unwrap()
            .evaluate_document(&undefined, &Bindings::new(), &options)
            .unwrap()
    };
    assert_eq!(eval("$"), None);
    assert_eq!(eval("a"), None);
    assert_eq!(eval("$exists($$)"), Some(json!(false)));
    assert_eq!(eval("1 + 2"), Some(json!(3)));
    assert_eq!(undefined.to_json(), json!(null));
    assert_eq!(
        undefined.pointer("").map(|d| d.to_json()),
        Some(json!(null))
    );
    assert!(undefined.pointer("/a").is_none());
    // As a variable.
    let expr = Expression::compile("$exists($x)").unwrap();
    let result = expr.evaluate_document_with(
        &Document::new(&json!({})),
        &[("x", &undefined)],
        &Bindings::new(),
        &options,
    );
    assert_eq!(result.unwrap(), Some(json!(false)));
}

#[test]
fn compiled_expressions_can_be_shared_between_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Expression>();
    let expr = std::sync::Arc::new(Expression::compile("$sum(items)").unwrap());
    let handles: Vec<_> = (0..4)
        .map(|i| {
            let expr = expr.clone();
            std::thread::spawn(move || {
                expr.evaluate(Some(&json!({"items": [i, 1]})), &Bindings::new())
                    .unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(
        results,
        [json!(1), json!(2), json!(3), json!(4)].map(Some).to_vec()
    );
}

#[test]
fn transforms_cannot_create_circular_objects() {
    // Found by the property tests: this used to overflow the stack when converting the result.
    for source in [
        "$ ~> |items|{'p': $}|",
        "$ ~> |$|{'self': $}|",
        "$ ~> |items|{'wrapped': {'again': [$]}}|",
    ] {
        let expr = Expression::compile(source).unwrap();
        let err = expr
            .evaluate(Some(&json!({"items": [{"a": 1}]})), &Bindings::new())
            .unwrap_err();
        assert_eq!(err.code(), "U1001", "{source}");
    }
    // Copying a value that merely looks the same is fine.
    let expr = Expression::compile("$ ~> |items|{'copy': $clone($)}|").unwrap();
    assert_eq!(
        expr.evaluate(Some(&json!({"items": [{"a": 1}]})), &Bindings::new())
            .unwrap(),
        Some(json!({"items": [{"a": 1, "copy": {"a": 1}}]}))
    );
}

/// A transform changes a copy: its input, a value it was given twice and the values bound to
/// variables stay as they were, and the copy has the values of jsonata-js, which copies by
/// serializing and parsing (results checked against jsonata-js 2.2.2).
#[test]
fn transforms_change_a_copy_alone() {
    let input = json!({"items": [{"a": 1, "b": {"c": 2}}, {"a": 0.1, "b": {"c": 3}}], "n": 0.30000000000000004});
    let document = Document::new(&input);
    for (source, expected) in [
        (
            "($t := $ ~> |items|{'x': 1}|; [$t, $])",
            json!([
                {"items": [{"a": 1, "b": {"c": 2}, "x": 1}, {"a": 0.1, "b": {"c": 3}, "x": 1}], "n": 0.3},
                input,
            ]),
        ),
        (
            "($o := {'a': 1}; $t := $o ~> |$|{'a': 2}|; [$o, $t])",
            json!([{"a": 1}, {"a": 2}]),
        ),
        (
            "($o := {'a': 1}; {'x': $o, 'y': $o} ~> |x|{'a': 2}|)",
            json!({"x": {"a": 2}, "y": {"a": 1}}),
        ),
        (
            "$ ~> |items|{'n': $ ~> |b|{'d': 4}|}|",
            json!({"items": [
                {"a": 1, "b": {"c": 2}, "n": {"a": 1, "b": {"c": 2, "d": 4}}},
                {"a": 0.1, "b": {"c": 3}, "n": {"a": 0.1, "b": {"c": 3, "d": 4}}},
            ], "n": 0.3}),
        ),
        (
            "($t := |items.b|{'c': c + 10}|; $ ~> $t ~> $t)",
            json!({"items": [{"a": 1, "b": {"c": 22}}, {"a": 0.1, "b": {"c": 23}}], "n": 0.3}),
        ),
        (
            "$ ~> |items|{}, ['b']|",
            json!({"items": [{"a": 1}, {"a": 0.1}], "n": 0.3}),
        ),
        (
            "($f := function(){1}; {'f': $f, 'g': [$f, 1], 'n': 0.1 + 0.2, 'z': -0} ~> |$|{}|)",
            json!({"f": "", "g": ["", 1], "n": 0.3, "z": 0}),
        ),
    ] {
        let expr = Expression::compile(source).unwrap();
        let result = expr.evaluate(Some(&input), &Bindings::new()).unwrap();
        assert_eq!(result, Some(expected.clone()), "{source}");
        let result = expr
            .evaluate_document(&document, &Bindings::new(), &Options::default())
            .unwrap();
        assert_eq!(result, Some(expected), "{source} against a document");
        assert_eq!(document.to_json(), input, "{source}: the document changed");
    }
}

#[test]
fn missing_arguments_behave_as_in_javascript() {
    // Found by `cargo xtask differential`. jsonata-js computes `Math.pow(1, undefined)`, which is
    // NaN.
    for source in [
        "$power(1, nothing)",
        "$power(-1, nothing)",
        "$power(2, nothing)",
    ] {
        let err = Expression::compile(source)
            .unwrap()
            .evaluate(Some(&json!({})), &Bindings::new())
            .unwrap_err();
        assert_eq!(err.code(), "D3061", "{source}");
    }
}

#[test]
fn match_offsets_count_utf16_units_from_where_the_last_match_ended() {
    let backwards = "function($s){ {'start': 3, 'match': 'b', 'next': function(){ \
                     {'start': 2, 'match': 'a', 'next': function(){ () }} }} }";
    for (source, expected) in [
        ("$match('𝄞a𝄞b', /[ab]/).index", json!([2, 5])),
        ("$replace('𝄞a𝄞b𝄞', /[ab]/, 'x')", json!("𝄞x𝄞x𝄞")),
        ("$split('𝄞a𝄞b𝄞', /[ab]/)", json!(["𝄞", "𝄞", "𝄞"])),
        // `next` asked twice gives the same match.
        (
            "($m := /b/('𝄞b𝄞b'); [$m.next().start, $m.next().start, $m.next().end])",
            json!([5, 5, 6]),
        ),
        // A search from an offset, one inside a pair and one past the text.
        ("/b/('𝄞b𝄞b', 3).start", json!(5)),
        ("/b/('𝄞b𝄞b', 1).start", json!(2)),
        // A matcher of the expression's own may go backwards.
        (
            &format!("$replace('𝄞abc', {backwards}, 'x')"),
            json!("𝄞axabxbc"),
        ),
    ] {
        assert_eq!(eval(source, json!(null)), Some(expected), "{source}");
    }
    assert_eq!(eval("/b/('𝄞b𝄞b', 100)", json!(null)), None);
}

#[test]
fn a_negative_limit_is_checked_before_a_missing_pattern() {
    // Found by `cargo xtask differential`: jsonata-js checks the limit first.
    for (source, code) in [
        ("$split('abc', nothing, -2.5)", "D3020"),
        ("$replace('abc', nothing, 'x', -1)", "D3011"),
    ] {
        let err = Expression::compile(source)
            .unwrap()
            .evaluate(Some(&json!(null)), &Bindings::new())
            .unwrap_err();
        assert_eq!(err.code(), code, "{source}");
    }
    assert_eq!(eval("$split('abc', nothing, 1)", json!(null)), None);
    assert_eq!(eval("$replace('abc', nothing, 'x', 1)", json!(null)), None);
}

#[test]
fn a_missing_replacement_removes_what_the_pattern_finds() {
    // Found by `cargo xtask differential`: jsonata-js writes "undefined" for a string pattern
    // (`"aundefined"`) and fails with a `TypeError` for a regex; jlens removes the matches of
    // either (compatibility.md).
    for (source, expected) in [
        ("$replace('abab', 'b', nothing)", json!("aa")),
        ("$replace('abab', 'b', nothing, 1)", json!("aab")),
        ("$replace('abab', /b/, nothing)", json!("aa")),
        ("$replace('abab', /b/, nothing, 1)", json!("aab")),
        (
            "$replace('abab', function($s){ /b/($s) }, nothing)",
            json!("aa"),
        ),
        ("$replace('abab', 'c', nothing)", json!("abab")),
        // The input as the string, the pattern and the missing replacement after it.
        ("$replace('ab', nothing)", json!("")),
    ] {
        assert_eq!(eval(source, json!("ab")), Some(expected), "{source}");
    }
}

/// Past the items compared one by one, each kind of value keeps what `=` tells apart: functions
/// by identity, numbers by value, whatever the order of an object's fields.
#[test]
fn distinct_past_a_few_items_compares_as_equality_does() {
    for (source, expected) in [
        ("$distinct($append([1..40], [1..40]))[-1]", json!(40)),
        (
            "($f := function($x){ $x }; $g := function($x){ $x };
              $count($distinct([1..40, $f, $g, $f, $g])))",
            json!(42),
        ),
        (
            "$count($distinct($map([1..40], function($i){
              $i % 2 = 0 ? {'a': $i % 20, 'b': 1} : {'b': 1, 'a': $i % 20} })))",
            json!(20),
        ),
        (
            "$count($distinct($map([1..40], function($i){ 0 * ($i % 2 - 0.5) })))",
            json!(1),
        ),
    ] {
        assert_eq!(eval(source, json!(null)), Some(expected), "{source}");
    }
}

#[test]
fn a_builtin_is_one_value_per_evaluation() {
    assert_eq!(eval("$sum = $sum", json!(null)), Some(json!(true)));
    assert_eq!(
        eval("($f := $sum; (function() { $f = $sum })())", json!(null)),
        Some(json!(true))
    );
    assert_eq!(eval("$sum = $count", json!(null)), Some(json!(false)));
}

#[test]
fn the_clock_is_read_once_per_evaluation() {
    // Without `Options::now` and `seed`, the timestamp and the seed come from one clock read,
    // the first time the evaluation needs it.
    let result = eval(
        "[$now() = $now(), $millis() = $millis(), $toMillis($now()) = $millis(), $random() != $random()]",
        Value::Null,
    );
    assert_eq!(result, Some(json!([true, true, true, true])));
}

#[test]
fn the_input_is_found_as_a_variable_unless_it_is_hidden() {
    let input = json!({"a": 1});
    assert_eq!(eval("[1].$$.a", input.clone()), Some(json!(1)));
    assert_eq!(eval("($$ := 5; $$)", input.clone()), Some(json!(5)));
    assert_eq!(
        eval("($f := function(){ $$.a }; {'a': 2}.$f())", input.clone()),
        Some(json!(1))
    );
    // A binding named `$` does not hide the input, as before.
    let bindings = Bindings::new().var("$", 7);
    let result = Expression::compile("$$")
        .unwrap()
        .evaluate(Some(&input), &bindings)
        .unwrap();
    assert_eq!(result, Some(input));
    let result = Expression::compile("$$")
        .unwrap()
        .evaluate(None, &bindings)
        .unwrap();
    assert_eq!(result, None);
}
