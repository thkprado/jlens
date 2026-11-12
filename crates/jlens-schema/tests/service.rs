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
        // The parent operator: the item the step it refers to is evaluated against.
        ("items.%.|", &["customer", "items"]),
        ("items[%.c|]", &["customer"]),
        ("items.{'n': %.customer.|}", &["login", "name"]),
        ("customer.%.items.%.c|", &["customer"]),
        ("items.code.%.ta|", &["tax"]),
        ("items.code.%.%.c|", &["customer"]),
        ("items@$i.%.c|", &["customer"]),
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
    // No suggestions inside comments, or strings that are not compared with an enumeration.
    assert!(labels(&svc, "$uppercase('ele|')", None).is_empty());
    assert!(labels(&svc, "items[code = 'ele|']", None).is_empty());
    assert!(labels(&svc, "/* it| */ items", None).is_empty());
}

#[test]
fn value_completion_offers_the_left_hand_sides_enumeration() {
    let svc = service();
    let all: &[&str] = &[r#""apparel""#, r#""electronic""#, r#""toy""#];
    let cases: &[(&str, &[&str])] = &[
        ("items[type = |]", all),
        ("items[type != |]", all),
        ("items[type = e|]", &[r#""electronic""#]),
        ("items[type = E|]", &[r#""electronic""#]),
        ("items[type in |]", all),
        ("items[type in [|]]", all),
        // Values already in the array are left out.
        (
            r#"items[type in ["apparel", |]]"#,
            &[r#""electronic""#, r#""toy""#],
        ),
        // Inside quotes: the quote is kept, and only strings are offered.
        (r#"items[type = "ap|"]"#, &[r#""apparel""#]),
        ("items[type = 'ap|']", &["'apparel'"]),
        (
            r#"items[type in ["toy", "|"]]"#,
            &[r#""apparel""#, r#""electronic""#],
        ),
        (r#"items[type = "|"#, all),
        // Not on the left-hand side, after `<`, nor for a field without an enumeration.
        ("items[| = type]", &[]),
        ("items[type < |]", &[]),
        ("items[code = |]", &[]),
        ("items.type = |", all),
    ];
    for (marked, expected) in cases {
        let got = labels(&svc, marked, Some(CompletionKind::Value));
        assert_eq!(got, *expected, "{marked}");
    }
    // Values come first; fields, variables and functions follow.
    let (text, offset) = cursor("items[type = |]");
    let items = svc.complete(&text, offset).items;
    assert_eq!(items[0].kind, CompletionKind::Value);
    assert!(items.iter().any(|i| i.kind == CompletionKind::Field));

    // Inside quotes the whole literal is replaced; an unclosed one up to the cursor.
    let (text, offset) = cursor(r#"items[type = "ap|p"]"#);
    let completions = svc.complete(&text, offset);
    assert_eq!((completions.range.start, completions.range.end), (13, 18));
    assert_eq!(completions.items[0].insert_text, r#""apparel""#);
    let (text, offset) = cursor(r#"items[type = "ap|"#);
    let completions = svc.complete(&text, offset);
    assert_eq!((completions.range.start, completions.range.end), (13, 16));

    // A `const`, non-string values, and strings that need escaping or keep double quotes.
    let schema = Schema::from_json_schema(&json!({
        "type": "object",
        "properties": {
            "kind": {"const": "order"},
            "level": {"enum": [1, 2.5, true, null]},
            "label": {"enum": ["say \"hi\"", "it's"]}
        }
    }))
    .unwrap();
    let svc = LanguageService::new(Some(schema));
    assert_eq!(
        labels(&svc, "kind = |", Some(CompletionKind::Value)),
        [r#""order""#]
    );
    assert_eq!(
        labels(&svc, "level = |", Some(CompletionKind::Value)),
        ["1", "2.5", "true", "null"]
    );
    assert!(labels(&svc, "level = '|'", Some(CompletionKind::Value)).is_empty());
    // Numbers are compared by value: `1.0` is `1`.
    assert_eq!(
        labels(&svc, "level in [1.0, |]", Some(CompletionKind::Value)),
        ["2.5", "true", "null"]
    );
    assert_eq!(
        labels(&svc, "label = '|'", Some(CompletionKind::Value)),
        [r#"'say \"hi\"'"#, r#""it's""#]
    );
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
        // Under the parent operator, the fields of the parent.
        ("items.%.customer.name", &[]),
        ("items[%.customer.name = 'x'].code", &[]),
        ("items.code.%.tax", &[]),
        (
            "items.%.custmer",
            &[(
                "W0201",
                "Field \"custmer\" is not in the schema; did you mean `customer`?",
            )],
        ),
        (
            "items.code.%.nick",
            &[("W0201", "Field \"nick\" is not in the schema")],
        ),
        // A parent the typing does not know is not checked: the step it refers to is inside
        // a block, or under an unknown value.
        ("customer.(login).%.anything", &[]),
        ("$lookup(customer, 'x').a.%.anything", &[]),
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
        ("$sum([1]) & $substring('abc', 1) & $string()", &[]),
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

fn assert_diagnostics(svc: &LanguageService, cases: &[(&str, &[(&str, &str)])]) {
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
}

#[test]
fn diagnostics_report_values_outside_a_declared_enumeration() {
    let svc = service();
    let outside = |value: &str, hint: &str| {
        format!("The value {value} is not one of the allowed values{hint}")
    };
    let aparel = outside("'aparel'", "; did you mean `'apparel'`?");
    let aparel_dq = outside(r#""aparel""#, r#"; did you mean `"apparel"`?"#);
    let food = outside("'food'", "");
    let cases: &[(&str, &[(&str, &str)])] = &[
        ("items[type = 'apparel']", &[]),
        ("items[type = 'aparel']", &[("W0206", &aparel)]),
        (r#"items[type != "aparel"]"#, &[("W0206", &aparel_dq)]),
        ("items[type in ['toy', 'food']]", &[("W0206", &food)]),
        ("items[type in 'food']", &[("W0206", &food)]),
        ("items[0].type = 'food'", &[("W0206", &food)]),
        // A number outside a string enumeration has no suggestion.
        ("items[type = 1]", &[("W0206", &outside("1", ""))]),
        // An enumeration inferred from samples is a guess: `quantity` was only seen as 1 and 2.
        ("items[quantity = 3]", &[]),
        // Values that are not literals, and literals on the left-hand side, are not checked.
        ("items[type = color]", &[]),
        ("items[type = 'f' & 'ood']", &[]),
        ("items['food' = type]", &[]),
        ("items[type = 'food'[false]]", &[]),
        ("items[type in ['toy', 'food'][0]]", &[]),
        // Nor fields without an enumeration, nor unknown values.
        ("items[code = 'x']", &[]),
        ("$lookup(items, 'type') = 'food'", &[]),
    ];
    assert_diagnostics(&svc, cases);
    let d = &svc.diagnostics("items[type in ['toy', 'food']]")[0];
    assert_eq!((d.span.start, d.span.end), (22, 28));

    // `const`, numbers compared by value, and enumerations of array items.
    let schema = Schema::from_json_schema(&json!({
        "type": "object",
        "properties": {
            "kind": {"const": "order"},
            "level": {"type": "integer", "enum": [1, 2]},
            "tags": {"type": "array", "items": {"enum": ["new", "sale"]}},
            "guess": {"type": "integer", "enum": [1, 2], "x-jlens-enum-inferred": true}
        }
    }))
    .unwrap();
    let svc = LanguageService::new(Some(schema));
    let ordr = outside("'ordr'", "; did you mean `'order'`?");
    // A number has no nearest value.
    let level = outside("3", "");
    let sael = outside(r#""sael""#, r#"; did you mean `"sale"`?"#);
    let cases: &[(&str, &[(&str, &str)])] = &[
        ("kind = 'ordr'", &[("W0206", &ordr)]),
        ("level = 2.0", &[]),
        ("level in [1, 3]", &[("W0206", &level)]),
        (r#""new" in tags or tags = "sael""#, &[("W0206", &sael)]),
        ("guess = 3", &[]),
    ];
    assert_diagnostics(&svc, cases);
}

#[test]
fn open_domains_request_their_values_from_the_application() {
    let schema = Schema::from_json_schema(&json!({
        "type": "object",
        "properties": {
            "customer": {"type": "object", "properties": {
                "country": {"type": "string", "x-jlens-domain": "countries", "enum": ["BR"]}
            }},
            "orders": {"type": "array", "items": {"type": "object", "properties": {
                "sku": {"type": "string", "x-jlens-domain": "products"}
            }}},
            "tags": {"type": "array", "items": {"type": "string", "x-jlens-domain": "tags"}},
            "kind": {"type": "string"}
        }
    }))
    .unwrap();
    let home = schema.shape_at("customer.country").unwrap().clone();
    let svc = LanguageService::new(Some(schema)).with_variable("home", Some(home));
    // (expression, domain, path, prefix, replaced range, quote)
    type Case<'a> = (
        &'a str,
        &'a str,
        Option<&'a str>,
        &'a str,
        (usize, usize),
        char,
    );
    let cases: &[Case] = &[
        (
            "customer.country = |",
            "countries",
            Some("customer.country"),
            "",
            (19, 19),
            '"',
        ),
        (
            "customer.country != Bra|zil",
            "countries",
            Some("customer.country"),
            "Bra",
            (20, 26),
            '"',
        ),
        (
            r#"orders[sku = "ab|"]"#,
            "products",
            Some("orders[].sku"),
            "ab",
            (13, 17),
            '"',
        ),
        (
            "orders[sku in ['x', 'y|",
            "products",
            Some("orders[].sku"),
            "y",
            (20, 22),
            '\'',
        ),
        (
            "orders[sku in [|]]",
            "products",
            Some("orders[].sku"),
            "",
            (15, 15),
            '"',
        ),
        ("tags = |", "tags", Some("tags[]"), "", (7, 7), '"'),
        // A variable's shape is not a field of the schema: no path.
        ("$home = |", "countries", None, "", (8, 8), '"'),
    ];
    for &(marked, domain, path, prefix, (start, end), quote) in cases {
        let (text, offset) = cursor(marked);
        let completions = svc.complete(&text, offset);
        let request = completions
            .value_request
            .unwrap_or_else(|| panic!("no value request for {marked}"));
        assert_eq!(request.domain, domain, "{marked}");
        assert_eq!(request.path.as_deref(), path, "{marked}");
        assert_eq!(request.prefix, prefix, "{marked}");
        assert_eq!(
            (request.range.start, request.range.end),
            (start, end),
            "{marked}"
        );
        assert_eq!(request.range, completions.range, "{marked}");
        assert_eq!(request.quote, quote, "{marked}");
    }
    // The declared values are still offered beside the request.
    assert_eq!(
        labels(&svc, "customer.country = |", Some(CompletionKind::Value)),
        [r#""BR""#]
    );
    // Not where a value is not expected, nor for a field without a domain.
    for marked in [
        "customer.|",
        "customer = |",
        "kind = |",
        "customer.country & |",
        "$|",
        "customer.country = 'BR' and |",
    ] {
        let (text, offset) = cursor(marked);
        assert!(
            svc.complete(&text, offset).value_request.is_none(),
            "{marked}"
        );
    }
    // A value outside an open domain is never warned about, even with an `enum`.
    assert_diagnostics(
        &svc,
        &[("customer.country = 'XX'", &[]), ("'z' in tags", &[])],
    );
    // Hover names the domain.
    let hover = svc.hover("customer.country", 12).unwrap();
    assert!(
        hover.contents.contains("Domain: `countries`"),
        "{}",
        hover.contents
    );

    // An override's `type` is one name or a list, as in JSON Schema, and is written back as the
    // export writes it.
    let one: Override = serde_json::from_value(json!({"type": "integer"})).unwrap();
    let list: Override = serde_json::from_value(json!({"type": ["integer"]})).unwrap();
    assert_eq!(one, list);
    assert_eq!(
        serde_json::to_value(&one).unwrap()["type"],
        json!("integer")
    );
    let union: Override = serde_json::from_value(json!({"type": ["string", "null"]})).unwrap();
    assert_eq!(
        serde_json::to_value(&union).unwrap()["type"],
        json!(["string", "null"])
    );
    assert!(serde_json::from_value::<Override>(json!({"type": "int"})).is_err());
    assert!(serde_json::from_value::<Override>(json!({"type": 1})).is_err());

    // An override declares a domain on an inferred schema, and literals keep the request's quote.
    let mut schema = Schema::infer(&[json!({"status": "open"}), json!({"status": "closed"})]);
    let overrides: std::collections::BTreeMap<String, Override> =
        serde_json::from_value(json!({"status": {"domain": "statuses"}})).unwrap();
    assert!(schema.apply_overrides(&overrides).is_empty());
    let svc = LanguageService::new(Some(schema));
    let request = svc.complete("status = 'op", 12).value_request.unwrap();
    assert_eq!(
        (request.domain.as_str(), request.path.as_deref()),
        ("statuses", Some("status"))
    );
    assert_eq!(request.literal(&json!("open")), "'open'");
    assert_eq!(request.literal(&json!("it's")), r#""it's""#);
    assert_eq!(request.literal(&json!(2)), "2");
    let json = serde_json::to_value(svc.complete("status = ", 9)).unwrap();
    assert_eq!(
        json["valueRequest"],
        json!({
            "domain": "statuses", "path": "status", "prefix": "", "range": {"start": 9, "end": 9},
            "quote": "\"", "conditions": [], "inArray": false, "listed": []
        })
    );
}

#[test]
fn value_requests_carry_the_conditions_beside_them() {
    let schema = Schema::from_json_schema(&json!({
        "type": "object",
        "properties": {
            "country": {"type": "string", "x-jlens-domain": "countries"},
            "state": {"type": "string", "x-jlens-domain": "states"},
            "year": {"type": "integer"},
            "items": {"type": "array", "items": {"type": "object", "properties": {
                "code": {"type": "integer"},
                "categories": {"type": "array", "items": {"type": "object", "properties": {
                    "group": {"type": "string", "x-jlens-domain": "groups"},
                    "subgroup": {"type": "string", "x-jlens-domain": "subgroups"}
                }}}
            }}}
        }
    }))
    .unwrap();
    let svc = LanguageService::new(Some(schema));
    let group = "items[].categories[].group";
    // (expression, the conditions as (path, values), whether the value is in an array)
    let cases: &[(&str, Value, bool)] = &[
        // Unfinished, as while typing.
        (
            "items[categories[group = 'Electronics' and subgroup = '|",
            json!([[group, ["Electronics"]]]),
            false,
        ),
        (
            "items[categories[group in ['A', 'B'] and subgroup in ['x', |]]]",
            json!([[group, ["A", "B"]]]),
            true,
        ),
        (
            "items[categories[subgroup = | and group = 'A']]",
            json!([[group, ["A"]]]),
            false,
        ),
        // Through `or` and parentheses: `group = 'A'` holds whichever side does.
        (
            "items[categories[group = 'A' and (subgroup = | or subgroup = 'y')]]",
            json!([[group, ["A"]]]),
            false,
        ),
        // The other conditions of the same predicate, not those of an outer one.
        (
            "items[code = 1].categories[group = 'A' and subgroup = |]",
            json!([[group, ["A"]]]),
            false,
        ),
        (
            "country = 'BR' and state = |",
            json!([["country", ["BR"]]]),
            false,
        ),
        // Numbers keep the value written: an integer stays one.
        (
            "year in [2025, 2026.5] and country = 'BR' and state = |",
            json!([["year", [2025, 2026.5]], ["country", ["BR"]]]),
            false,
        ),
        ("state = |", json!([]), false),
        // Not a condition: one side of `or`, `!=`, a value that is not a literal, a negation, a
        // comparison in another predicate, or something that is not a field.
        (
            "items[categories[(group = 'A' or group = 'B') and subgroup = |]]",
            json!([]),
            false,
        ),
        (
            "items[categories[group != 'A' and subgroup = |]]",
            json!([]),
            false,
        ),
        (
            "items[categories[group = $g and subgroup = |]]",
            json!([]),
            false,
        ),
        (
            "items[categories[$not(group = 'A') and subgroup = |]]",
            json!([]),
            false,
        ),
        (
            "items[categories[group = 'A']] and items[categories[subgroup = |]]",
            json!([]),
            false,
        ),
        ("$x = 'A' and 'B' = country and state = |", json!([]), false),
    ];
    for (marked, conditions, in_array) in cases {
        let (text, offset) = cursor(marked);
        let request = svc
            .complete(&text, offset)
            .value_request
            .unwrap_or_else(|| panic!("no value request for {marked}"));
        let found: Vec<Value> = request
            .conditions
            .iter()
            .map(|c| json!([c.path, c.values]))
            .collect();
        assert_eq!(Value::from(found), *conditions, "{marked}");
        assert_eq!(request.in_array, *in_array, "{marked}");
    }
    let json = serde_json::to_value(svc.complete("country = 'BR' and state = ", 27)).unwrap();
    assert_eq!(
        json["valueRequest"]["conditions"],
        json!([{"path": "country", "values": ["BR"]}])
    );

    // The literals already in the array of `in`, wherever the cursor is in it; none elsewhere.
    for (marked, listed) in [
        ("state in ['SP', 'RJ', |]", json!(["SP", "RJ"])),
        ("state in ['SP', '|', 'RJ']", json!(["SP", "RJ"])),
        ("state in [|", json!([])),
        ("year = 1 and state in ['SP', $x, 2, |]", json!(["SP", 2])),
        ("state = |", json!([])),
    ] {
        let (text, offset) = cursor(marked);
        let request = svc.complete(&text, offset).value_request.unwrap();
        assert_eq!(Value::from(request.listed), listed, "{marked}");
    }
}

#[test]
fn diagnostics_report_undeclared_variables_when_variables_are_declared() {
    use jlens_schema::FunctionDecl;
    let svc = LanguageService::new(None)
        .with_variable("limit", None)
        .with_variable("region", None)
        .with_function(FunctionDecl::new("rate", ""));
    assert_diagnostics(
        &svc,
        &[
            ("$limit + $count($region)", &[]),
            (
                "$limt",
                &[(
                    "W0204",
                    "Variable $limt is not declared; did you mean `$limit`?",
                )],
            ),
            ("$other", &[("W0204", "Variable $other is not declared")]),
            // Names bound anywhere in the expression, the context, the input and functions.
            ("($f := function($n) { $n + $m }; $m := 1; $f(2))", &[]),
            ("items@$i#$p.($i + $p)", &[]),
            ("$ + $$.a", &[]),
            ("$map([1], $string) ~> $rate", &[]),
            // A call of an unknown name gets `W0202` alone.
            ("$other()", &[("W0202", "Unknown function $other")]),
        ],
    );
    let d = &svc.diagnostics("1 + $other")[0];
    assert_eq!((d.span.start, d.span.end), (4, 10));

    // Without declarations, variables are not checked: the bindings are not known.
    let undeclared = LanguageService::new(None);
    assert!(undeclared.diagnostics("$other").is_empty());
}

#[test]
fn diagnostics_report_argument_types_the_signature_rejects() {
    use jlens_schema::FunctionDecl;
    let schema = Schema::infer(&[
        json!({
            "name": "Ann",
            "age": 30,
            "tags": ["a", "b"],
            "scores": [1, 2],
            "items": [{"price": 1.5, "label": "x"}],
            "mixed": "1",
        }),
        json!({"mixed": 2}),
    ]);
    let svc = LanguageService::new(Some(schema))
        .with_variable("limit", Some(Schema::infer(&[json!(5)]).root))
        .with_variable("anything", None)
        .with_function(FunctionDecl::new("rate", "").signature("<sn?:n>"));
    let message = |index: usize, function: &str, ty: &str, signature: &str| {
        format!(
            "Argument {index} of ${function} is {ty}, which does not match the signature \
             {signature}"
        )
    };
    let cases: Vec<(&str, Vec<(&str, String)>)> = vec![
        // Known types that match, and types that are not known.
        (
            "$uppercase(name) & $sum(scores) & $sum(items.price) & $string(age)",
            vec![],
        ),
        ("$join(tags, ', ') & $count(tags) & $length(mixed)", vec![]),
        (
            "$uppercase($anything) & $sum({'a': 1}.a) & $abs(1 + 2)",
            vec![],
        ),
        (
            "$substring(name, 1) & $substring(1) & 'abc' ~> $substring(1)",
            vec![],
        ),
        (
            "$map(scores, function($v) { $v * 2 }) & $map(scores, $string)",
            vec![],
        ),
        // The context value fills in a missing first argument.
        ("items.label.$uppercase()", vec![]),
        (
            "$uppercase(age)",
            vec![("W0205", message(1, "uppercase", "a number", "<s-:s>"))],
        ),
        (
            "$sum(tags)",
            vec![(
                "W0205",
                message(1, "sum", "an array of strings", "<a<n>:n>"),
            )],
        ),
        (
            "$sum(items.label)",
            vec![(
                "W0205",
                message(1, "sum", "a string or an array of strings", "<a<n>:n>"),
            )],
        ),
        (
            "$substring(name, 'x')",
            vec![("W0205", message(2, "substring", "a string", "<s-nn?:s>"))],
        ),
        (
            "$length($limit)",
            vec![("W0205", message(1, "length", "a number", "<s-:n>"))],
        ),
        (
            "$map(scores, 5)",
            vec![("W0205", message(2, "map", "a number", "<af>"))],
        ),
        (
            "$not(true) and $uppercase(1 = 1)",
            vec![("W0205", message(1, "uppercase", "a boolean", "<s-:s>"))],
        ),
        (
            "age ~> $join()",
            vec![("W0205", message(1, "join", "a number", "<a<s>s?:s>"))],
        ),
        // Declared functions are checked against their signature.
        ("$rate(name, age)", vec![]),
        (
            "$rate(age)",
            vec![("W0205", message(1, "rate", "a number", "<sn?:n>"))],
        ),
        // Partial applications and functions bound in the expression are not checked.
        ("$uppercase(?)(1)", vec![]),
        ("($length := function($x) { $x }; $length(1))", vec![]),
    ];
    for (text, expected) in &cases {
        let got: Vec<(String, String)> = svc
            .diagnostics(text)
            .into_iter()
            .map(|d| (d.code, d.message))
            .collect();
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(c, m)| ((*c).into(), m.clone()))
            .collect();
        assert_eq!(got, expected, "{text}");
    }
    // The warning points at the argument.
    let d = &svc.diagnostics("$uppercase(age)")[0];
    assert_eq!((d.span.start, d.span.end), (11, 14));
}

#[test]
fn diagnostics_report_operands_the_operator_cannot_take() {
    let schema = Schema::from_json_schema(&json!({
        "type": "object",
        "required": ["total"],
        "properties": {
            "total": {"type": "string"},
            "amount": {"type": "number"},
            "count": {"type": "integer"},
            "active": {"type": "string"},
            "flag": {"type": "boolean"},
            "note": {"type": ["string", "null"]},
            "code": {"type": ["string", "number"]},
            "kind": {"enum": ["a", "b"]},
            "tags": {"type": "array", "items": {"type": "string"}},
            "scores": {"type": "array", "items": {"type": "number"}},
            "grid": {"type": "array", "items": {"type": "array", "items": {"type": "number"}}},
            "customer": {"type": "object", "properties": {"name": {"type": "string"}}},
            "free": {}
        }
    }))
    .unwrap();
    let svc = LanguageService::new(Some(schema))
        .with_variable("limit", Some(Schema::infer(&[json!(5)]).root));
    let side = |code: &str, side: &str, op: &str, ty: &str| {
        let tail = if code == "W0207" {
            "which has no order"
        } else {
            "not a number"
        };
        (
            code.to_owned(),
            format!("The {side} side of {op} is {ty}, {tail}"),
        )
    };
    let sides = |op: &str, lhs: &str, rhs: &str| {
        (
            "W0208".to_owned(),
            format!("The sides of {op} are {lhs} and {rhs}, which cannot be compared"),
        )
    };
    let never = |value: &str, ty: &str| {
        (
            "W0210".to_owned(),
            format!("The value {value} never equals {ty}"),
        )
    };
    let cases: Vec<(&str, Vec<(String, String)>)> = vec![
        // Types the operator takes, and types that are not known.
        (
            "amount > 1000 and count <= $limit and total < 'm' and code > 1 and code < 'x'",
            vec![],
        ),
        ("amount + count * 2 - $limit / 4 % 3 + $sum(scores)", vec![]),
        (
            "free > 1 and free + 1 = 2 and missing > 1 and $lookup($, 'x') + 1",
            vec![(
                "W0201".to_owned(),
                "Field \"missing\" is not in the schema".to_owned(),
            )],
        ),
        // An array stands for its items, which `W0215` warns about on its own; arrays of
        // arrays are left.
        (
            "scores > 1 and scores + 1 = 2 and grid > 1 and grid + 1 = 2",
            vec![],
        ),
        // Order comparisons (`T2010`, `T2009` when evaluated).
        ("total > 1000", vec![sides(">", "a string", "a number")]),
        ("1000 <= total", vec![sides("<=", "a number", "a string")]),
        (
            "amount >= 'x' & ''",
            vec![sides(">=", "a number", "a string")],
        ),
        ("flag < 1", vec![side("W0207", "left", "<", "a boolean")]),
        (
            "1 < customer",
            vec![side("W0207", "right", "<", "an object")],
        ),
        (
            "tags > 1",
            vec![sides(">", "an array of strings", "a number")],
        ),
        (
            "(amount > 1) > (flag = true)",
            vec![
                side("W0207", "left", ">", "a boolean"),
                side("W0207", "right", ">", "a boolean"),
            ],
        ),
        ("note > 'a'", vec![]),
        ("note > 1", vec![sides(">", "null or a string", "a number")]),
        // Arithmetic (`T2001`, `T2002`).
        ("total + 1", vec![side("W0209", "left", "+", "a string")]),
        (
            "1 - (amount & '')",
            vec![side("W0209", "right", "-", "a string")],
        ),
        (
            "flag * customer",
            vec![
                side("W0209", "left", "*", "a boolean"),
                side("W0209", "right", "*", "an object"),
            ],
        ),
        (
            "tags / 2",
            vec![side("W0209", "left", "/", "an array of strings")],
        ),
        (
            "note % 2",
            vec![side("W0209", "left", "%", "null or a string")],
        ),
        ("code + 1", vec![]),
        // Negation (`D1002`).
        ("-amount + -scores + -code + -free + -$limit", vec![]),
        (
            "-total",
            vec![(
                "W0211".to_owned(),
                "The operand of - is a string, not a number".to_owned(),
            )],
        ),
        (
            "-(flag) + -tags",
            vec![
                (
                    "W0211".to_owned(),
                    "The operand of - is a boolean, not a number".to_owned(),
                ),
                (
                    "W0211".to_owned(),
                    "The operand of - is an array of strings, not a number".to_owned(),
                ),
            ],
        ),
        // A field compared with a literal of another type is always false.
        ("active = true", vec![never("true", "a string")]),
        ("amount != '1'", vec![never("'1'", "a number")]),
        (
            "count in [1, '2', null]",
            vec![never("'2'", "a number"), never("null", "a number")],
        ),
        (
            "note = null and note = 'x' and code = 1 and code = 'x'",
            vec![],
        ),
        ("tags = 'a' and 'a' in tags and scores = 1", vec![]),
        ("tags = 1", vec![never("1", "an array of strings")]),
        // A declared enumeration has its own warning, `W0206`; two literals are left alone, as
        // are values that are not literals.
        (
            "kind = 1",
            vec![(
                "W0206".to_owned(),
                "The value 1 is not one of the allowed values".to_owned(),
            )],
        ),
        ("'3' = 3 and 'a' in [1, 2]", vec![]),
        (
            "active = flag and amount = 1 + 1 and active = 'a'[false]",
            vec![],
        ),
    ];
    for (text, expected) in &cases {
        let got: Vec<(String, String)> = svc
            .diagnostics(text)
            .into_iter()
            .filter(|d| d.code != "W0215")
            .map(|d| (d.code, d.message))
            .collect();
        assert_eq!(&got, expected, "{text}");
    }
    // An operand's warning points at the operand, `W0208` at the comparison, `W0210` at the
    // literal.
    let span = |text: &str| {
        let d = &svc.diagnostics(text)[0];
        (d.span.start, d.span.end)
    };
    assert_eq!(span("1 + total"), (4, 9));
    assert_eq!(span("1 + -total"), (5, 10));
    assert_eq!(span("total > 1000"), (0, 12));
    assert_eq!(span("active = true"), (9, 13));
}

#[test]
fn diagnostics_report_operands_that_may_be_several_values() {
    let schema = Schema::from_json_schema(&json!({
        "type": "object",
        "required": ["items", "order", "meta", "tags"],
        "properties": {
            "items": {"type": "array", "items": {
                "type": "object",
                "required": ["qty", "price"],
                "properties": {
                    "qty": {"type": "number"},
                    "price": {"type": "number"},
                    "type": {"type": "string"}
                }
            }},
            "order": {"type": "object", "required": ["qty"], "properties": {"qty": {"type": "number"}}},
            "meta": {"type": "object", "properties": {"a": {"type": "number"}, "b": {"type": "number"}}},
            "tags": {"type": "array", "items": {"type": "string"}},
            "free": {}
        }
    }))
    .unwrap();
    let svc = LanguageService::new(Some(schema));
    let several = |side: &str, op: &str, cause: &str, hint: &str| {
        format!("The {side} of {op} may be several values, because of {cause}; {hint}")
    };
    let compare = |value: &str| {
        format!("compare one of them, such as `$max({value})`, or test each one in a filter")
    };
    let compute = |value: &str| {
        format!("compute one of them, such as `$sum({value})`, or each one inside a step")
    };
    let look = "to look for a value among them, write `in`";
    let cases: Vec<(&str, Vec<String>)> = vec![
        // A step over an array gives a sequence: an error for two items (`T2010`, `T2001`,
        // `D1002`), and never equal to one value.
        (
            "items.qty > 1",
            vec![several("left side", ">", "items", &compare("items.qty"))],
        ),
        (
            "1 + items.qty",
            vec![several("right side", "+", "items", &compute("items.qty"))],
        ),
        (
            "-items.qty",
            vec![several("operand", "-", "items", &compute("items.qty"))],
        ),
        (
            "items.qty = 2",
            vec![several("left side", "=", "items", look)],
        ),
        (
            "2 != items.qty",
            vec![several("right side", "!=", "items", look)],
        ),
        (
            "items.qty in [1, 2]",
            vec![several(
                "left side",
                "in",
                "items",
                "test each one in a filter",
            )],
        ),
        // A filter that may keep several items, the fields of an object, descendants and keys.
        (
            "items[type = 'a'].price > 1",
            vec![several(
                "left side",
                ">",
                "items[type = 'a']",
                &compare("items[type = 'a'].price"),
            )],
        ),
        ("meta.* = 1", vec![several("left side", "=", "*", look)]),
        ("**.qty = 1", vec![several("left side", "=", "**", look)]),
        (
            "$keys(meta) = 'a'",
            vec![several("left side", "=", "$keys(meta)", look)],
        ),
        // An array is an array even of one item: `["a"] > "a"` fails, `["a"] = "a"` is false.
        (
            "tags > 'a'",
            vec![several("left side", ">", "tags", &compare("tags"))],
        ),
        ("tags = 'a'", vec![several("left side", "=", "tags", look)]),
        // One value: a number filter, a filter of one object, an aggregate, a step's body, a
        // predicate, `in` with the sequence on the right.
        ("items[0].qty > 1 and items[-1].qty + 1 = 2", vec![]),
        (
            "items[qty > 1][0].price > 1 and items[0][qty > 1].price > 1",
            vec![],
        ),
        ("order[qty > 1].qty + 1 = 2", vec![]),
        ("$sum(items.qty) > 1 and $count(items[qty > 1]) > 0", vec![]),
        ("items.(qty * price) and items[qty > 1]", vec![]),
        ("2 in items.qty and 'a' in tags", vec![]),
        // Two sides that may be arrays, or a side whose type is not known, are compared whole.
        (
            "items.qty = items.price and tags = ['a'] and items.qty = free",
            vec![],
        ),
    ];
    for (text, expected) in &cases {
        let got: Vec<String> = svc
            .diagnostics(text)
            .into_iter()
            .map(|d| {
                assert_eq!(d.code, "W0215", "{text}");
                d.message
            })
            .collect();
        assert_eq!(&got, expected, "{text}");
    }
    // The warning points at the operand; nothing is known without a schema.
    let d = &svc.diagnostics("1 < items[0].qty + items.qty")[0];
    assert_eq!((d.span.start, d.span.end), (19, 28));
    assert!(
        LanguageService::new(None)
            .diagnostics("items.qty > 1 and items.qty = 2")
            .is_empty()
    );
    // A schema inferred from a sample with one item still has an array: a second item fails.
    let inferred = Schema::infer(&[json!({"Order": [{"Product": [{"Price": 1}]}]})]);
    let codes: Vec<String> = LanguageService::new(Some(inferred))
        .diagnostics("Order.Product.Price > 0")
        .into_iter()
        .map(|d| d.code)
        .collect();
    assert_eq!(codes, ["W0215"]);
}

/// Every operand built from these parts, under each operator: `W0215` is reported exactly where
/// the operand is an array on one of the documents that follow the schema.
#[test]
fn several_values_are_warned_about_exactly_where_an_operand_is_an_array() {
    let schema = Schema::from_json_schema(&json!({
        "type": "object",
        "required": ["items", "order", "meta", "tags"],
        "properties": {
            "items": {"type": "array", "minItems": 1, "items": {
                "type": "object",
                "required": ["qty", "price", "type"],
                "properties": {
                    "qty": {"type": "number"},
                    "price": {"type": "number"},
                    "type": {"type": "string"}
                }
            }},
            "order": {"type": "object", "required": ["qty"], "properties": {"qty": {"type": "number"}}},
            "meta": {"type": "object", "required": ["a"], "properties": {"a": {"type": "number"}, "b": {"type": "number"}}},
            "tags": {"type": "array", "minItems": 1, "items": {"type": "string"}}
        }
    }))
    .unwrap();
    // One item of each, and two: every filter below keeps both.
    let documents = [
        json!({"items": [{"qty": 2, "price": 3, "type": "a"}], "order": {"qty": 2},
               "meta": {"a": 1}, "tags": ["a"]}),
        json!({"items": [{"qty": 2, "price": 3, "type": "a"}, {"qty": 5, "price": 1, "type": "a"}],
               "order": {"qty": 2}, "meta": {"a": 1, "b": 2}, "tags": ["a", "b"]}),
    ];
    let svc = LanguageService::new(Some(schema));
    let bases = [
        "items",
        "items[0]",
        "items[-1]",
        "items[qty > 1]",
        "items[type = 'a']",
        "items[qty > 1][0]",
        "order",
        "order[qty > 1]",
        "meta",
        "tags",
        "tags[0]",
        "**",
        "$keys(meta)",
        "$keys(meta)[0]",
        "(items)",
        "$",
    ];
    let tails = ["", ".qty", ".price", ".*", ".type", ".qty[0]"];
    let options = jlens_core::Options::default();
    let bindings = jlens_core::Bindings::new();
    let mut warned = 0;
    for base in bases {
        for tail in tails {
            let operand = format!("{base}{tail}");
            let expr = jlens_core::Expression::compile(&operand).unwrap();
            let array = documents.iter().any(|d| {
                matches!(
                    expr.evaluate_with(Some(d), &bindings, &options),
                    Ok(Some(Value::Array(_)))
                )
            });
            for text in [
                format!("{operand} > 1"),
                format!("1 + {operand}"),
                format!("-{operand}"),
                format!("{operand} = 1"),
                format!("{operand} in [1]"),
            ] {
                let found = svc.diagnostics(&text).iter().any(|d| d.code == "W0215");
                assert_eq!(found, array, "{text}");
                warned += usize::from(found);
            }
        }
    }
    assert!(warned > 100, "{warned}");
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

/// An order: `total` always there, `notes` optional, `status` a string or null, `items` an
/// array that may be empty, `lines` one with at least one item.
fn order_service() -> LanguageService {
    let schema = Schema::from_json_schema(&json!({
        "type": "object",
        "required": ["total", "status", "items", "lines"],
        "properties": {
            "total": {"type": "number"},
            "notes": {"type": "string"},
            "status": {"type": ["string", "null"]},
            "items": {"type": "array", "items": {
                "type": "object",
                "required": ["price"],
                "properties": {"price": {"type": "number"}, "type": {"type": "string"}}
            }},
            "lines": {"type": "array", "minItems": 1, "items": {
                "type": "object",
                "required": ["price"],
                "properties": {"price": {"type": "number"}}
            }}
        }
    }))
    .unwrap();
    LanguageService::new(Some(schema))
}

/// The code of each diagnostic of `text` in `svc`, with the text it points at.
fn located(svc: &LanguageService, text: &str) -> Vec<(String, String)> {
    svc.diagnostics(text)
        .into_iter()
        .map(|d| (d.code, text[d.span.start..d.span.end].to_owned()))
        .collect()
}

#[test]
fn diagnostics_compare_the_result_with_the_expected_type() {
    use jlens_schema::JsonType::{Array, Boolean, Number};
    let boolean = order_service().with_result_type([Boolean]);
    for (text, expected) in [
        // Always a boolean.
        ("total > 100", vec![]),
        ("total > 100 and $contains(notes, 'x')", vec![]),
        ("status = 'open' or $exists(notes)", vec![]),
        ("$count(items[type = 'a']) > 0", vec![]),
        // `lines` is never empty: its sum, and its largest price, are always there.
        ("$sum(lines.price) > 100", vec![]),
        ("$max(lines.price) > 100", vec![]),
        // Never a boolean.
        ("total", vec![("W0212", "total")]),
        ("lines.price", vec![("W0212", "lines.price")]),
        // A boolean or undefined, because of the operand that may give nothing.
        ("$contains(notes, 'x')", vec![("W0214", "notes")]),
        ("$sum(items.price) > 100", vec![("W0214", "items")]),
        ("$max(items.price) > 100", vec![("W0214", "items")]),
        // Several prices fail the comparison, `W0215`.
        (
            "items[type = 'a'].price > 1",
            vec![
                ("W0215", "items[type = 'a'].price"),
                ("W0214", "items[type = 'a']"),
            ],
        ),
        ("total > 1 ? true", vec![("W0214", "total > 1")]),
        // A boolean or another type.
        (
            "total > 1 ? true : status",
            vec![("W0213", "total > 1 ? true : status")],
        ),
        // A syntax error leaves the result alone.
        ("total >", vec![("S0207", "")]),
    ] {
        let found = located(&boolean, text);
        let found: Vec<(&str, &str)> = found
            .iter()
            .map(|(c, s)| (c.as_str(), s.as_str()))
            .collect();
        assert_eq!(found, expected, "{text}");
    }

    let number = order_service().with_result_type([Number]);
    assert_eq!(located(&number, "$sum(lines.price)"), []);
    // An integer is a number; several prices, gathered by the step over `lines`, an array.
    assert_eq!(located(&number, "$count(items)"), []);
    assert_eq!(
        located(&number, "lines.price"),
        [("W0213".to_owned(), "lines".to_owned())]
    );
    assert_eq!(
        number.diagnostics("lines.price")[0].message,
        "The result may be an array, not a number"
    );
    let array = order_service().with_result_type([Array]);
    for text in [
        "items",
        "[total]",
        "$sort(lines.price)",
        "$split(status, ',') ?? []",
    ] {
        assert_eq!(located(&array, text), [], "{text}");
    }
    assert_eq!(
        array.diagnostics("lines.price")[0].message,
        "The result may be a number, not an array"
    );
}

#[test]
fn result_type_tells_what_the_typing_knows() {
    use jlens_schema::JsonType::{Array, Boolean, Null, Number, String};
    use jlens_schema::ResultCheck::{Always, Maybe, Never, Unknown};
    let svc = order_service();
    let text = "lines.price";
    let result = svc.result_type(text);
    assert_eq!(result.types, Some(vec![Number, Array]));
    assert_eq!(result.undefined, None);
    let at = result.sequence.unwrap();
    assert_eq!(&text[at.start..at.end], "lines");
    assert_eq!(result.check(&[Number, Array]), Always);
    assert_eq!(result.check(&[Number]), Maybe);
    assert_eq!(result.check(&[Boolean]), Never);

    assert_eq!(svc.result_type("status").types, Some(vec![Null, String]));
    // `$x`, not declared, an object constructor and a tuple stream (`items@$v` gives the order,
    // once for each item) are not known, nor is a text that does not parse.
    for text in ["$x > 1", "{'a': total}", "items@$v", "items#$i", "total >"] {
        assert_eq!(svc.result_type(text).check(&[Boolean]), Unknown, "{text}");
    }
    // Without a schema, neither are the fields.
    let none = LanguageService::new(None);
    assert_eq!(none.result_type("a > 1").check(&[Boolean]), Unknown);
    assert_eq!(none.result_type("a = 1").check(&[Boolean]), Always);
}

#[test]
fn an_expression_is_typed_as_the_body_of_a_step() {
    use jlens_schema::JsonType::Boolean;
    use jlens_schema::ResultCheck::Always;
    // Each item of the order, as inside `items.( … )`: `%` is the order.
    let svc = service().with_context("items[]");
    let cases: &[(&str, &[&str])] = &[
        ("|", ITEM_FIELDS),
        ("t|", &["type", "tax"]),
        ("%.|", &["customer", "items"]),
        ("%.customer.|", &["login", "name"]),
        ("%.items[%.c|]", &["customer"]),
        ("$$.cus|", &["customer"]),
        ("`t|", &["type", "tax"]),
    ];
    for (marked, expected) in cases {
        let got = labels(&svc, marked, Some(CompletionKind::Field));
        assert_eq!(got, *expected, "{marked}");
    }
    // `items` and `items[]` are the same context: a step reads the items of an array.
    assert_eq!(
        labels(
            &service().with_context("items"),
            "|",
            Some(CompletionKind::Field)
        ),
        ITEM_FIELDS
    );

    // The diagnostics are those of the body inside `items.( … )`, at the text's offsets.
    let wrapped = service();
    for body in [
        "type = 'toy' and %.customer.login = 'a'",
        "customer",
        "%.custmer",
        "type = 'toys'",
        "$sum(%.items.unitPrice) > quantity",
        "unitPrice > 'a'",
        "%.items.unitPrice > 1",
        "$$.customer.nick",
        "%.items.(%.customer)",
    ] {
        let text = format!("items.({body})");
        let expected: Vec<(String, usize, usize)> = wrapped
            .diagnostics(&text)
            .into_iter()
            .map(|d| (d.code, d.span.start - 7, d.span.end - 7))
            .collect();
        let found: Vec<(String, usize, usize)> = svc
            .diagnostics(body)
            .into_iter()
            .map(|d| (d.code, d.span.start, d.span.end))
            .collect();
        assert_eq!(found, expected, "{body}");
    }
    let codes = |svc: &LanguageService, text: &str| -> Vec<String> {
        svc.diagnostics(text).into_iter().map(|d| d.code).collect()
    };
    assert_eq!(codes(&svc, "customer"), ["W0201"]);
    assert_eq!(codes(&svc, "%.custmer"), ["W0201"]);
    // Past the document, as without a context.
    assert_eq!(codes(&svc, "%.%.x"), ["S0217"]);
    assert_eq!(codes(&service(), "%.customer"), ["S0217"]);

    // Two steps: `%` is the order, `%.%` the document.
    let schema = Schema::infer(&[json!({
        "region": "south",
        "orders": [{"total": 10, "items": [{"price": 1}]}]
    })]);
    let nested = LanguageService::new(Some(schema)).with_context("orders[].items[]");
    assert_eq!(
        labels(&nested, "|", None).first().map(String::as_str),
        Some("price")
    );
    assert_eq!(
        labels(&nested, "%.|", Some(CompletionKind::Field)),
        ["total", "items"]
    );
    assert_eq!(
        labels(&nested, "%.%.|", Some(CompletionKind::Field)),
        ["region", "orders"]
    );
    assert!(
        nested
            .diagnostics("price < %.total and %.%.region = 'south'")
            .is_empty()
    );
    assert_eq!(codes(&nested, "%.region"), ["W0201"]);
    assert_eq!(codes(&nested, "%.%.%"), ["S0217"]);

    // The item is always there: a condition on its required fields is always a boolean.
    let boolean = order_service()
        .with_context("lines[]")
        .with_result_type([Boolean]);
    assert_eq!(located(&boolean, "price > 10 and %.total > 100"), []);
    assert_eq!(
        boolean.result_type("price > %.total").check(&[Boolean]),
        Always
    );
    assert_eq!(
        located(&boolean, "$contains(%.notes, 'x')"),
        [("W0214".to_owned(), "notes".to_owned())]
    );

    // A path the schema does not have: an unknown context, nothing warned; its parent, the
    // document, is known.
    let unknown = service().with_context("nothing[]");
    assert!(unknown.diagnostics("anything.at.all").is_empty());
    assert!(unknown.diagnostics("%.customer.name").is_empty());
    assert_eq!(codes(&unknown, "%.anything"), ["W0201"]);
}
