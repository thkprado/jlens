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
    let options = Options {
        max_depth: Some(500),
        ..Options::default()
    };
    let err = deep
        .evaluate_with(None, &Bindings::new(), &options)
        .unwrap_err();
    assert_eq!(err.code(), "D1011");

    let forever = Expression::compile("($f := function($n){ $f($n + 1) }; $f(0))").unwrap();
    let options = Options {
        timeout: Some(Duration::from_millis(50)),
        ..Options::default()
    };
    let err = forever
        .evaluate_with(None, &Bindings::new(), &options)
        .unwrap_err();
    assert_eq!(err.code(), "D1012");
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
