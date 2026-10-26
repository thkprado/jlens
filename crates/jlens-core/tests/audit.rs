//! Auditing: audit trees, explanations and event logs.

use jlens_core::audit::{
    AuditLevel, AuditOptions, AuditTree, CaptureLimits, JsonlTracer, OutcomeKind, explain,
};
use jlens_core::{Bindings, Expression, Options};
use serde_json::{Value, json};

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

fn order() -> Value {
    serde_json::from_str(&fixture("online-store-order.json")).unwrap()
}

fn audit(src: &str, input: &Value, options: AuditOptions) -> AuditTree {
    let expr = Expression::compile(src).unwrap();
    expr.evaluate_audited(Some(input), &Bindings::new(), &Options::default(), &options)
        .1
}

fn rule_audit(input: &Value) -> AuditTree {
    audit(
        &fixture("shipping-rule.jsonata"),
        input,
        AuditOptions::decisions(),
    )
}

/// The store order example from the guides: an order that qualifies.
#[test]
fn approved_order_explanation() {
    let tree = rule_audit(&order());
    insta::assert_snapshot!(explain(&tree).to_string());
}

/// An order below the minimum total: the explanation points at the failed check and shows
/// its business message.
#[test]
fn rejected_order_explanation() {
    let mut doc = order();
    doc["items"][1]["unitPrice"] = json!(5000);
    let tree = rule_audit(&doc);
    assert_eq!(
        tree.header.result.as_ref().unwrap()["action"],
        "REGULAR_FEE"
    );
    let explanation = explain(&tree);
    let failed: Vec<&str> = explanation
        .lines
        .iter()
        .filter(|l| l.passed == Some(false))
        .map(|l| l.text.as_str())
        .collect();
    assert!(
        failed
            .iter()
            .any(|t| t.starts_with("Minimum order grand total"))
    );
    insta::assert_snapshot!(explanation.to_string());
}

#[test]
fn audit_tree_json() {
    let tree = rule_audit(&order());
    insta::assert_json_snapshot!(tree, {
        ".header.timestamp" => "[timestamp]",
        ".header.durationMs" => "[duration]",
    });
}

#[test]
fn levels_control_what_is_recorded() {
    let src = "$sum(items[quantity > 1].(unitPrice * quantity))";
    let doc = order();
    let off = audit(
        src,
        &doc,
        AuditOptions {
            level: AuditLevel::Off,
            ..Default::default()
        },
    );
    let decisions = audit(src, &doc, AuditOptions::decisions());
    let full = audit(src, &doc, AuditOptions::full());
    assert!(off.root.is_none());
    assert_eq!(off.header.result, Some(json!(20000)));
    // Decisions: the root and the path that carries the filter statistics, no per-item nodes.
    assert_eq!(decisions.node_count, 2);
    let filter = &decisions.root.as_ref().unwrap().children[0].filters[0];
    assert_eq!((filter.tested, filter.passed), (3, 1));
    assert!(full.node_count > 10, "full: {}", full.node_count);
    assert!(full.root.unwrap().input.is_some());
}

#[test]
fn capture_limits_bound_the_audit() {
    let items: Vec<Value> = (0..100)
        .map(|i| json!({"n": i, "text": "x".repeat(50)}))
        .collect();
    let input = json!({"items": items});
    let options = AuditOptions {
        level: AuditLevel::Full,
        limits: CaptureLimits {
            max_items: 3,
            max_string: 10,
            max_nodes: 50,
            ..CaptureLimits::default()
        },
    };
    let tree = audit("items[n % 2 = 0].text", &input, options);
    assert!(tree.truncated);
    assert!(tree.node_count <= 50);
    let output = tree.root.unwrap().output.unwrap();
    let array = output.as_array().unwrap();
    assert_eq!(array.len(), 4);
    assert_eq!(array[0], json!("xxxxxxxxxx…"));
    assert_eq!(array[3], json!("… 47 more"));
}

/// The nodes reached from the root, and the items their filters tested.
fn reached(tree: &AuditTree) -> (usize, usize) {
    let mut stack: Vec<_> = tree.root.iter().collect();
    let (mut nodes, mut tested) = (0, 0);
    while let Some(node) = stack.pop() {
        nodes += 1;
        tested += node.filters.iter().map(|f| f.tested).sum::<usize>();
        stack.extend(&node.children);
    }
    (nodes, tested)
}

#[test]
fn a_tree_cut_by_max_nodes_holds_the_nodes_it_counts() {
    let rule = fixture("shipping-rule.jsonata");
    let cases = [
        (rule.as_str(), order()),
        (
            "items[n > 1].(n * 2)",
            json!({"items": [{"n": 1}, {"n": 2}, {"n": 3}]}),
        ),
    ];
    for (src, input) in &cases {
        for level in [AuditLevel::Decisions, AuditLevel::Full] {
            let options = AuditOptions {
                level,
                ..AuditOptions::default()
            };
            let whole = audit(src, input, options);
            assert!(!whole.truncated);
            let (count, tested) = reached(&whole);
            assert_eq!(count, whole.node_count);
            for max_nodes in 0..whole.node_count + 3 {
                let options = AuditOptions {
                    level,
                    limits: CaptureLimits {
                        max_nodes,
                        ..CaptureLimits::default()
                    },
                };
                let cut = audit(src, input, options);
                let at = format!("{level:?}, max_nodes {max_nodes}, {src}");
                assert_eq!(reached(&cut), (cut.node_count, tested), "{at}");
                assert_eq!(cut.node_count, max_nodes.clamp(1, whole.node_count), "{at}");
                assert_eq!(cut.truncated, max_nodes.max(1) < whole.node_count, "{at}");
            }
        }
    }
}

#[test]
fn errors_are_audited() {
    let tree = audit("($x := 'a'; $x + 1)", &json!({}), AuditOptions::decisions());
    let error = tree.header.error.as_ref().unwrap();
    assert_eq!(error.code, "T2001");
    assert_eq!(tree.root.as_ref().unwrap().outcome, OutcomeKind::Error);
    assert!(explain(&tree).result.starts_with("error T2001"));
}

#[test]
fn short_circuits_and_branches_are_recorded() {
    let tree = audit(
        "a > 1 or b > 1 ? 'yes' : 'no'",
        &json!({"a": 5}),
        AuditOptions::decisions(),
    );
    let root = tree.root.unwrap();
    assert_eq!(root.branch, Some(jlens_core::audit::Branch::Then));
    let or = &root.children[0];
    assert_eq!(or.op.as_deref(), Some("or"));
    assert!(or.short_circuited);
}

#[test]
fn jsonl_events_are_balanced() {
    let expr = Expression::compile("items[type = 'electronic'].code").unwrap();
    let mut log = JsonlTracer::new(Vec::new(), CaptureLimits::default());
    let result = expr
        .evaluate_traced(
            Some(&order()),
            &Bindings::new(),
            &Options::default(),
            &mut log,
        )
        .unwrap();
    assert_eq!(result, Some(json!(["2", "3"])));
    let text = String::from_utf8(log.into_inner()).unwrap();
    let events: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let count = |e: &str| events.iter().filter(|v| v["event"] == e).count();
    assert_eq!(count("begin"), 1);
    assert_eq!(count("end"), 1);
    assert_eq!(count("enter"), count("exit"));
    assert_eq!(count("filter"), 3);
    assert_eq!(count("operands"), 3);
}

#[test]
fn audited_results_match_plain_evaluation() {
    let rule = Expression::compile(&fixture("shipping-rule.jsonata")).unwrap();
    let doc = order();
    let plain = rule.evaluate(Some(&doc), &Bindings::new()).unwrap();
    let (audited, _) = rule.evaluate_audited(
        Some(&doc),
        &Bindings::new(),
        &Options::default(),
        &AuditOptions::full(),
    );
    assert_eq!(audited.unwrap(), plain);
}

/// A function called by a host function (ADR 0011): its steps nest under the host call's node.
#[test]
fn host_callback_nests_under_the_call() {
    use jlens_core::HostFunction;
    let apply = HostFunction::new(2, |args, call| call.function(0).unwrap().call(&args[1..]));
    let bindings = Bindings::new().function("apply", apply);
    let expr = Expression::compile("$apply(function($x) { $x * 2 }, 21)").unwrap();
    let (result, tree) =
        expr.evaluate_audited(None, &bindings, &Options::default(), &AuditOptions::full());
    assert_eq!(result.unwrap(), Some(json!(42)));
    insta::assert_json_snapshot!(tree, {
        ".header.timestamp" => "[timestamp]",
        ".header.durationMs" => "[duration]",
    });
}
