//! Property tests: no input, however malformed, makes the parser or the evaluator panic.

use std::time::Duration;

use jlens_core::input::{Sink, Source};
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
                "$formatInteger",
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

/// JSON read through the [`Source`] trait, one level at a time. An object with a field named
/// `fails` cannot be read: its level fails.
enum JsonSource<'j> {
    /// The input itself, the only value of its level.
    Root(&'j serde_json::Value),
    Level(&'j serde_json::Value),
}

fn write<'j>(sink: &mut Sink<'_, 'j>, json: &'j serde_json::Value) {
    match json {
        serde_json::Value::Null => sink.null(),
        serde_json::Value::Bool(b) => sink.bool(*b),
        serde_json::Value::Number(n) => sink.number(n.as_f64().unwrap_or(f64::NAN)),
        serde_json::Value::String(s) => sink.string(s),
        serde_json::Value::Array(_) => sink.array(JsonSource::Level(json)),
        serde_json::Value::Object(_) => sink.object(JsonSource::Level(json)),
    }
}

impl<'j> Source<'j> for JsonSource<'j> {
    fn read(&self, sink: &mut Sink<'_, 'j>) -> Result<(), String> {
        match self {
            JsonSource::Root(json) => write(sink, json),
            JsonSource::Level(serde_json::Value::Array(items)) => {
                items.iter().for_each(|item| write(sink, item))
            }
            JsonSource::Level(serde_json::Value::Object(fields)) => {
                if fields.contains_key("fails") {
                    // Half a level, then a failure: the level is left empty.
                    sink.key("half");
                    sink.null();
                    return Err("bad field".into());
                }
                for (key, value) in fields {
                    sink.key(key);
                    write(sink, value);
                }
            }
            JsonSource::Level(_) => {}
        }
        Ok(())
    }
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

    /// `evaluate` reads its input and bindings lazily, `evaluate_document` converts them first
    /// (ADR 0010): the results are the same.
    #[test]
    fn lazy_input_agrees_with_a_document(source in expression()) {
        if let Ok(expr) = Expression::compile(&source) {
            let options = Options::new()
                .timeout(Duration::from_millis(50))
                .max_depth(300)
                .now(0)
                .seed(1);
            let mut input = input();
            input["n"] = json!([2.0, -0.0, 1e300, 18446744073709551615u64, {"o": [[]], "e": {}}]);
            let bindings = bindings().var("o", input.clone());
            let lazy = expr.evaluate_with(Some(&input), &bindings, &options);
            let document = expr.evaluate_document(&Document::new(&input), &bindings, &options);
            let source = expr.evaluate_source(&JsonSource::Root(&input), &bindings, &options);
            let timed_out = |r: &Result<Option<serde_json::Value>, Error>| {
                r.as_ref().is_err_and(|e| e.code() == "D1012")
            };
            if !timed_out(&lazy) && !timed_out(&document) {
                prop_assert_eq!(format!("{lazy:?}"), format!("{document:?}"));
            }
            if !timed_out(&source) && !timed_out(&document) {
                prop_assert_eq!(format!("{source:?}"), format!("{document:?}"));
            }
        }
    }
}

#[test]
fn unread_input_is_returned_as_converted() {
    let input = json!({"n": [2.0, -0.0, 1e300, 18446744073709551615u64], "o": {"a": [1.5]}});
    let document = Document::new(&input);
    for source in ["n", "o", "$", "$$.o", "[o]", "{'x': n}"] {
        let expr = Expression::compile(source).unwrap();
        let options = Options::default();
        assert_eq!(
            expr.evaluate_with(Some(&input), &Bindings::new(), &options)
                .unwrap(),
            expr.evaluate_document(&document, &Bindings::new(), &options)
                .unwrap(),
            "{source}"
        );
    }
    // A result larger than the size limit fails before it is copied.
    let options = Options::new().max_result_size(10);
    let expr = Expression::compile("o").unwrap();
    let error = expr.evaluate_with(Some(&input), &Bindings::new(), &options);
    assert_eq!(error.unwrap_err().code(), "U1001");
}

#[test]
fn a_level_that_cannot_be_read_fails_the_evaluation() {
    let input = json!({
        "ok": {"a": 1},
        "bad": {"fails": true, "a": 2},
        "list": [{"bad": {"fails": true}}]
    });
    let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = called.clone();
    let bindings = Bindings::new().function(
        "f",
        HostFunction::new(1, move |args, _| {
            seen.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(args[0].clone())
        }),
    );
    let evaluate = |source: &str| {
        let expr = Expression::compile(source).unwrap();
        expr.evaluate_source(&JsonSource::Root(&input), &bindings, &Options::default())
    };
    // The levels an expression does not reach are never read.
    assert_eq!(evaluate("ok.a").unwrap(), Some(json!(1)));
    assert_eq!(evaluate("$exists(bad)").unwrap(), Some(json!(true)));
    // Reading the bad level fails with U1004, carrying the source's message, whatever the
    // expression does with the empty level.
    for source in [
        "bad.a",
        "bad",
        "$count($keys(bad))",
        "list.bad.x",
        "$string($)",
        "**.a",
    ] {
        let error = evaluate(source).unwrap_err();
        assert_eq!(error.code(), "U1004", "{source}");
        assert!(error.message().contains("bad field"), "{source}");
    }
    // A host function is not called with a level that could not be read.
    assert_eq!(evaluate("$f(bad)").unwrap_err().code(), "U1004");
    assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
    // Nothing written: the input is undefined.
    let expr = Expression::compile("$type($)").unwrap();
    let nothing = JsonSource::Level(&serde_json::Value::Null);
    let result = expr.evaluate_source(&nothing, &Bindings::new(), &Options::default());
    assert_eq!(result.unwrap(), None);
}

/// Patterns that backtrack exponentially on a run of a's that ends in a character they cannot
/// match, as in ADR 0009.
fn catastrophic_regex() -> impl Strategy<Value = String> {
    let body = prop::sample::select(&["a+", "a|a", "aa|a", "a|aa", "a?a", "(a+)+", "a+a+"][..]);
    let outer = prop::sample::select(&["+", "*"][..]);
    let flags = prop::sample::select(&["", "i", "m"][..]);
    let call = prop::sample::select(
        &[
            "$contains($s, R)",
            "$match($s, R)",
            "$replace($s, R, 'x')",
            "$split($s, R)",
            "R($s)",
        ][..],
    );
    (body, outer, flags, call, 30usize..45).prop_map(|(body, outer, flags, call, n)| {
        let regex = format!("/^({body}){outer}$/{flags}");
        format!(
            "($s := '{}!'; {})",
            "a".repeat(n),
            call.replace('R', &regex)
        )
    })
}

#[test]
fn catastrophic_regexes_stop_within_the_timeout() {
    // Each case runs until its timeout, so the number of cases is fixed. The `proptest!` macro
    // would let `PROPTEST_CASES` raise it, and 100,000 cases would wait more than half an hour
    // to repeat the few thousand patterns the strategy can generate.
    let config = ProptestConfig {
        cases: 32,
        failure_persistence: None,
        ..ProptestConfig::default()
    };
    let result =
        proptest::test_runner::TestRunner::new(config).run(&catastrophic_regex(), |source| {
            let expr = Expression::compile(&source).unwrap();
            let options = Options::new().timeout(Duration::from_millis(20));
            let started = std::time::Instant::now();
            let result = expr.evaluate_with(None, &Bindings::new(), &options);
            let code = result.err().map(|e| e.code().to_string());
            prop_assert_eq!(code.as_deref(), Some("D1012"), "{}", source);
            prop_assert!(
                started.elapsed() < Duration::from_secs(2),
                "{:?}",
                started.elapsed()
            );
            Ok(())
        });
    if let Err(e) = result {
        panic!("{e}");
    }
}

/// Inputs that made the fuzz targets (`fuzz/`) panic, with what they give now: a result, or an
/// error code.
/// Inputs on which the fuzz targets that parse overflowed their 1 MB stack.
#[test]
fn fuzz_parse_crashes_stay_fixed() {
    let sources = [
        // Recovery wrapped every unclosed level in an error node that cloned the levels inside.
        "(".repeat(600),
        "[".repeat(600),
        format!("{}1;", "$f(".repeat(600)),
        // Chains of operators built trees deeper than the nesting limit.
        format!("1{}", "+1".repeat(10_000)),
    ];
    // A lambda whose body is a long chain of conditions, processed with little stack left:
    // marking its tail calls recursed without growing the stack.
    let valid = [format!(
        "{}function(){{ {}1 }}{}",
        "(".repeat(175),
        "1 ? 1 : ".repeat(405),
        ")".repeat(175)
    )];
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            for source in &sources {
                let ast = jlens_core::parse_recover(source);
                assert!(ast.has_errors(), "{}", &source[..20]);
                let _ = jlens_core::parse(source);
            }
            for source in &valid {
                assert!(jlens_core::parse(source).is_ok(), "{}", &source[..20]);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

/// A number that is not finite, written as the input by a source that does not follow JSON.
struct Infinite(f64);

impl<'a> Source<'a> for Infinite {
    fn read(&self, sink: &mut Sink<'_, 'a>) -> Result<(), String> {
        sink.number(self.0);
        Ok(())
    }
}

#[test]
fn infinite_input_ends() {
    let cases: &[(f64, &str, Result<serde_json::Value, &str>)] = &[
        // Letters and words for infinity looped forever.
        (f64::INFINITY, "$formatInteger($, 'w')", Ok(json!(""))),
        (f64::INFINITY, "$formatInteger($, 'a')", Ok(json!(""))),
        // An exponent picture scaled an infinite mantissa forever. The text is what the other
        // pictures give for infinity.
        (
            f64::INFINITY,
            "$formatNumber($, '0e0')",
            Ok(json!("Infinitye0")),
        ),
        (
            f64::NEG_INFINITY,
            "$formatNumber($, '0.0e0')",
            Ok(json!("-NaN.0e0")),
        ),
        (f64::INFINITY, "$ + 1", Err("D1001")),
    ];
    for (input, source, expected) in cases {
        let result = Expression::compile(source).unwrap().evaluate_source(
            &Infinite(*input),
            &Bindings::new(),
            &Options::new(),
        );
        let got = match &result {
            Ok(v) => Ok(v.clone().unwrap_or(serde_json::Value::Null)),
            Err(e) => Err(e.code()),
        };
        assert_eq!(&got, expected, "{source}");
    }
}

#[test]
fn nested_fallbacks_stay_linear() {
    // `?:` and `??` used to copy their left side into a condition, doubling the tree at each
    // level: 22 levels made 29 million nodes.
    for op in ["?:", "??"] {
        let source = format!("{}a{}", "(".repeat(200), format!(" {op} 1)").repeat(200));
        let compiled = Expression::compile(&source).unwrap();
        let mut nodes = 0;
        compiled.ast().root.walk(&mut |_| nodes += 1);
        assert!(nodes < 1_000, "{op}: {nodes} nodes");
        let result = compiled.evaluate(Some(&json!({"a": 2})), &Bindings::new());
        assert_eq!(result.unwrap(), Some(json!(2)));
    }
}

#[test]
fn fuzz_crashes_stay_fixed() {
    let cases: &[(&str, Result<serde_json::Value, &str>)] = &[
        // A picture marker that starts with a comma: an unknown component.
        ("$fromMillis(0, '[,2]')", Err("D3132")),
        ("$fromMillis(0, '[,DWwo]')", Err("D3132")),
        ("$fromMillis(0, '[,,]')", Err("D3132")),
        ("$toMillis('1', '[,]')", Err("D3133")),
        // Characters below the picture's digit family end the number.
        ("$parseInteger('１２ [1]３４０', 'map###０')", Ok(json!(12))),
        ("$parseInteger('－１２x', '０')", Ok(json!(-12))),
        ("$parseInteger('12', '##０')", Ok(json!(null))),
        // A name presentation for fractional seconds.
        (
            "$fromMillis(1533038400000, 'Week: [w]xNo[ f n]')",
            Err("D3133"),
        ),
        // Rounding a huge number to many decimals overflowed.
        (
            "$formatNumber(1e300, '##4.11111111111111111101')",
            Ok(json!("1e+300.00000000000000000000")),
        ),
        ("$round(1e300, 20)", Ok(json!(1e300))),
        // Width modifiers beyond the string limit.
        ("$fromMillis(0, '[Y,5555555555555555555553]')", Err("U1001")),
        (
            "$fromMillis(0, '[D0,66666666666666666666666666661]')",
            Err("U1001"),
        ),
        ("$fromMillis(0, '[Y01,99999999999999999999]')", Err("U1001")),
        ("$fromMillis(0, '[Y,3-4]')", Ok(json!("1970"))),
        // A year with dozens of digits overflowed the calendar arithmetic.
        (
            "$toMillis('444444444444444444440008-10-21', '[Y0000]-[M01]-[D01]')",
            Ok(json!(null)),
        ),
        (
            "$toMillis('275760-09-13', '[Y0001]-[M01]-[D01]')",
            Ok(json!(8_640_000_000_000_000_u64)),
        ),
        (
            "$toMillis('275760-09-14', '[Y0001]-[M01]-[D01]')",
            Ok(json!(null)),
        ),
        // A long width with a separator took quadratic time.
        (
            "$length($fromMillis(0, '[DU0,1020449:]'))",
            Ok(json!(2040897)),
        ),
        // Parsing does not pad: a huge width only fails to match, as in jsonata-js.
        (
            "$toMillis('197001', '[Y,99999999999999999999][M01]')",
            Ok(json!(null)),
        ),
        // Words for a NaN field (a date beyond JavaScript's range) recursed forever.
        ("$fromMillis(1e17, '[dw]|[dI]|[d]')", Ok(json!("||NaN"))),
        ("$fromMillis(0, '[dwo]', '10010200101041')", Ok(json!(""))),
        // A number that overflows fails where it overflows, so no expression gives infinity to
        // a function; `infinite_input_ends` passes it from a source.
        ("$formatInteger(1e308 * 10, 'w')", Err("D1001")),
        ("$formatNumber(-1 / 0, '0e0')", Err("D1001")),
        ("$sum([1e308, 1e308])", Err("D1001")),
        ("$average([1e308, 1e308])", Ok(json!(1e308))),
        ("$round(1.7e308, -308)", Err("D1001")),
    ];
    for (source, expected) in cases {
        let result = Expression::compile(source).unwrap().evaluate_with(
            None,
            &Bindings::new(),
            &Options::new(),
        );
        let got = match &result {
            Ok(v) => Ok(v.clone().unwrap_or(serde_json::Value::Null)),
            Err(e) => Err(e.code()),
        };
        assert_eq!(&got, expected, "{source}");
    }
}
