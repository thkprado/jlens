//! Property tests: no input, however malformed, makes the parser or the evaluator panic.

use std::time::Duration;

use jlens_core::{Bindings, CancelToken, Document, Error, Expression, HostFunction, Options};
use proptest::prelude::*;
use serde_json::json;

/// Fragments of JSONata, so that random expressions are often nearly valid and reach deep into
/// the parser and the evaluator.
#[rustfmt::skip]
const TOKENS: &[&str] = &[
    "a", "b", "items", "price", "$", "$$", "$x", "$i", "1", "0", "-2.5", "1e3", "'s'", "\"t\"",
    "true", "null", ".", "[", "]", "(", ")", "{", "}", ":", ",", ";", "+", "-", "*", "/", "%",
    "=", "!=", "<", "<=", ">", "and", "or", "in", "?", "??", "?:", ":=", "~>", "&", "..", "**",
    "^(", ">price", "@$v", "#$j", "|", "function($v){", "λ($w){", "$sum(", "$map(", "$string(",
    "$substring(", "$match(", "$replace(", "$split(", "$formatNumber(", "$fromMillis(", "$keys(",
    "$merge(", "$sort(", "$reduce(", "$each(", "$eval(", "$echo(", "$checked(", "$fail(", "$echo(?, ", "/a+/", "/[/", "`a b`", "/* c */",
    "/** @label \"x\" */", "<s:s>", "%", "\\", "é", "𝄞", " ",
];

/// Syntactically valid expressions, built from a small grammar, so that the evaluator is
/// exercised with every kind of node and with values of unexpected types.
fn valid() -> impl Strategy<Value = String> {
    let leaf = prop::sample::select(
        &[
            "a",
            "b",
            "items",
            "price",
            "$",
            "$$",
            "$x",
            "1",
            "0",
            "-2.5",
            "'s'",
            "true",
            "null",
            "[]",
            "{}",
            "items.price",
            "a.b",
            "$x[1]",
            "s",
            "$now()",
            "$random()",
        ][..],
    )
    .prop_map(str::to_owned);
    leaf.prop_recursive(4, 48, 3, |inner| {
        let ops = prop::sample::select(
            &[
                "+", "-", "*", "/", "%", "=", "!=", "<", ">=", "and", "or", "in", "&", "..", "??",
                "?:",
            ][..],
        );
        let functions = prop::sample::select(
            &[
                "$string",
                "$number",
                "$sum",
                "$count",
                "$max",
                "$keys",
                "$reverse",
                "$sort",
                "$distinct",
                "$uppercase",
                "$length",
                "$boolean",
                "$not",
                "$exists",
                "$type",
                "$abs",
                "$floor",
                "$round",
                "$append",
                "$spread",
                "$each",
                "$merge",
                "$lookup",
                "$substring",
                "$split",
                "$join",
                "$contains",
                "$replace",
                "$formatNumber",
                "$formatBase",
                "$fromMillis",
                "$toMillis",
                "$zip",
                "$single",
                "$shuffle",
                "$echo",
                "$checked",
                "$fail",
            ][..],
        );
        prop_oneof![
            (inner.clone(), ops, inner.clone()).prop_map(|(l, op, r)| format!("({l} {op} {r})")),
            (inner.clone(), inner.clone()).prop_map(|(p, q)| format!("{p}[{q}]")),
            (inner.clone(), inner.clone()).prop_map(|(p, q)| format!("{p}.{q}")),
            (functions, prop::collection::vec(inner.clone(), 0..3))
                .prop_map(|(f, args)| format!("{f}({})", args.join(", "))),
            (inner.clone(), inner.clone(), inner.clone())
                .prop_map(|(c, t, e)| format!("({c} ? {t} : {e})")),
            prop::collection::vec(inner.clone(), 0..3).prop_map(|v| format!("[{}]", v.join(", "))),
            (inner.clone(), inner.clone()).prop_map(|(k, v)| format!("{{{k}: {v}}}")),
            (inner.clone(), inner.clone()).prop_map(|(v, b)| format!("($y := {v}; {b})")),
            (inner.clone(), inner.clone())
                .prop_map(|(d, b)| format!("($f := function($y){{ {b} }}; $f({d}))")),
            (inner.clone(), inner.clone()).prop_map(|(s, k)| format!("{s}^({k})")),
            (inner.clone(), inner.clone(), inner.clone())
                .prop_map(|(s, k, v)| format!("{s}{{{k}: {v}}}")),
            (inner.clone(), inner.clone())
                .prop_map(|(v, f)| format!("{v} ~> $map(function($e){{ {f} }})")),
            inner.clone().prop_map(|v| format!("items#$i[$i < 2].{v}")),
            inner.clone().prop_map(|v| format!("items@$o.{v}")),
            (inner.clone(), inner).prop_map(|(v, u)| format!("{v} ~> |items|{{'p': {u}}}|")),
        ]
    })
}

fn expression() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => valid(),
        3 => prop::collection::vec(prop::sample::select(TOKENS), 0..24).prop_map(|t| t.join("")),
        1 => prop::collection::vec(prop::sample::select(TOKENS), 0..24).prop_map(|t| t.join(" ")),
        1 => any::<String>(),
    ]
}

/// Host functions: one that returns its arguments, one with a signature, one that fails.
fn bindings() -> Bindings {
    Bindings::new()
        .var("x", json!([1, "two", null]))
        .function(
            "echo",
            HostFunction::new(2, |args, call| {
                Ok(Some(json!({"args": args, "input": call.input()})))
            }),
        )
        .function(
            "checked",
            HostFunction::with_signature("<n-a<s>?f?:a>", |args, _| Ok(Some(json!(args))))
                .expect("valid signature"),
        )
        .function(
            "fail",
            HostFunction::new(0, |_, _| Err(Error::new("X0001").with("message", "fails"))),
        )
}

fn input() -> serde_json::Value {
    json!({
        "a": {"b": [1, 2, {"b": "x"}]},
        "items": [{"price": 2, "a": "p"}, {"price": 3.5, "a": null}, {"price": "4"}],
        "s": "hello 𝄞"
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn parsing_never_panics(source in expression()) {
        let ast = jlens_core::parse_recover(&source);
        // Every diagnostic lies within the source.
        for d in &ast.diagnostics {
            prop_assert!(d.span.start <= d.span.end && d.span.end <= source.len(), "{d:?}");
        }
        let _ = jlens_core::parse(&source);
    }

    #[test]
    fn evaluation_never_panics(source in expression()) {
        if let Ok(expr) = Expression::compile(&source) {
            let options = Options::new()
                .timeout(Duration::from_millis(50))
                .max_depth(300);
            let bindings = bindings();
            let input = input();
            let _ = expr.evaluate_with(Some(&input), &bindings, &options);
            let _ = expr.evaluate_with(None, &bindings, &options);
            let _ = expr.evaluate_document(&Document::new(&input), &bindings, &options);
            let _ = expr.evaluate_audited(
                Some(&input),
                &bindings,
                &options,
                &jlens_core::audit::AuditOptions::full(),
            );
        }
    }

    #[test]
    fn cancelled_token_stops_every_evaluation(source in expression()) {
        if let Ok(expr) = Expression::compile(&source) {
            let token = CancelToken::new();
            token.cancel();
            let options = Options::new().cancel_token(token);
            let bindings = bindings();
            let input = input();
            let code = |r: Result<Option<serde_json::Value>, Error>| {
                r.err().map(|e| e.code().to_string())
            };
            let expected = Some("U1003".to_string());
            prop_assert_eq!(code(expr.evaluate_with(Some(&input), &bindings, &options)), expected.clone());
            prop_assert_eq!(
                code(expr.evaluate_document(&Document::new(&input), &bindings, &options)),
                expected.clone()
            );
            let (result, _) = expr.evaluate_audited(
                None,
                &bindings,
                &options,
                &jlens_core::audit::AuditOptions::full(),
            );
            prop_assert_eq!(code(result), expected);
        }
    }

    #[test]
    fn fixed_clock_and_seed_are_reproducible(now in any::<i64>(), seed in any::<u64>()) {
        let expr = Expression::compile(
            "[$now(), $millis(), $now('[FNn] [D1o] [MNn] [Y]', '-0500'), \
              $toMillis('12:00', '[H]:[m]'), $random(), $shuffle([1..20])]",
        )
        .unwrap();
        let options = Options::new().now(now).seed(seed);
        let run = || format!("{:?}", expr.evaluate_with(None, &Bindings::new(), &options));
        prop_assert_eq!(run(), run());
    }

    #[test]
    fn size_caps_fail_with_u1001_or_change_nothing(source in expression()) {
        if let Ok(expr) = Expression::compile(&source) {
            let options = Options::new()
                .timeout(Duration::from_millis(50))
                .max_depth(300)
                .now(0)
                .seed(1);
            let capped = options
                .clone()
                .max_string_length(8)
                .max_array_length(4)
                .max_result_size(16);
            let bindings = bindings();
            let input = input();
            let free = expr.evaluate_with(Some(&input), &bindings, &options);
            let tight = expr.evaluate_with(Some(&input), &bindings, &capped);
            let timed_out = |r: &Result<Option<serde_json::Value>, Error>| {
                r.as_ref().is_err_and(|e| e.code() == "D1012")
            };
            if !timed_out(&free) && !timed_out(&tight) {
                match &tight {
                    Err(e) if e.code() == "U1001" => {}
                    _ => prop_assert_eq!(format!("{tight:?}"), format!("{free:?}")),
                }
            }
        }
    }
}
