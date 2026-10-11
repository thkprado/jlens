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
