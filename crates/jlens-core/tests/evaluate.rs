//! End-to-end evaluation through the public API.

use std::time::Duration;

use jlens_core::{Bindings, Expression, Options};
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
    use jlens_core::Document;
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
fn a_builtin_is_one_value_per_evaluation() {
    assert_eq!(eval("$sum = $sum", json!(null)), Some(json!(true)));
    assert_eq!(
        eval("($f := $sum; (function() { $f = $sum })())", json!(null)),
        Some(json!(true))
    );
    assert_eq!(eval("$sum = $count", json!(null)), Some(json!(false)));
}
