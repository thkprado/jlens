//! Table-driven tests of the language service. `|` marks the cursor in each expression.

use jlens_schema::{CompletionKind, LanguageService, Override, Schema};
use serde_json::{Value, json};

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

fn service() -> LanguageService {
    let doc: Value = serde_json::from_str(&fixture("online-store-order.json")).unwrap();
    let mut schema = Schema::infer(&[doc]);
    let overrides: std::collections::BTreeMap<String, Override> = serde_json::from_value(json!({
        "items[].type": {"description": "Product category", "enum": ["apparel", "electronic", "toy"]},
        "items[].tax": {"label": "Import tax"},
        "number": {"hidden": true}
    }))
    .unwrap();
    assert!(schema.apply_overrides(&overrides).is_empty());
    LanguageService::new(Some(schema)).with_variable("approved", None)
}

/// Splits `"items[|"` into the text and the cursor offset.
fn cursor(marked: &str) -> (String, usize) {
    let at = marked.find('|').expect("cursor marker");
    (marked.replacen('|', "", 1), at)
}

fn labels(svc: &LanguageService, marked: &str, kind: Option<CompletionKind>) -> Vec<String> {
    let (text, offset) = cursor(marked);
    svc.complete(&text, offset)
        .items
        .into_iter()
        .filter(|i| kind.is_none_or(|k| i.kind == k))
        .map(|i| i.label)
        .collect()
}

const ITEM_FIELDS: &[&str] = &["code", "color", "type", "unitPrice", "quantity", "tax"];

#[test]
fn field_completion_follows_the_context() {
    let svc = service();
    let cases: &[(&str, &[&str])] = &[
        // Root fields; `number` is hidden by an override.
        ("|", &["customer", "items"]),
        ("cus|", &["customer"]),
        ("customer.|", &["login", "name"]),
        ("customer.lo|x", &["login"]),
        // Array items are flattened, as during evaluation.
        ("items.|", ITEM_FIELDS),
        ("items[type = 'electronic'].|", ITEM_FIELDS),
        // Inside a predicate the context is one item.
        ("items[|]", ITEM_FIELDS),
        ("items[quantity > 1 and c|]", &["code", "color"]),
        // Inside a block applied to each item.
        ("$sum(items.(unitPrice * q|))", &["quantity"]),
        // Group-by keys and values see the items.
        ("items{ty|: 1}", &["type"]),
        // Order-by terms see the items.
        ("items^(>t|)", &["type", "tax"]),
        // Variables keep the type of what they were bound to.
        ("($i := items[0]; $i.|)", ITEM_FIELDS),
        ("($o := customer; $o.n|)", &["name"]),
        // Focus variables bind the step's items.
        ("items@$it.$it.u|", &["unitPrice"]),
        // Callback parameters of $map / $filter / $sort get the item type.
        ("$map(items, function($i){ $i.c| })", &["code", "color"]),
        ("$filter(items, function($i){ $i.q| > 1 })", &["quantity"]),
        // Functions that return their input keep its type.
        (
            "$sort(items, function($a, $b){ $a.tax > $b.tax })[0].u|",
            &["unitPrice"],
        ),
        (
            "(items ~> $filter(function($i){ $i.tax > 0 })).t|",
            &["type", "tax"],
        ),
        // Parsing recovers from the rest of the expression being incomplete.
        ("items[type = 'x' and |", ITEM_FIELDS),
        ("$count(items[|", ITEM_FIELDS),
        // Quoted names.
        ("items.`ty|", &["type"]),
    ];
    for (marked, expected) in cases {
        let got = labels(&svc, marked, Some(CompletionKind::Field));
        assert_eq!(got, *expected, "{marked}");
    }
}

#[test]
fn variables_functions_and_operators() {
    let svc = service();
    let vars = labels(&svc, "($total := 1; $t|)", Some(CompletionKind::Variable));
    assert_eq!(vars, ["$total"]);
    let fns = labels(&svc, "$su|", Some(CompletionKind::Function));
    assert_eq!(
        fns,
        ["$sum", "$substring", "$substringBefore", "$substringAfter"]
    );
    let external = labels(&svc, "$appr|", None);
    assert_eq!(external, ["$approved"]);
    // After a complete operand and a space, an operator is expected.
    let ops = labels(&svc, "items[type = 'x' a|]", None);
    assert_eq!(ops, ["and"]);
    let ops = labels(&svc, "customer.login |", None);
    assert_eq!(ops, ["and", "or", "in"]);
    // No suggestions inside strings or comments.
    assert!(labels(&svc, "items[type = 'ele|']", None).is_empty());
    assert!(labels(&svc, "/* it| */ items", None).is_empty());
}

#[test]
fn completion_items_carry_types_docs_and_ranges() {
    let svc = service();
    let (text, offset) = cursor("items.ty|pe");
    let completions = svc.complete(&text, offset);
    assert_eq!((completions.range.start, completions.range.end), (6, 10));
    let item = &completions.items[0];
    assert_eq!(item.label, "type");
    assert_eq!(item.detail.as_deref(), Some("string"));
    let docs = item.documentation.as_deref().unwrap();
    assert!(docs.contains("Product category"), "{docs}");
    assert!(docs.contains("`\"electronic\"`"), "{docs}");

    let schema = Schema::infer(&[json!({"first name": 1, "and": 2, "ok": 3})]);
    let svc = LanguageService::new(Some(schema));
    let inserts: Vec<String> = svc
        .complete("", 0)
        .items
        .into_iter()
        .filter(|i| i.kind == CompletionKind::Field)
        .map(|i| i.insert_text)
        .collect();
    assert_eq!(inserts, ["`first name`", "`and`", "ok"]);
}

#[test]
fn diagnostics_report_unknown_fields() {
    let svc = service();
    let cases: &[(&str, &[(&str, &str)])] = &[
        ("items[type = 'electronic'].unitPrice", &[]),
        (
            "items.typ",
            &[(
                "W0201",
                "Field \"typ\" is not in the schema; did you mean `type`?",
            )],
        ),
        (
            "customer.nick",
            &[("W0201", "Field \"nick\" is not in the schema")],
        ),
        // Unknown contexts are not checked.
        ("$lookup(customer, 'x').anything", &[]),
        ("{'a': 1}.b", &[]),
        // Syntax errors come from the parser.
        (
            "items[",
            &[
                ("S0203", "Expected \"]\" before end of expression"),
                ("S0207", "Unexpected end of expression"),
            ],
        ),
    ];
    for (text, expected) in cases {
        let got: Vec<(String, String)> = svc
            .diagnostics(text)
            .into_iter()
            .map(|d| (d.code, d.message))
            .collect();
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(c, m)| ((*c).into(), (*m).into()))
            .collect();
        assert_eq!(got, expected, "{text}");
    }
    let d = &svc.diagnostics("items.typ")[0];
    assert_eq!((d.span.start, d.span.end), (6, 9));
}

#[test]
fn diagnostics_report_unknown_functions_and_argument_counts() {
    use jlens_schema::FunctionDecl;
    let svc = LanguageService::new(None)
        .with_variable("callback", None)
        .with_function(FunctionDecl::new("shipping", "").signature("<ns?:n>"))
        .with_function(FunctionDecl::new("audit", ""));
    let cases: &[(&str, &[(&str, &str)])] = &[
        ("$sum([1]) + $substring('abc', 1) + $string()", &[]),
        (
            "$summ([1])",
            &[("W0202", "Unknown function $summ; did you mean `$sum`?")],
        ),
        (
            "$frobnicate(1)",
            &[("W0202", "Unknown function $frobnicate")],
        ),
        (
            "$count()",
            &[("W0203", "Function $count takes 1 argument but is given 0")],
        ),
        (
            "$substring('abc', 1, 2, 3)",
            &[(
                "W0203",
                "Function $substring takes 1 to 3 arguments but is given 4",
            )],
        ),
        (
            "$string(1, true, 3)",
            &[(
                "W0203",
                "Function $string takes at most 2 arguments but is given 3",
            )],
        ),
        (
            "$zip()",
            &[(
                "W0203",
                "Function $zip takes at least 1 argument but is given 0",
            )],
        ),
        // The value applied with `~>` is the first argument.
        ("'abc' ~> $substring(1)", &[]),
        (
            "'abc' ~> $uppercase(1)",
            &[(
                "W0203",
                "Function $uppercase takes at most 1 argument but is given 2",
            )],
        ),
        // Partial application counts the placeholders.
        ("$substring(?, 1)('abc')", &[]),
        (
            "$sum(?, ?)",
            &[("W0203", "Function $sum takes 1 argument but is given 2")],
        ),
        // Names bound in the expression, wherever they are bound, and declared names.
        (
            "($f := function($n) { $n < 1 ? 0 : $f($n - 1) }; $f(3))",
            &[],
        ),
        (
            "($g := function() { $h() }; $h := function() { 1 }; $g())",
            &[],
        ),
        ("function($fn) { $fn(1, 2) }", &[]),
        (
            "($count := function($a, $b) { $a + $b }; $count(1, 2))",
            &[],
        ),
        ("a@$x.$x()", &[]),
        ("$callback(1) + $audit(1, 2, 3)", &[]),
        // Declared functions with a signature are checked against it.
        ("$shipping(1)", &[]),
        (
            "$shipping(1, 'eu', 3)",
            &[(
                "W0203",
                "Function $shipping takes 1 to 2 arguments but is given 3",
            )],
        ),
        // `$` and `$$` hold values, not functions, but are not names to suggest for.
        ("$()", &[]),
    ];
    for (text, expected) in cases {
        let got: Vec<(String, String)> = svc
            .diagnostics(text)
            .into_iter()
            .map(|d| (d.code, d.message))
            .collect();
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(c, m)| ((*c).into(), (*m).into()))
            .collect();
        assert_eq!(got, expected, "{text}");
    }
    let d = &svc.diagnostics("1 + $summ(2)")[0];
    assert_eq!((d.span.start, d.span.end), (4, 9));
}

#[test]
fn hover_and_signature_help() {
    let svc = service();
    let (text, offset) = cursor("items.t|ax");
    let hover = svc.hover(&text, offset).unwrap();
    assert!(
        hover.contents.starts_with("`tax`: number"),
        "{}",
        hover.contents
    );
    assert!(hover.contents.contains("Import tax"));

    let (text, offset) = cursor("$su|m(items.tax)");
    let hover = svc.hover(&text, offset).unwrap();
    assert!(hover.contents.contains("Sum of an array of numbers"));

    let (text, offset) = cursor("$substring(customer.name, |");
    let help = svc.signature_help(&text, offset).unwrap();
    assert_eq!(help.name, "$substring");
    assert_eq!(help.params, ["$str", "$start", "$length"]);
    assert_eq!(help.active_param, 1);
    let (text, offset) = cursor("$join(items[type = 'a'].code, |)");
    assert_eq!(svc.signature_help(&text, offset).unwrap().active_param, 1);
    assert!(svc.signature_help("items[0]", 7).is_none());
}

#[test]
fn works_without_a_schema() {
    let svc = LanguageService::new(None);
    assert!(svc.diagnostics("a.b.c").is_empty());
    let fns = labels(&svc, "$upp|", None);
    assert_eq!(fns, ["$uppercase"]);
}

#[test]
fn declared_host_functions_are_completed_and_documented() {
    use jlens_schema::FunctionDecl;
    let svc = service()
        .with_function(
            FunctionDecl::new("shipping", "Shipping cost of a weight to a region")
                .signature("<ns?:n>")
                .params(["weight", "region"]),
        )
        .with_function(FunctionDecl::new("sort", "The host's own sort"));

    // Declared functions come first and replace built-ins of the same name.
    let fns = labels(&svc, "$s|", Some(CompletionKind::Function));
    assert_eq!(fns[..2], ["$shipping", "$sort"]);
    assert_eq!(fns.iter().filter(|f| *f == "$sort").count(), 1);
    let (text, offset) = cursor("$shi|");
    let item = &svc.complete(&text, offset).items[0];
    assert_eq!(item.detail.as_deref(), Some("<ns?:n>"));
    assert_eq!(
        item.documentation.as_deref(),
        Some("Shipping cost of a weight to a region")
    );
    // Without a signature, there is no detail.
    let (text, offset) = cursor("$sor|");
    assert_eq!(svc.complete(&text, offset).items[0].detail, None);

    let (text, offset) = cursor("$ship|ping(3)");
    let hover = svc.hover(&text, offset).unwrap();
    assert_eq!(
        hover.contents,
        "`$shipping<ns?:n>`\n\nShipping cost of a weight to a region"
    );
    let (text, offset) = cursor("$shipping(3, |)");
    let help = svc.signature_help(&text, offset).unwrap();
    assert_eq!(help.params, ["$weight", "$region"]);
    assert_eq!(help.active_param, 1);
    assert_eq!(help.summary, "Shipping cost of a weight to a region");

    // The result of a host function is not typed as the built-in's.
    assert_eq!(service().diagnostics("$sort(items)[0].nick").len(), 1);
    assert!(svc.diagnostics("$sort(items)[0].nick").is_empty());
}

#[test]
fn function_declarations_deserialize_from_json() {
    use jlens_schema::FunctionDecl;
    let decl: FunctionDecl =
        serde_json::from_value(json!({"name": "tax", "signature": "<n:n>"})).unwrap();
    assert_eq!(decl, FunctionDecl::new("tax", "").signature("<n:n>"));
    assert!(serde_json::from_value::<FunctionDecl>(json!({"name": "x", "sig": "<n>"})).is_err());
}
