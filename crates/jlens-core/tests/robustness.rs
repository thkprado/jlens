//! Property tests: no input, however malformed, makes the parser or the evaluator panic.

use std::collections::BTreeMap;
use std::time::Duration;

use jlens_core::input::{Sink, Source};
use jlens_core::output::Builder;
use jlens_core::reads::{Read, Reads};
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
    "$merge(", "$sort(", "$reduce(", "$each(", "$eval(", "$echo(", "$checked(", "$fail(", "$echo(?, ", "$apply(", "/a+/", "/[/", "`a b`", "/* c */",
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
            "'p'",
            "true",
            "null",
            "[]",
            "{}",
            "items.price",
            "a.b",
            "o.p.r",
            "o.p.r.z",
            "$.o.p.q",
            "$$.o.p",
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
                "$apply",
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
            (inner.clone(), inner.clone())
                .prop_map(|(d, b)| format!("$apply(function($y){{ {b} }}, {d})")),
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

/// Host functions: one that returns its arguments, one with a signature, one that fails, one
/// that calls its first argument with the others.
fn bindings() -> Bindings {
    functions().var("x", variable_x())
}

fn variable_x() -> serde_json::Value {
    json!([1, "two", {"p": [{"q": 1, "r": 2}], "s": {}}, null])
}

/// The host functions of [`bindings`], without its variable.
fn functions() -> Bindings {
    Bindings::new()
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
        .function(
            "apply",
            HostFunction::new(2, |args, call| match call.function(0) {
                Some(f) => f.call(args.get(1..).unwrap_or_default()),
                None => Ok(None),
            }),
        )
}

/// JSON read through the [`Source`] trait, one level at a time, and one field at a time when
/// `fields` is set. An object with a field named `fails` cannot be read, whole or one field.
#[derive(Clone, Copy)]
struct JsonSource<'j> {
    json: &'j serde_json::Value,
    /// Whether this is the input itself, the only value of its level.
    root: bool,
    fields: bool,
}

thread_local! {
    /// The levels read whole, and the fields read alone, by the sources of this thread.
    static READS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

impl<'j> JsonSource<'j> {
    fn root(json: &'j serde_json::Value, fields: bool) -> Self {
        JsonSource {
            json,
            root: true,
            fields,
        }
    }

    fn write(&self, sink: &mut Sink<'_, 'j>, json: &'j serde_json::Value) {
        let level = JsonSource {
            json,
            root: false,
            fields: self.fields,
        };
        match json {
            serde_json::Value::Null => sink.null(),
            serde_json::Value::Bool(b) => sink.bool(*b),
            serde_json::Value::Number(n) => sink.number(n.as_f64().unwrap_or(f64::NAN)),
            serde_json::Value::String(s) => sink.string(s),
            serde_json::Value::Array(_) => sink.array(level),
            serde_json::Value::Object(_) => sink.object(level),
        }
    }
}

impl<'j> Source<'j> for JsonSource<'j> {
    fn read(&self, sink: &mut Sink<'_, 'j>) -> Result<(), String> {
        READS.with(|reads| reads.set((reads.get().0 + 1, reads.get().1)));
        if self.root {
            self.write(sink, self.json);
            return Ok(());
        }
        match self.json {
            serde_json::Value::Array(items) => items.iter().for_each(|item| self.write(sink, item)),
            serde_json::Value::Object(fields) => {
                if fields.contains_key("fails") {
                    // Half a level, then a failure: the level is left empty.
                    sink.key("half");
                    sink.null();
                    return Err("bad field".into());
                }
                for (key, value) in fields {
                    sink.key(key);
                    self.write(sink, value);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn read_field(&self, key: &str, sink: &mut Sink<'_, 'j>) -> Result<bool, String> {
        let serde_json::Value::Object(fields) = self.json else {
            return Ok(false);
        };
        if !self.fields || self.root {
            return Ok(false);
        }
        READS.with(|reads| reads.set((reads.get().0, reads.get().1 + 1)));
        if fields.contains_key("fails") {
            return Err("bad field".into());
        }
        if let Some(value) = fields.get(key) {
            self.write(sink, value);
        }
        Ok(true)
    }
}

fn input() -> serde_json::Value {
    json!({
        "a": {"b": [1, 2, {"b": "x"}]},
        "items": [{"price": 2, "a": "p"}, {"price": 3.5, "a": null}, {"price": "4"}],
        "s": "hello 𝄞",
        "o": {"p": {"q": [1], "r": "deep"}}
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
            let source = expr.evaluate_source(&JsonSource::root(&input, false), &bindings, &options);
            let fields = expr.evaluate_source(&JsonSource::root(&input, true), &bindings, &options);
            let timed_out = |r: &Result<Option<serde_json::Value>, Error>| {
                r.as_ref().is_err_and(|e| e.code() == "D1012")
            };
            if !timed_out(&lazy) && !timed_out(&document) {
                prop_assert_eq!(format!("{lazy:?}"), format!("{document:?}"));
            }
            if !timed_out(&source) && !timed_out(&document) {
                prop_assert_eq!(format!("{source:?}"), format!("{document:?}"));
            }
            if !timed_out(&fields) && !timed_out(&document) {
                prop_assert_eq!(format!("{fields:?}"), format!("{document:?}"));
            }
        }
    }

    /// A result written with a [`Builder`] (ADR 0017) is the one `evaluate` returns, and fails
    /// the same way, within a size limit or not.
    #[test]
    fn a_builder_writes_what_evaluate_returns(source in expression()) {
        if let Ok(expr) = Expression::compile(&source) {
            let options = Options::new()
                .timeout(Duration::from_millis(50))
                .max_depth(300)
                .now(0)
                .seed(1);
            let input = input();
            let bindings = bindings();
            let document = Document::new(&input);
            let timed_out = |r: &Result<Option<serde_json::Value>, Error>| {
                r.as_ref().is_err_and(|e| e.code() == "D1012")
            };
            for options in [options.clone(), options.max_result_size(16)] {
                let json = expr.evaluate_document(&document, &bindings, &options);
                let built = expr.evaluate_document_to(&document, &bindings, &options, |out| {
                    out.write(&mut Rebuilt)
                });
                if !timed_out(&json) && !timed_out(&built) {
                    prop_assert_eq!(format!("{built:?}"), format!("{json:?}"));
                }
                let source = JsonSource::root(&input, true);
                let json = expr.evaluate_source(&source, &bindings, &options);
                let built = expr.evaluate_source_to(&source, &bindings, &options, |out| {
                    out.write(&mut Rebuilt)
                });
                if !timed_out(&json) && !timed_out(&built) {
                    prop_assert_eq!(format!("{built:?}"), format!("{json:?}"));
                }
            }
        }
    }

    /// Evaluation takes fast paths only without a tracer; an audit takes the general path
    /// everywhere: the results are the same.
    #[test]
    fn fast_paths_agree_with_an_audit(source in expression()) {
        if let Ok(expr) = Expression::compile(&source) {
            let options = Options::new()
                .timeout(Duration::from_millis(50))
                .max_depth(300)
                .now(0)
                .seed(1);
            let bindings = bindings();
            let input = input();
            let plain = expr.evaluate_with(Some(&input), &bindings, &options);
            let (audited, _) = expr.evaluate_audited(
                Some(&input),
                &bindings,
                &options,
                &jlens_core::audit::AuditOptions::full(),
            );
            let timed_out = |r: &Result<Option<serde_json::Value>, Error>| {
                r.as_ref().is_err_and(|e| e.code() == "D1012")
            };
            if !timed_out(&plain) && !timed_out(&audited) {
                prop_assert_eq!(format!("{plain:?}"), format!("{audited:?}"));
            }
        }
    }
}

/// Paths of field names take a fast path that falls back on the general one where an array is
/// met; an audit takes the general path. The results and errors are the same, at any depth.
#[test]
fn field_paths_agree_with_an_audit() {
    let input = json!({
        "o": {"p": {"q": [1, [2]], "r": "deep", "s": {"t": null}}},
        "l": [{"m": {"n": 1}}, {"m": [2, 3]}]
    });
    let bindings = Bindings::new()
        .var("v", input.clone())
        .var("w", json!([{"a": 1}]));
    let sources = [
        "o.p.r",
        "o.p.q",
        "o.p.s.t",
        "o.x.y",
        "o.p.r.z",
        "$.o.p",
        "$$.o.p.q",
        "$v.o.p.r",
        "$w.a",
        "$u.a",
        "l.m.n",
        "l.m",
        "$string.a",
        "o.p.q[]",
        "o.p.r[]",
        "$.o.p.q[0]",
    ];
    for source in sources {
        let expr = Expression::compile(source).unwrap();
        for depth in [1, 2, 3, 4, 100] {
            let options = Options::new().max_depth(depth);
            let plain = expr.evaluate_with(Some(&input), &bindings, &options);
            let (audited, _) = expr.evaluate_audited(
                Some(&input),
                &bindings,
                &options,
                &jlens_core::audit::AuditOptions::full(),
            );
            assert_eq!(
                format!("{plain:?}"),
                format!("{audited:?}"),
                "{source} at depth {depth}"
            );
        }
    }
}

/// A predicate of literals alone is evaluated once without a tracer, and once per item in an
/// audit. The results and errors are the same.
#[test]
fn constant_predicates_agree_with_an_audit() {
    let input = json!({
        "items": [{"n": [1, 2]}, {"n": 3}, [4, 5], "six", {"n": [7]}],
        "one": {"n": 1}
    });
    let bindings = Bindings::new().var("x", json!([1, [2, 3], 4]));
    let sources = [
        "items[[0..2]]",
        "items[[2, 0, 0, -1]]",
        "items[[1.5, -0.5, -0, 7, -9]]",
        "items[[]]",
        "items[[1, 'a']]",
        "items[true]",
        "items[false]",
        "items['x']",
        "items['']",
        "items[null]",
        "items[1 + 1]",
        "items[-1]",
        "items[2 > 1]",
        "items[1 = 1 and 2 = 3]",
        "items[1 + 'a']",
        "items[-(1 + 1)]",
        "items[('x'; [0, 1])]",
        "missing[1 + 'a']",
        "one[[0, 0]]",
        "one[[1]]",
        "items.n[[0..1]]",
        "items.n[[1]][]",
        "items[[0, 1]]#$i.{'i': $i}",
        "items#$i[[3, 4]].{'i': $i}",
        "items@$o[[1]].$o",
        "items[[0..4]][[1, 3]][-1]",
        "[1, 2, 3][[0, 2]]",
        "[][1 + 'a']",
        "$x[[0, 1]]",
        "$x[[1..9]]",
        "$string(items[[0]])",
    ];
    for source in sources {
        let expr = Expression::compile(source).unwrap();
        let plain = expr.evaluate_with(Some(&input), &bindings, &Options::new());
        let (audited, _) = expr.evaluate_audited(
            Some(&input),
            &bindings,
            &Options::new(),
            &jlens_core::audit::AuditOptions::full(),
        );
        assert_eq!(format!("{plain:?}"), format!("{audited:?}"), "{source}");
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
    // Levels read whole, then one field at a time: a field that cannot be read fails the same way.
    for fields in [false, true] {
        let evaluate = |source: &str| {
            let expr = Expression::compile(source).unwrap();
            let input = JsonSource::root(&input, fields);
            expr.evaluate_source(&input, &bindings, &Options::default())
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
    }
    // Nothing written: the input is undefined.
    let expr = Expression::compile("$type($)").unwrap();
    let null = serde_json::Value::Null;
    let nothing = JsonSource {
        json: &null,
        root: false,
        fields: false,
    };
    let result = expr.evaluate_source(&nothing, &Bindings::new(), &Options::default());
    assert_eq!(result.unwrap(), None);
}

/// A path step reads one field of an object not read yet, when the source can; the object keeps
/// it, and reads its level once when it is needed whole (ADR 0010, amendment).
#[test]
fn a_path_step_reads_one_field() {
    let input = json!({"o": {"a": 1, "b": {"c": 2}, "d": [3]}});
    let reads = |source: &str, fields: bool| {
        READS.with(|reads| reads.set((0, 0)));
        let expr = Expression::compile(source).unwrap();
        let input = JsonSource::root(&input, fields);
        let result = expr.evaluate_source(&input, &Bindings::new(), &Options::default());
        let (levels, fields) = READS.with(|reads| reads.get());
        (result.unwrap(), levels, fields)
    };
    // The input itself is one read; each step reads one field.
    assert_eq!(reads("o.b.c", true), (Some(json!(2)), 1, 3));
    // A source that reads no fields alone reads each level whole.
    assert_eq!(reads("o.b.c", false), (Some(json!(2)), 4, 0));
    // An array or object read alone is kept, and so is it when its object is read whole later;
    // a scalar is read again: `o` is read alone and whole once, `a` twice alone.
    assert_eq!(
        reads("[o.a, o.a, $count($keys(o)), o.a, o.b.c, $keys(o)]", true),
        (Some(json!([1, 1, 3, 1, 2, "a", "b", "d"])), 2, 4)
    );
    // A missing field is looked up each time.
    assert_eq!(reads("[o.x, o.x, o.a]", true), (Some(json!([1])), 1, 4));
    // An object some fields of which were read is returned whole, in its order: `o`, `b` and
    // `d` are read whole.
    assert_eq!(
        reads("o.b.c and o.a ? o : null", true),
        (Some(input["o"].clone()), 4, 4)
    );
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
            Ok(json!(format!(
                "{}.{}",
                format!("{:.0}", 1e300),
                "0".repeat(20)
            ))),
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
        // A date beyond JavaScript's range (±8.64e15 ms after the offset) fails. Its fields used
        // to be NaN in part, and words for them recursed forever.
        ("$fromMillis(1e17, '[dw]|[dI]|[d]')", Err("D1001")),
        ("$fromMillis(1e17, '[Y0001]')", Err("D1001")),
        ("$fromMillis(-1e300)", Err("D1001")),
        ("$fromMillis(0, '[dwo]', '10010200101041')", Err("D1001")),
        ("$fromMillis(8.64e15 + 1)", Err("D1001")),
        ("$fromMillis(8.64e15, '[Y]', '+0100')", Err("D1001")),
        ("$fromMillis(8.64e15, '[Y]', '-0100')", Ok(json!("275760"))),
        // Within the range every component is exact, also where the day of the year or the week
        // counts from a date beyond it (jsonata-js: "-271821 NaN NaN NaN").
        (
            "$fromMillis(-8.64e15, '[Y] [d] [W] [X] [w] [x]')",
            Ok(json!("-271821 110 16 -271821 4 4")),
        ),
        (
            "$fromMillis(8.64e15, '[Y] [d] [W] [X]')",
            Ok(json!("275760 257 37 275760")),
        ),
        // A timezone that is not a number fails (jsonata-js writes every component as "NaN"); one
        // that starts with a number is read as far as its digits go, as JavaScript's parseInt.
        ("$fromMillis(0, undefined, 'two')", Err("D3150")),
        ("$fromMillis(0, '[Y] [H]:[m] [Z]', '')", Err("D3150")),
        ("$fromMillis(0, '[Y] [H]:[m] [Z]', '+')", Err("D3150")),
        ("$now('[Y]', 'abc')", Err("D3150")),
        (
            "$fromMillis(0, '[H]:[m] [Z]', ' +0530x')",
            Ok(json!("5:30 +05:30")),
        ),
        // A date that the offset moves beyond the range is not a date (jsonata-js returns it).
        (
            "$toMillis('275760-09-13T00:00-0100', '[Y]-[M]-[D]T[H]:[m][Z0000]')",
            Ok(json!(null)),
        ),
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

/// Builds `serde_json` values, as `evaluate` returns them, counting the values it builds.
struct Rebuilt;

thread_local! {
    static BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Rebuilt {
    fn built<T>(value: T) -> T {
        BUILT.with(|built| built.set(built.get() + 1));
        value
    }
}

impl Builder for Rebuilt {
    type Value = serde_json::Value;
    type Array = Vec<serde_json::Value>;
    type Object = serde_json::Map<String, serde_json::Value>;

    fn null(&mut self) -> serde_json::Value {
        Rebuilt::built(serde_json::Value::Null)
    }
    fn bool(&mut self, value: bool) -> serde_json::Value {
        Rebuilt::built(value.into())
    }
    fn number(&mut self, value: f64) -> serde_json::Value {
        assert!(value.is_finite());
        Rebuilt::built(if value.fract() == 0.0 && value.abs() < 2f64.powi(53) {
            (value as i64).into()
        } else {
            value.into()
        })
    }
    fn string(&mut self, value: &str) -> serde_json::Value {
        Rebuilt::built(value.into())
    }
    fn array(&mut self, len: usize) -> Vec<serde_json::Value> {
        Vec::with_capacity(len)
    }
    fn push(&mut self, array: &mut Vec<serde_json::Value>, item: serde_json::Value) {
        array.push(item);
    }
    fn end_array(&mut self, array: Vec<serde_json::Value>) -> serde_json::Value {
        Rebuilt::built(array.into())
    }
    fn object(&mut self, len: usize) -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::with_capacity(len)
    }
    fn insert(
        &mut self,
        object: &mut serde_json::Map<String, serde_json::Value>,
        key: &str,
        value: serde_json::Value,
    ) {
        assert!(object.insert(key.into(), value).is_none(), "{key} twice");
    }
    fn end_object(
        &mut self,
        object: serde_json::Map<String, serde_json::Value>,
    ) -> serde_json::Value {
        Rebuilt::built(object.into())
    }
}

/// A builder stops at the size and nesting limits of a result, writing reads what the evaluation
/// left unread, and the closure that writes is called only on success (ADR 0017).
#[test]
fn a_builder_stops_at_the_limits() {
    let write = |source: &str, options: &Options| {
        BUILT.with(|built| built.set(0));
        let expr = Expression::compile(source).unwrap();
        let document = Document::new(&json!({"a": [1, 2, 3]}));
        let result = expr.evaluate_document_to(&document, &Bindings::new(), options, |out| {
            out.write(&mut Rebuilt)
        });
        (result, BUILT.with(|built| built.get()))
    };
    let small = Options::new().max_result_size(10);
    assert_eq!(write("a", &small), (Ok(Some(json!([1, 2, 3]))), 4));
    // Writing stops at the limit: a few values of the thousand.
    let (result, built) = write("[1..1000]", &small);
    assert_eq!(result.unwrap_err().code(), "U1001");
    assert!(built < 10, "{built}");
    let build = "$build := function($n, $acc) { $n = 0 ? $acc : $build($n - 1, {'a': $acc}) }";
    let (result, built) = write(&format!("({build}; $build(1000, 1))"), &Options::new());
    assert_eq!((result.is_ok(), built), (true, 1001));
    let (result, _) = write(&format!("({build}; $build(1001, 1))"), &Options::new());
    assert_eq!(result.unwrap_err().code(), "U1001");
    // Undefined.
    assert_eq!(write("b", &Options::new()), (Ok(None), 0));

    // Deep input read from a source.
    let expr = Expression::compile("$").unwrap();
    let deep = Nested(100_000);
    let result = expr.evaluate_source_to(&deep, &Bindings::new(), &Options::new(), |out| {
        out.write(&mut Rebuilt)
    });
    assert_eq!(result.unwrap_err().code(), "U1001");

    // A level read only when the result is written, and that fails.
    let input = json!({"bad": {"fails": true}, "ok": 1});
    let mut called = false;
    let expr = Expression::compile("bad").unwrap();
    let source = JsonSource::root(&input, false);
    let result = expr.evaluate_source_to(&source, &Bindings::new(), &Options::new(), |out| {
        called = true;
        out.write(&mut Rebuilt)
    });
    assert_eq!(result.unwrap_err().code(), "U1004");
    assert!(called);
    // An evaluation that fails does not call the closure.
    let mut called = false;
    let expr = Expression::compile("$error('no')").unwrap();
    let result = expr.evaluate_source_to(&source, &Bindings::new(), &Options::new(), |out| {
        called = true;
        out.write(&mut Rebuilt)
    });
    assert_eq!(result.unwrap_err().code(), "D3137");
    assert!(!called);
    // What the closure returns is returned.
    let expr = Expression::compile("ok").unwrap();
    let result = expr.evaluate_source_to(&source, &Bindings::new(), &Options::new(), |_| Ok(7));
    assert_eq!(result.unwrap(), 7);
}

/// Arrays nested `.0` levels deep around a number, read one level at a time.
struct Nested(usize);

impl<'a> Source<'a> for Nested {
    fn read(&self, sink: &mut Sink<'_, 'a>) -> Result<(), String> {
        match self.0 {
            0 => sink.number(1.0),
            n => sink.array(Nested(n - 1)),
        }
        Ok(())
    }
}

/// Values nested 100,000 levels deep (5,000 where that is enough), read from a source or built
/// by a tail-recursive function, give a result or fail with `U1001` beyond the nesting limit:
/// nothing overflows the stack, dropping them included.
#[test]
fn deep_values_fail_cleanly() {
    let run = || {
        let bindings = Bindings::new()
            .function("arg", HostFunction::new(1, |args, _| Ok(args[0].clone())))
            .function("input", HostFunction::new(0, |_, call| Ok(call.input())));
        let from_source = [
            ("$", Err("U1001")),
            ("$[0]", Err("U1001")),
            ("$string($)", Err("U1001")),
            ("$clone($)", Err("U1001")),
            ("[$, $] ~> $distinct", Err("U1001")),
            ("$arg($)", Err("U1001")),
            ("$.$input()", Err("U1001")),
            ("$count(**)", Err("U1001")),
            ("$count($)", Ok(json!(1))),
            ("$ = $", Ok(json!(true))),
            ("$boolean($)", Ok(json!(true))),
            ("$count($keys($))", Ok(json!(0))),
            ("$spread($)", Ok(json!([1]))),
            ("*", Ok(json!([1]))),
            // Undefined.
            ("$.a", Ok(json!(null))),
        ];
        let deep = Nested(100_000);
        for (source, expected) in from_source {
            let expr = Expression::compile(source).unwrap();
            let result = expr.evaluate_source(&deep, &bindings, &Options::new());
            let got = result.map(|v| v.unwrap_or_default());
            match expected {
                Ok(value) => assert_eq!(got.unwrap(), value, "{source}"),
                Err(code) => assert_eq!(got.unwrap_err().code(), code, "{source}"),
            }
        }
        let build = "$build := function($n, $acc) { $n = 0 ? $acc : $build($n - 1, {'a': $acc}) }";
        let built = [
            ("$build(100000, 1)", Err("U1001")),
            ("$string($build(5000, 1))", Err("U1001")),
            ("$build(5000, 1) = $build(5000, 1)", Err("U1001")),
            ("$count([$build(100000, 1)])", Ok(json!(1))),
            ("$count($build(5000, 1).**)", Err("U1001")),
            // Up to the limit, the value is whole.
            ("$build(1000, 1) = $build(1000, 1)", Ok(json!(true))),
            ("$build(1001, 1) = $build(1001, 1)", Err("U1001")),
            ("$length($string($build(1000, 1)))", Ok(json!(1000 * 6 + 1))),
        ];
        for (source, expected) in built {
            let source = format!("({build}; {source})");
            let expr = Expression::compile(&source).unwrap();
            let result = expr.evaluate(None, &Bindings::new());
            match expected {
                Ok(value) => assert_eq!(result.unwrap(), Some(value), "{source}"),
                Err(code) => assert_eq!(result.unwrap_err().code(), code, "{source}"),
            }
        }
        // An audit keeps the values it captures within the limit, whatever depth it asks for.
        let expr = Expression::compile(&format!("({build}; $build(1100, 1))")).unwrap();
        let mut audit = jlens_core::audit::AuditOptions::default();
        audit.limits.max_depth = usize::MAX;
        let (result, tree) = expr.evaluate_audited(None, &Bindings::new(), &Options::new(), &audit);
        assert_eq!(result.unwrap_err().code(), "U1001");
        drop(tree);
        // Up to the limit, a source's value is returned whole.
        let expr = Expression::compile("$").unwrap();
        let options = Options::new();
        let result = expr.evaluate_source(&Nested(1_000), &Bindings::new(), &options);
        assert!(result.unwrap().is_some());
        let result = expr.evaluate_source(&Nested(1_001), &Bindings::new(), &options);
        assert_eq!(result.unwrap_err().code(), "U1001");
    };
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(run)
        .unwrap()
        .join()
        .unwrap();
}

/// The outcome of an evaluation as the analysis of reads promises it: the result, or the error
/// code. `None` for a timeout, which depends on the machine.
fn outcome(result: Result<Option<serde_json::Value>, Error>) -> Option<String> {
    match result {
        Ok(value) => Some(format!("{value:?}")),
        Err(e) if e.code() == "D1012" => None,
        Err(e) => Some(e.code().to_owned()),
    }
}

/// `bindings` with every host function marked as not reading its context.
fn without_input(bindings: Bindings) -> Bindings {
    let mut marked = Bindings::new();
    for (name, value) in bindings.iter() {
        marked = marked.var(name, value.clone());
    }
    for (name, function) in bindings.functions() {
        marked = marked.function(name, function.clone().without_input());
    }
    marked
}

/// [`bindings`] cut by `reads`: the host functions, and the variable if it is read.
fn cut_bindings(reads: &Reads) -> Bindings {
    let x = variable_x();
    let x = if reads.every_binding() {
        Some(x)
    } else {
        reads.binding("x").map(|read| read.cut(&x))
    };
    match x {
        Some(x) => functions().var("x", x),
        None => functions(),
    }
}

/// Whether `expr` gives the same outcome on the input and bindings cut by `reads`.
fn agrees_when_cut(expr: &Expression, reads: &Reads) -> Result<(), TestCaseError> {
    agrees_when_cut_with(expr, reads, false)
}

/// [`agrees_when_cut`], with the host functions marked as not reading their context when
/// `marked` is set.
fn agrees_when_cut_with(
    expr: &Expression,
    reads: &Reads,
    marked: bool,
) -> Result<(), TestCaseError> {
    let mark = |bindings: Bindings| {
        if marked {
            without_input(bindings)
        } else {
            bindings
        }
    };
    let options = Options::new()
        .timeout(Duration::from_millis(50))
        .max_depth(300)
        .now(0)
        .seed(1);
    let input = input();
    let whole = expr.evaluate_with(Some(&input), &mark(bindings()), &options);
    let cut_input = reads.input().map(|read| read.cut(&input));
    let cut = expr.evaluate_with(cut_input.as_ref(), &mark(cut_bindings(reads)), &options);
    if let (Some(whole), Some(cut)) = (outcome(whole), outcome(cut)) {
        prop_assert_eq!(
            whole,
            cut,
            "{} reads {}",
            expr.source(),
            serde_json::to_string(reads).unwrap()
        );
    }
    Ok(())
}

fn read() -> impl Strategy<Value = Read> {
    let leaf = prop_oneof![Just(Read::Whole), Just(Read::Fields(BTreeMap::new()))];
    leaf.prop_recursive(4, 24, 3, |inner| {
        prop::collection::btree_map(
            prop::sample::select(&["a", "b", "items", "price"][..]).prop_map(String::from),
            inner,
            0..3,
        )
        .prop_map(Read::Fields)
    })
}

/// JSON values over the field names of [`read`], and one it never lists.
fn json_value() -> impl Strategy<Value = serde_json::Value> {
    let leaf = prop_oneof![
        Just(json!(null)),
        any::<bool>().prop_map(|b| json!(b)),
        (-1000i32..1000).prop_map(|n| json!(n)),
        (-1e6f64..1e6).prop_map(|n| json!(n)),
        "[a-z\"\\\\𝄞]{0,4}".prop_map(|s| json!(s)),
    ];
    leaf.prop_recursive(4, 32, 4, |inner| {
        let name = prop::sample::select(&["a", "b", "items", "price", "x"][..]);
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(serde_json::Value::Array),
            prop::collection::vec((name, inner), 0..4).prop_map(|fields| {
                serde_json::Value::Object(
                    fields
                        .into_iter()
                        .map(|(name, value)| (name.to_owned(), value))
                        .collect(),
                )
            }),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// ADR 0016: an expression gives the same result, or error code, on its input and
    /// bindings cut by its reads. The analysis never panics, however deep the expression.
    #[test]
    fn reads_keep_what_an_evaluation_needs(source in expression()) {
        if let Ok(expr) = Expression::compile(&source) {
            agrees_when_cut(&expr, &expr.reads())?;
        }
    }

    /// The same, with host functions that do not read the context of their call, given to
    /// the analysis.
    #[test]
    fn reads_with_marked_functions_keep_what_an_evaluation_needs(source in expression()) {
        if let Ok(expr) = Expression::compile(&source) {
            let reads = expr.reads_with(&without_input(bindings()));
            agrees_when_cut_with(&expr, &reads, true)?;
        }
    }

    /// An input cut by the union of several expressions' reads gives each its result.
    #[test]
    fn a_union_of_reads_serves_each_expression(a in valid(), b in valid()) {
        if let (Ok(a), Ok(b)) = (Expression::compile(&a), Expression::compile(&b)) {
            let union = a.reads().union(&b.reads());
            agrees_when_cut(&a, &union)?;
            agrees_when_cut(&b, &union)?;
        }
    }

    #[test]
    fn union_of_reads_is_a_join(a in read(), b in read(), c in read()) {
        prop_assert_eq!(a.union(&b), b.union(&a));
        prop_assert_eq!(a.union(&b).union(&c), a.union(&b.union(&c)));
        prop_assert_eq!(a.union(&a), a.clone());
        prop_assert_eq!(Read::Whole.union(&a), Read::Whole);
        // The union keeps what each needs.
        let value = input();
        prop_assert_eq!(a.cut(&a.union(&b).cut(&value)), a.cut(&value));
    }

    /// Parsing JSON text cut by a `Read` gives the value the `Read` cuts.
    #[test]
    fn parsing_cut_by_a_read_cuts_the_value(read in read(), value in json_value()) {
        let text = serde_json::to_string(&value).unwrap();
        let parsed = Document::parse_cut(&text, &read).unwrap();
        prop_assert_eq!(parsed.to_json(), Document::new(&read.cut(&value)).to_json());
    }

    /// Parsing any text cut by a `Read` never panics, and accepts only JSON.
    #[test]
    fn parsing_any_text_cut_by_a_read_never_panics(read in read(), text in "[\\[\\]{}\":,a-z0-9 ]{0,24}") {
        if Document::parse_cut(&text, &read).is_ok() {
            prop_assert!(serde_json::from_str::<serde_json::Value>(&text).is_ok(), "{}", text);
        }
    }
}

#[test]
fn reads_of_deep_expressions_stay_within_the_stack() {
    let deep = format!("{}a{}", "(".repeat(990), ")".repeat(990));
    let reads = Expression::compile(&deep).unwrap().reads();
    assert_eq!(
        serde_json::to_value(&reads).unwrap()["input"],
        json!({"a": true})
    );
    let long = vec!["a"; 900].join(".");
    let reads = Expression::compile(&long).unwrap().reads();
    assert!(serde_json::to_string(&reads).is_ok());
    let wide = format!("[{}]", vec!["a.b"; 10_000].join(", "));
    assert!(Expression::compile(&wide).is_ok_and(|e| e.reads().input().is_some()));
}
