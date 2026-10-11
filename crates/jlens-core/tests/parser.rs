//! Parser tests: structure of the processed tree, recovery (ported from jsonata-js
//! `test/parser-recovery.js`), and audit annotations.

use jlens_core::ast::{Node, NodeKind, Stage};
use jlens_core::{Severity, parse, parse_recover};

/// A compact, span-free rendering of a tree, used to compare structures.
fn outline(n: &Node) -> String {
    let join = |nodes: &[Node], sep: &str| nodes.iter().map(outline).collect::<Vec<_>>().join(sep);
    let mut s = match &n.kind {
        NodeKind::Path {
            steps,
            keep_singleton_array,
        } => {
            format!(
                "path({}){}",
                join(steps, ", "),
                if *keep_singleton_array { "[]" } else { "" }
            )
        }
        NodeKind::Binary { op, lhs, rhs } => {
            format!("({} {} {})", outline(lhs), op.symbol(), outline(rhs))
        }
        NodeKind::Negate { expr } => format!("-{}", outline(expr)),
        NodeKind::Array { items } => format!("[{}]", join(items, ", ")),
        NodeKind::Object { pairs } => format!(
            "{{{}}}",
            pairs
                .iter()
                .map(|(k, v)| format!("{}: {}", outline(k), outline(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        NodeKind::Name { name } => name.clone(),
        NodeKind::Variable { name } => format!("${name}"),
        NodeKind::String { value } => format!("{value:?}"),
        NodeKind::Number { value } => value.to_string(),
        NodeKind::Bool { value } => value.to_string(),
        NodeKind::Null => "null".into(),
        NodeKind::Regex { pattern, flags } => format!("/{pattern}/{flags}"),
        NodeKind::Wildcard => "*".into(),
        NodeKind::Descendant => "**".into(),
        NodeKind::Parent { slot, .. } => format!("%{slot}"),
        NodeKind::Condition {
            condition,
            then,
            otherwise,
        } => match otherwise {
            Some(e) => format!(
                "({} ? {} : {})",
                outline(condition),
                outline(then),
                outline(e)
            ),
            None => format!("({} ? {})", outline(condition), outline(then)),
        },
        NodeKind::Fallback { op, lhs, rhs } => {
            format!("({} {} {})", outline(lhs), op.symbol(), outline(rhs))
        }
        NodeKind::Block { exprs } => format!("({})", join(exprs, "; ")),
        NodeKind::Bind { lhs, rhs } => format!("({} := {})", outline(lhs), outline(rhs)),
        NodeKind::Apply { lhs, rhs } => format!("({} ~> {})", outline(lhs), outline(rhs)),
        NodeKind::Function { procedure, args } => {
            format!("{}({})", outline(procedure), join(args, ", "))
        }
        NodeKind::Partial { procedure, args } => {
            format!("partial {}({})", outline(procedure), join(args, ", "))
        }
        NodeKind::Placeholder => "?".into(),
        NodeKind::Lambda(def) => {
            let (params, body, signature) = (&def.params, &def.body, &def.signature);
            if def.thunk {
                format!("thunk{{{}}}", outline(body))
            } else {
                let sig = signature
                    .as_ref()
                    .map_or(String::new(), |s| s.definition.clone());
                let params: Vec<_> = params.iter().map(|p| format!("${p}")).collect();
                format!("λ({}){sig}{{{}}}", params.join(", "), outline(body))
            }
        }
        NodeKind::Transform(def) => format!(
            "|{}|{}{}|",
            outline(&def.pattern),
            outline(&def.update),
            def.delete
                .as_ref()
                .map_or(String::new(), |d| format!(", {}", outline(d)))
        ),
        NodeKind::Sort { terms } => format!(
            "^({})",
            terms
                .iter()
                .map(|t| format!(
                    "{}{}",
                    if t.descending { ">" } else { "<" },
                    outline(&t.expr)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        NodeKind::Error { error } => format!("!{}", error.code()),
    };
    if let Some(focus) = &n.attrs.focus {
        s.push_str(&format!("@${focus}"));
    }
    if let Some(index) = &n.attrs.index {
        s.push_str(&format!("#${index}"));
    }
    for (stages, prefix) in [(&n.attrs.stages, ""), (&n.attrs.predicate, "?")] {
        for stage in stages {
            match stage {
                Stage::Filter { expr, .. } => s.push_str(&format!("{prefix}[{}]", outline(expr))),
                Stage::Index { var, .. } => s.push_str(&format!("#${var}")),
            }
        }
    }
    if let Some(group) = &n.attrs.group {
        let pairs: Vec<_> = group
            .pairs
            .iter()
            .map(|(k, v)| format!("{}: {}", outline(k), outline(v)))
            .collect();
        s.push_str(&format!("{{{}}}", pairs.join(", ")));
    }
    if n.attrs.keep_array && !matches!(n.kind, NodeKind::Path { .. }) {
        s.push_str("[]");
    }
    if let Some(a) = &n.annotation {
        s = format!("«{}»{s}", a.label.as_deref().unwrap_or("?"));
    }
    s
}

fn tree(src: &str) -> String {
    outline(&parse(src).unwrap_or_else(|e| panic!("{src}: {e}")).root)
}

fn error_code(src: &str) -> String {
    parse(src)
        .map(|_| "ok".to_owned())
        .unwrap_or_else(|e| e.code().to_owned())
}

#[test]
fn paths_predicates_and_constructors() {
    let cases = [
        (
            "Account.Order[0].Product",
            "path(Account, Order[0], Product)",
        ),
        (
            "Account.Order.Product[]",
            "path(Account, Order, Product[])[]",
        ),
        ("$.a.\"b c\"", "path($, a, b c)"),
        ("[1, 2..4][0]", "[1, (2 .. 4)]?[0]"),
        ("{\"a\": 1, \"b\": [x]}", "{\"a\": 1, \"b\": [path(x)]}"),
        ("-5 + -x", "(-5 + -path(x))"),
        ("a.b{c: d}", "path(a, b){path(c): path(d)}"),
        (
            "Order^(>Price, Name)",
            "path(Order, ^(>path(Price), <path(Name)))",
        ),
        ("Order@$o.Product#$i", "path(Order@$o, Product#$i)"),
        ("Order[0]#$i", "path(Order[0]#$i)"),
        ("x ? 1 : 2", "(path(x) ? 1 : 2)"),
        ("x ?: 2", "(path(x) ?: 2)"),
        ("x ?? 2", "(path(x) ?? 2)"),
        ("x ?? y ?: 2", "(path(x) ?? (path(y) ?: 2))"),
        ("($a := 1; $a ~> $string)", "(($a := 1); ($a ~> $string))"),
        ("$f(?, 2)", "partial $f(?, 2)"),
        (
            "| a | {\"b\": 1}, [\"c\"] |",
            "|path(a)|{\"b\": 1}, [\"c\"]|",
        ),
        ("/ab+c/i", "/ab+c/i"),
        ("and.or.in", "path(and, or, in)"),
        (
            "a = 1 and b != 2 or c in [3]",
            "(((path(a) = 1) and (path(b) != 2)) or (path(c) in [3]))",
        ),
    ];
    for (src, expected) in cases {
        assert_eq!(tree(src), expected, "{src}");
    }
}

#[test]
fn lambdas_signatures_and_tail_calls() {
    assert_eq!(
        tree("function($n)<n:n>{ $n <= 1 ? 1 : $n * $fact($n - 1) }"),
        "λ($n)<n:n>{((path($n) <= 1) ? 1 : (path($n) * $fact((path($n) - 1))))}"
            .replace("path($n)", "$n")
    );
    // A call in tail position becomes a thunk for the trampoline.
    assert_eq!(
        tree("λ($n, $acc){ $n = 0 ? $acc : $loop($n - 1, $acc * $n) }"),
        "λ($n, $acc){(($n = 0) ? $acc : thunk{$loop(($n - 1), ($acc * $n))})}"
    );
    assert_eq!(error_code("function(x){1}"), "S0208");
    assert_eq!(error_code("function($x)<n<s>>{1}"), "S0401");
    assert_eq!(error_code("function($x)<(sa<n>)>{1}"), "S0402");
}

#[test]
fn parent_operator_resolves_ancestors() {
    let ast = parse("Account.Order.Product.{ 'order': %.OrderID }").unwrap();
    let mut ancestors = Vec::new();
    ast.root.walk(&mut |n| {
        if let Some(slot) = n.attrs.ancestor {
            ancestors.push((outline(n), ast.slot_label(slot).to_owned()));
        }
    });
    assert_eq!(ancestors, vec![("Product".to_owned(), "!0".to_owned())]);
    assert_eq!(error_code("%.a"), "S0217");
}

#[test]
fn syntax_errors() {
    let cases = [
        ("", "S0207"),
        ("a +", "S0207"),
        ("(a", "S0203"),
        ("[1, 2", "S0203"),
        ("a)", "S0201"),
        ("a !b", "S0204"),
        (")", "S0211"),
        ("1 := 2", "S0212"),
        ("a.1", "S0213"),
        ("a.true", "S0213"),
        ("a@b", "S0214"),
        ("a[0]@$x", "S0215"),
        ("a^(b)@$x", "S0216"),
        ("[1,2,3]{\"num\": $}[true]", "S0209"),
        ("[1,2,3]{\"num\": $}{\"num\": $}", "S0210"),
        ("'abc", "S0101"),
        ("1e400", "S0102"),
        ("/*", "S0106"),
        ("`a", "S0105"),
        ("//", "S0301"),
        ("/abc", "S0302"),
    ];
    for (src, code) in cases {
        assert_eq!(error_code(src), code, "{src:?}");
    }
}

/// The recovery cases of jsonata-js `test/parser-recovery.js`: same error codes and positions
/// (UTF-16 offsets just past the offending token), and the same partial tree.
#[test]
fn recovery_matches_jsonata_js() {
    type Case<'a> = (&'a str, &'a str, &'a [(&'a str, usize)]);
    let cases: &[Case] = &[
        ("Account.Order[0]", "path(Account, Order[0])", &[]),
        ("Account.", "path(Account, !S0207)", &[("S0207", 8)]),
        (
            "Account[",
            "path(Account[!S0207])",
            &[("S0203", 8), ("S0207", 8)],
        ),
        (
            "Account.Order[;0].Product",
            "path(Account, Order[!S0211])",
            &[("S0211", 15), ("S0202", 16)],
        ),
        (
            "Account.Order[0;].Product",
            "path(Account, Order[0])",
            &[("S0202", 16)],
        ),
        (
            "Account.Order[0].Product;",
            "path(Account, Order[0], Product)",
            &[("S0201", 25)],
        ),
        (
            "$inputSource[0].UnstructuredAnswers^()[0].Text",
            "path($inputSource?[0], UnstructuredAnswers, ^(<!S0211?[!S0207]))",
            &[("S0211", 38), ("S0203", 46), ("S0203", 46), ("S0207", 46)],
        ),
    ];
    for (src, expected_tree, expected_errors) in cases {
        let ast = parse_recover(src);
        assert_eq!(outline(&ast.root), *expected_tree, "{src}");
        // The sources are ASCII, so the byte offset at the end of a span equals the UTF-16
        // position jsonata-js reports.
        let errors: Vec<(&str, usize)> = ast
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| (d.code.as_str(), d.span.end))
            .collect();
        assert_eq!(errors, *expected_errors, "{src}");
    }
}

#[test]
fn recovery_survives_lexer_errors() {
    let ast = parse_recover("items[name = 'abc");
    assert_eq!(ast.diagnostics[0].code, "S0101");
    assert_eq!(outline(&ast.root), "path(items[(path(name) = !S0101)])");
}

#[test]
fn annotations_attach_to_the_following_operand() {
    let src = r#"
        /** @label "authorized" */ customer.login in $approved
        and /** @label "total" */ $sum(items.price) > 50000
        or /** @label "fallback" */ $override
    "#;
    assert_eq!(
        tree(src),
        "((«authorized»(path(customer, login) in $approved) and «total»($sum(path(items, price)) > 50000)) or «fallback»$override)"
    );
}

#[test]
fn annotations_never_change_the_tree() {
    let annotated = [
        "a - /** @label \"x\" */ b + c",
        "/** @label \"x\" */ $x := 1 + 2",
        "/** @label \"x\" */ a > 1 ? 'y' : 'n'",
        "f(/** @label \"x\" */ a.b, c)",
        "a./** @label \"x\" */ b.c",
    ];
    let strip = |s: &str| {
        let mut out = String::new();
        let mut rest = s;
        while let Some(i) = rest.find("/**") {
            out.push_str(&rest[..i]);
            rest = &rest[i + rest[i..].find("*/").unwrap() + 2..];
        }
        out + rest
    };
    let unlabel = |s: String| s.replace("«x»", "");
    for src in annotated {
        assert_eq!(unlabel(tree(src)), tree(&strip(src)), "{src}");
        assert!(tree(src).contains("«x»"), "{src} lost its annotation");
    }
    assert_eq!(
        tree("a - /** @label \"x\" */ b + c"),
        "((path(a) - path(«x»b)) + path(c))"
    );
    assert_eq!(
        tree("/** @label \"x\" */ a > 1 ? 'y' : 'n'"),
        "(«x»(path(a) > 1) ? \"y\" : \"n\")"
    );
}

#[test]
fn misplaced_annotations_are_warnings() {
    let ast = parse("a /** @label \"x\" */ and b /** @label \"y\" */").unwrap();
    let codes: Vec<_> = ast.diagnostics.iter().map(|d| d.code.as_str()).collect();
    assert_eq!(codes, ["W0101", "W0101"]);
    assert!(
        ast.diagnostics
            .iter()
            .all(|d| d.severity == Severity::Warning)
    );
}

#[test]
fn spans_cover_whole_expressions() {
    let src = "Account.Order[0].Product + 1";
    let ast = parse(src).unwrap();
    assert_eq!(&src[ast.root.span.start..ast.root.span.end], src);
    let NodeKind::Binary { lhs, .. } = &ast.root.kind else {
        panic!()
    };
    assert_eq!(
        &src[lhs.span.start..lhs.span.end],
        "Account.Order[0].Product"
    );
    let NodeKind::Path { steps, .. } = &lhs.kind else {
        panic!()
    };
    assert_eq!(&src[steps[1].span.start..steps[1].span.end], "Order[0]");
}

#[test]
fn node_ids_are_unique_and_preordered() {
    let ast = parse("a.b[c > 1]{d: e}").unwrap();
    let mut ids = Vec::new();
    ast.root.walk(&mut |n| ids.push(n.id));
    assert_eq!(ids, (0..ids.len() as u32).collect::<Vec<_>>());
}

/// Every expression of the official suite must parse, except those whose expected error is a
/// static (`S0xxx`) one, which must fail with exactly that code.
#[test]
fn official_suite_expressions_parse() {
    let groups =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/jsonata-suite/groups");
    let mut failures = Vec::new();
    let mut checked = 0;
    for group in std::fs::read_dir(&groups).unwrap() {
        let group = group.unwrap().path();
        for file in std::fs::read_dir(&group).unwrap() {
            let file = file.unwrap().path();
            if file.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Ok(spec) =
                serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&file).unwrap())
            else {
                continue; // lone surrogates; see tests/conformance/overrides.json
            };
            let cases = match spec {
                serde_json::Value::Array(items) => items,
                single => vec![single],
            };
            for case in cases {
                let expr = match (case.get("expr"), case.get("expr-file")) {
                    (Some(e), _) => e.as_str().unwrap().to_owned(),
                    (_, Some(f)) => {
                        std::fs::read_to_string(group.join(f.as_str().unwrap())).unwrap()
                    }
                    _ => continue,
                };
                let code = case
                    .get("code")
                    .or_else(|| case.get("error").and_then(|e| e.get("code")))
                    .and_then(|c| c.as_str())
                    .filter(|c| c.starts_with('S'));
                let got = parse(&expr).map(|_| ()).map_err(|e| e.code().to_owned());
                checked += 1;
                let ok = match (code, &got) {
                    (Some(want), Err(code)) => want == code,
                    (None, Ok(())) => true,
                    _ => false,
                };
                if !ok {
                    failures.push(format!(
                        "{}: {expr:?} expected {code:?}, got {got:?}",
                        file.display()
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} cases:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The annotations in the shipping-rule fixture label exactly the conditions they precede.
#[test]
fn fixture_annotations_cover_their_conditions() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/shipping-rule.jsonata");
    let src = std::fs::read_to_string(path).unwrap();
    let ast = parse(&src).unwrap();
    assert!(ast.diagnostics.is_empty(), "{:?}", ast.diagnostics);
    let mut labelled = Vec::new();
    ast.root.walk(&mut |n| {
        if let Some(a) = &n.annotation {
            let text = src[n.span.start..n.span.end]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            labelled.push((a.label.clone().unwrap(), text));
        }
    });
    let labelled: Vec<(&str, &str)> = labelled
        .iter()
        .map(|(l, t)| (l.as_str(), t.as_str()))
        .collect();
    assert_eq!(
        labelled,
        [
            (
                "Authorized customer",
                r#"customer.login in ["alice.walker99", "bob.harrison42"]"#
            ),
            (
                "Customer not blocked",
                r#"$not(customer.login in ["mallory.cheat1", "eve.spammer007"])"#
            ),
            ("Minimum order grand total", "$total > 50000"),
            (
                "Order category check",
                r#"( /** @label "Electronics group" */ ('electronic' in items.type and $electronics > $total * 0.30 + 1000) or /** @label "Blue items group" */ ('blue' in items.color and $blueTax < $averageTax) )"#
            ),
            (
                "Electronics group",
                "('electronic' in items.type and $electronics > $total * 0.30 + 1000)"
            ),
            (
                "Blue items group",
                "('blue' in items.color and $blueTax < $averageTax)"
            ),
        ]
    );
}

#[test]
fn deep_nesting_is_an_error_not_a_crash() {
    let nested = |n: usize| format!("{}1{}", "(".repeat(n), ")".repeat(n));
    // The limit is generous for real expressions.
    let ok = jlens_core::Expression::compile(&nested(999)).unwrap();
    assert_eq!(
        ok.evaluate(None, &jlens_core::Bindings::new()).unwrap(),
        Some(1.into())
    );
    let err = jlens_core::Expression::compile(&nested(5000)).unwrap_err();
    assert_eq!(err.code(), "U1001");
    assert!(
        err.message().contains("nested too deeply"),
        "{}",
        err.message()
    );
    let ast = jlens_core::parse_recover(&format!("{}a", "[".repeat(5000)));
    assert_eq!(ast.diagnostics[0].code, "U1001");
}

#[test]
fn long_operator_chains_count_towards_the_nesting_limit() {
    // Each operator of a chain adds a level to the tree, which is walked and dropped recursively.
    let chain = |step: &str, n: usize| format!("1{}", step.repeat(n));
    let ok = jlens_core::Expression::compile(&chain("+1", 998)).unwrap();
    assert_eq!(
        ok.evaluate(None, &jlens_core::Bindings::new()).unwrap(),
        Some(999.into())
    );
    for step in ["+1", " and 1", ".a", "[0]", "~>$f"] {
        let source = chain(step, 5000);
        let err = jlens_core::Expression::compile(&source).unwrap_err();
        assert_eq!(err.code(), "U1001", "{step}");
        let ast = jlens_core::parse_recover(&source);
        assert_eq!(ast.diagnostics[0].code, "U1001", "{step}");
    }
}

#[test]
fn recovery_moves_past_lexer_errors() {
    // Found by the property tests: an out-of-range number made recovery loop forever.
    for source in [
        "1e310",
        "a + 1e310 + b",
        "$f(1e400, ",
        "\"unterminated",
        "/* open",
        "`open",
    ] {
        let ast = jlens_core::parse_recover(source);
        assert!(ast.has_errors(), "{source}");
    }
    assert_eq!(
        jlens_core::parse_recover("1e310").diagnostics[0].code,
        "S0102"
    );
}
