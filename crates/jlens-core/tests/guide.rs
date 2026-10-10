//! The code of the Rust crate guide (`docs/src/guides/crate.md`), which includes the parts between
//! the `ANCHOR` comments. Keeping it here means every snippet of the guide compiles and runs.

use std::error::Error;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[test]
fn quick_start() -> Result<()> {
    // ANCHOR: quick_start
    use jlens_core::{Bindings, Expression};
    use serde_json::json;

    let expr = Expression::compile("$sum(items[type = $kind].(price * qty))")?;
    let order = json!({"items": [
        {"type": "book", "price": 12.5, "qty": 2},
        {"type": "pen", "price": 1.5, "qty": 10}
    ]});
    let bindings = Bindings::new().var("kind", "book");
    let total = expr.evaluate(Some(&order), &bindings)?;
    assert_eq!(total, Some(json!(25)));
    // ANCHOR_END: quick_start
    Ok(())
}

#[test]
fn undefined_results() -> Result<()> {
    use jlens_core::{Bindings, Expression};
    use serde_json::json;
    // ANCHOR: undefined
    let expr = Expression::compile("customer.email")?;
    // No input document: `None`. A result of `None`: the expression produced nothing.
    assert_eq!(expr.evaluate(None, &Bindings::new())?, None);
    assert_eq!(
        expr.evaluate(Some(&json!({"customer": {}})), &Bindings::new())?,
        None
    );
    // ANCHOR_END: undefined
    Ok(())
}

#[test]
fn options() -> Result<()> {
    use jlens_core::{Bindings, Expression};
    // ANCHOR: options
    use jlens_core::Options;
    use std::time::Duration;

    let options = Options::new()
        .timeout(Duration::from_millis(100)) // D1012 when exceeded
        .max_depth(500); // D1011 when exceeded
    let runaway = Expression::compile("($f := function($n) { $f($n + 1) }; $f(0))")?;
    let err = runaway
        .evaluate_with(None, &Bindings::new(), &options)
        .unwrap_err();
    assert_eq!(err.code(), "D1012");
    // ANCHOR_END: options
    Ok(())
}

#[test]
fn reproducible() -> Result<()> {
    use jlens_core::{Bindings, Expression, Options};
    use serde_json::json;
    // ANCHOR: reproducible
    let options = Options::new()
        .now(1_700_000_000_000) // $now(), $millis()
        .seed(42); // $random(), $shuffle()
    let expr = Expression::compile("{'at': $now(), 'order': $shuffle([1, 2, 3, 4, 5])}")?;
    let bindings = Bindings::new();
    let first = expr.evaluate_with(None, &bindings, &options)?.unwrap();
    assert_eq!(first["at"], json!("2023-11-14T22:13:20.000Z"));
    // Same expression, input and options: the same result.
    assert_eq!(expr.evaluate_with(None, &bindings, &options)?, Some(first));
    // ANCHOR_END: reproducible
    Ok(())
}

#[test]
fn errors() {
    use jlens_core::{Bindings, Expression};
    use serde_json::json;
    // ANCHOR: errors
    let err = Expression::compile("Account.Order[0;]").unwrap_err();
    assert_eq!(err.code(), "S0202");
    assert_eq!(err.message(), r#"Expected "]", got ";""#);
    assert_eq!(err.position(), Some(16)); // UTF-16 offset, as in jsonata-js
    let span = err.span().unwrap(); // byte range of the offending token
    assert_eq!((span.start, span.end), (15, 16));
    assert_eq!(err.token().as_deref(), Some(";"));

    let expr = Expression::compile("name + 1").unwrap();
    let err = expr
        .evaluate(Some(&json!({"name": "x"})), &Bindings::new())
        .unwrap_err();
    assert_eq!(err.code(), "T2001");
    assert_eq!(err.value(), Some(&json!("x")));
    // `Display` gives the code, the message and the position.
    assert_eq!(
        err.to_string(),
        r#"T2001: The left side of the "+" operator must evaluate to a number (at position 6)"#
    );
    // ANCHOR_END: errors
}

#[test]
fn host_functions() -> Result<()> {
    use jlens_core::{Bindings, Expression};
    use serde_json::json;
    // ANCHOR: host_functions
    use jlens_core::{Error, HostFunction};

    // `$shipping(weight, region?)`: the region defaults to the context value's `region`.
    let shipping = HostFunction::with_signature("<ns?:n>", |args, call| {
        let weight = args[0].as_ref().and_then(|w| w.as_f64()).unwrap_or(0.0);
        let region = match &args[1] {
            Some(region) => region.clone(),
            None => call
                .input()
                .map_or(json!(null), |order| order["region"].clone()),
        };
        let rate = match region.as_str() {
            Some("north") => 1.5,
            Some("south") => 2.0,
            _ => return Err(Error::new("SHIP01").with("message", format!("No rate for {region}"))),
        };
        Ok(Some(json!(weight * rate)))
    })?;
    let bindings = Bindings::new().function("shipping", shipping);

    let expr = Expression::compile("$shipping($sum(items.weight))")?;
    let order = json!({"region": "south", "items": [{"weight": 2}, {"weight": 1.5}]});
    assert_eq!(expr.evaluate(Some(&order), &bindings)?, Some(json!(7)));

    let err = Expression::compile("$shipping(3, 'west')")?
        .evaluate(None, &bindings)
        .unwrap_err();
    assert_eq!(err.code(), "SHIP01");
    assert_eq!(err.message(), r#"No rate for "west""#);
    assert_eq!(err.position(), Some(10)); // the call, as for built-in functions
    // ANCHOR_END: host_functions
    Ok(())
}

#[test]
fn documents() -> Result<()> {
    use jlens_core::{Bindings, Expression, Options};
    use serde_json::json;
    // ANCHOR: documents
    use jlens_core::Document;

    let rules = [
        ("minimum total", "$sum(items.(price * qty)) >= 20"),
        ("at most 50 items", "$sum(items.qty) <= 50"),
        ("no pens", "$not('pen' in items.type)"),
    ];
    let compiled: Vec<_> = rules
        .iter()
        .map(|(name, source)| Ok((*name, Expression::compile(source)?)))
        .collect::<Result<_>>()?;

    let order = Document::parse(
        r#"{"items": [{"type": "book", "price": 12.5, "qty": 2},
                      {"type": "pen", "price": 1.5, "qty": 10}]}"#,
    )?;
    let failed: Vec<&str> = compiled
        .iter()
        .filter(|(_, rule)| {
            let outcome = rule.evaluate_document(&order, &Bindings::new(), &Options::default());
            outcome.ok().flatten() != Some(json!(true))
        })
        .map(|(name, _)| *name)
        .collect();
    assert_eq!(failed, ["no pens"]);
    // ANCHOR_END: documents
    Ok(())
}

#[test]
fn audit() -> Result<()> {
    use jlens_core::{Bindings, Expression, Options};
    use serde_json::json;
    // ANCHOR: audit
    use jlens_core::audit::{AuditOptions, explain};

    let rule = Expression::compile(
        r#"/** @label "Big order" @message "Orders must total at least 100." */
           $sum(items.(price * qty)) >= 100"#,
    )?;
    let order = json!({"items": [{"price": 12.5, "qty": 2}]});
    let (result, audit) = rule.evaluate_audited(
        Some(&order),
        &Bindings::new(),
        &Options::default(),
        &AuditOptions::decisions(),
    );
    assert_eq!(result?, Some(json!(false)));
    let explanation = explain(&audit);
    assert_eq!(explanation.result, "false");
    assert_eq!(
        explanation.lines[0].text,
        "Big order — $sum(items.(price * qty)) >= 100 → false (25 >= 100)"
    );
    // The whole tree serializes to JSON, to store it with the decision.
    let stored = serde_json::to_string(&audit)?;
    assert!(stored.contains(r#""label":"Big order""#));
    // ANCHOR_END: audit
    Ok(())
}

#[test]
fn custom_tracer() -> Result<()> {
    use jlens_core::{Bindings, Expression, Options};
    use serde_json::json;
    // ANCHOR: tracer
    use jlens_core::Node;
    use jlens_core::audit::{Tracer, ValueRef};

    /// Counts the comparisons an evaluation makes.
    #[derive(Default)]
    struct Comparisons(usize);

    impl Tracer for Comparisons {
        fn operands(&mut self, _node: &Node, _lhs: ValueRef<'_>, _rhs: ValueRef<'_>) {
            self.0 += 1;
        }
    }

    let expr = Expression::compile("items[price > 10 and qty < 5]")?;
    let mut counter = Comparisons::default();
    expr.evaluate_traced(
        Some(&json!({"items": [{"price": 12, "qty": 1}, {"price": 5, "qty": 9}]})),
        &Bindings::new(),
        &Options::default(),
        &mut counter,
    )?;
    assert_eq!(counter.0, 3); // the second item fails `price > 10`, so `qty < 5` is skipped
    // ANCHOR_END: tracer
    Ok(())
}

#[test]
fn tooling() -> Result<()> {
    // ANCHOR: tooling
    use jlens_core::{NodeKind, Severity, parse_recover};

    // Recovering parsing never fails: it returns a partial tree and every problem found.
    let ast = parse_recover("Account.Order[0;].Product");
    let errors: Vec<_> = ast
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| (d.code.as_str(), d.span.start))
        .collect();
    assert_eq!(errors, [("S0202", 15)]);

    // Walk the tree, for example to list the variables an expression uses (functions are
    // variables too).
    let ast = jlens_core::parse("$a + $b * $sum(items)")?;
    let mut variables = Vec::new();
    ast.root.walk(&mut |node| {
        if let NodeKind::Variable { name } = &node.kind {
            variables.push(name.clone());
        }
    });
    assert_eq!(variables, ["a", "b", "sum"]);
    // ANCHOR_END: tooling
    Ok(())
}

#[test]
fn functions() {
    // ANCHOR: functions
    let catalog = jlens_core::function_catalog();
    let substring = catalog.iter().find(|f| f.name == "substring").unwrap();
    assert_eq!(substring.signature, "<s-nn?:s>");
    assert_eq!(substring.params, ["str", "start", "length"]);
    // ANCHOR_END: functions
}

#[test]
fn threads() -> Result<()> {
    use jlens_core::{Bindings, Expression};
    use serde_json::json;
    // ANCHOR: threads
    use std::sync::Arc;
    use std::thread;

    let expr = Arc::new(Expression::compile("$uppercase(name)")?);
    let workers: Vec<_> = ["ann", "bob"]
        .into_iter()
        .map(|name| {
            let expr = Arc::clone(&expr);
            thread::spawn(move || expr.evaluate(Some(&json!({"name": name})), &Bindings::new()))
        })
        .collect();
    for worker in workers {
        assert!(worker.join().unwrap()?.is_some());
    }
    // ANCHOR_END: threads
    Ok(())
}
