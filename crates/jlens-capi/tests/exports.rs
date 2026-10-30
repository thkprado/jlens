//! The exports, called as the host calls them. The host writes buffers through their address;
//! these tests fill them with `buffers::insert`, which needs no pointer.

use jlens_capi::exports::*;
use jlens_capi::source::{
    ARRAY_WITH, DECLINED, FIELD, FIELDS, INPUT, ITEM, LENGTH, OBJECT_SOME, OBJECT_WITH, READ,
    VALUES,
};
use jlens_capi::values::{ARRAY, OBJECT, UNDEFINED};
use jlens_capi::{REPLY_ERROR, REPLY_HANDLE, REPLY_JSON, REPLY_UNDEFINED, REPLY_VALUES, buffers};
use serde_json::{Value, json};

mod host;
mod values;

#[derive(Debug, PartialEq)]
enum Reply {
    Undefined,
    Json(String),
    Error(Value),
    Handle(u64),
    Values(Vec<u8>),
}

/// Reads and frees a reply.
fn reply(handle: u64) -> Reply {
    let bytes = buffers::take(handle).expect("a reply");
    let (status, rest) = bytes.split_first().expect("a status byte");
    match *status {
        REPLY_UNDEFINED if rest.is_empty() => Reply::Undefined,
        REPLY_JSON => Reply::Json(String::from_utf8(rest.to_vec()).unwrap()),
        REPLY_ERROR => Reply::Error(serde_json::from_slice(rest).unwrap()),
        REPLY_HANDLE => Reply::Handle(u64::from_le_bytes(rest.try_into().unwrap())),
        REPLY_VALUES => Reply::Values(rest.to_vec()),
        status => panic!("unknown status {status}"),
    }
}

fn buffer(text: &str) -> u64 {
    buffers::insert(text.as_bytes().to_vec())
}

fn compile(source: &str) -> u64 {
    let text = buffer(source);
    let compiled = reply(jlens_compile(text));
    assert_eq!(jlens_buffer_free(text), 1);
    match compiled {
        Reply::Handle(handle) => handle,
        other => panic!("{source}: {other:?}"),
    }
}

fn evaluate(expression: u64, input: Option<&str>) -> Reply {
    evaluate_with(expression, input, 0)
}

fn evaluate_with(expression: u64, input: Option<&str>, options: u64) -> Reply {
    let input = input.map_or(0, buffer);
    let result = reply(jlens_evaluate(expression, input, 0, options, 0));
    if input != 0 {
        assert_eq!(jlens_buffer_free(input), 1);
    }
    result
}

/// Calls `export` with a buffer holding `text`, and frees the buffer.
fn with_buffer(text: &str, export: impl FnOnce(u64) -> u64) -> Reply {
    let buffer = buffer(text);
    let result = reply(export(buffer));
    assert_eq!(jlens_buffer_free(buffer), 1);
    result
}

fn handle(reply: Reply) -> u64 {
    match reply {
        Reply::Handle(handle) => handle,
        other => panic!("not a handle: {other:?}"),
    }
}

fn options(json: &str) -> u64 {
    handle(with_buffer(json, |b| jlens_options_new(b)))
}

fn document(text: &str, read: Option<&str>) -> u64 {
    let read = read.map_or(0, buffer);
    let made = with_buffer(text, |text| jlens_document_new(text, read));
    if read != 0 {
        assert_eq!(jlens_buffer_free(read), 1);
    }
    handle(made)
}

fn on_document(expression: u64, document: u64, options: u64) -> Reply {
    reply(jlens_evaluate_document(expression, document, 0, options, 0))
}

fn json(reply: Reply) -> Value {
    match reply {
        Reply::Json(text) => serde_json::from_str(&text).unwrap(),
        other => panic!("not JSON: {other:?}"),
    }
}

fn code(reply: &Reply) -> &str {
    match reply {
        Reply::Error(error) => error["code"].as_str().unwrap(),
        other => panic!("not an error: {other:?}"),
    }
}

#[test]
fn buffers_are_zeroed_and_freed_once() {
    let handle = jlens_buffer_new(16);
    assert_ne!(handle, 0);
    assert_ne!(jlens_buffer_address(handle), 0);
    assert_eq!(jlens_buffer_len(handle), 16);
    assert_eq!(
        buffers::with(handle, |bytes| bytes.clone()),
        Some(vec![0; 16])
    );
    assert_eq!(jlens_buffer_free(handle), 1);
    assert_eq!(jlens_buffer_free(handle), 0);
    assert_eq!(jlens_buffer_address(handle), 0);
    assert_eq!(jlens_buffer_len(handle), 0);
    assert_ne!(jlens_buffer_new(0), 0, "an empty buffer is a buffer");
    assert_eq!(jlens_buffer_new(usize::MAX), 0, "too large to allocate");
}

#[test]
fn evaluates_json_text() {
    let expression = compile("$sum(items.price)");
    assert_eq!(
        evaluate(
            expression,
            Some(r#"{"items": [{"price": 2}, {"price": 3.5}]}"#)
        ),
        Reply::Json("5.5".into())
    );
    assert_eq!(evaluate(expression, Some("{}")), Reply::Undefined);
    assert_eq!(evaluate(expression, None), Reply::Undefined);
    assert_eq!(jlens_expression_free(expression), 1);
    assert_eq!(jlens_expression_free(expression), 0);
}

#[test]
fn an_undefined_result_has_no_text_and_null_is_null() {
    let expression = compile("missing");
    assert_eq!(evaluate(expression, Some(r#"{"a": 1}"#)), Reply::Undefined);
    let expression = compile("null");
    assert_eq!(evaluate(expression, None), Reply::Json("null".into()));
}

#[test]
fn the_input_is_parsed_for_the_fields_read_only() {
    // The skipped field is never converted; the result is the same as a full parse.
    let expression = compile("a.b");
    let input = r#"{"a": {"b": [1, "é"], "c": {"deep": [[[]]]}}, "z": 1e400}"#;
    assert_eq!(
        evaluate(expression, Some(input)),
        Reply::Json(r#"[1,"é"]"#.into())
    );
}

#[test]
fn compile_errors_carry_the_envelope_in_utf16() {
    let source = buffer("\"😀\" + ");
    let compiled = reply(jlens_compile(source));
    jlens_buffer_free(source);
    let Reply::Error(error) = compiled else {
        panic!("{compiled:?}")
    };
    assert_eq!(error["code"], "S0207");
    assert!(error["message"].is_string());
    // The emoji counts two UTF-16 code units, as in Java's strings and jsonata-js.
    assert_eq!(error["position"], json!(7));
}

#[test]
fn evaluation_errors_carry_the_span_token_and_value() {
    let expression = compile("\"é\" & $x()");
    let error = evaluate(expression, None);
    assert_eq!(code(&error), "T1006");
    let Reply::Error(error) = error else {
        unreachable!()
    };
    assert_eq!((&error["start"], &error["end"]), (&json!(6), &json!(10)));
    assert_eq!(error["token"], "x");

    let expression = compile("$number(a)");
    let Reply::Error(error) = evaluate(expression, Some(r#"{"a": "one"}"#)) else {
        panic!()
    };
    assert_eq!(error["code"], "D3030");
    assert_eq!(error["value"], "one");
}

#[test]
fn input_that_is_not_json_or_not_utf8_fails_with_u2001() {
    let expression = compile("a");
    assert_eq!(code(&evaluate(expression, Some("{\"a\": "))), "U2001");
    let bytes = buffers::insert(b"{\"a\": \"\xff\"}".to_vec());
    assert_eq!(
        code(&reply(jlens_evaluate(expression, bytes, 0, 0, 0))),
        "U2001"
    );
    jlens_buffer_free(bytes);
}

#[test]
fn an_expression_that_is_not_utf8_fails_with_u2003() {
    let source = buffers::insert(b"a\xc3".to_vec());
    assert_eq!(code(&reply(jlens_compile(source))), "U2003");
    jlens_buffer_free(source);
}

#[test]
fn unknown_and_freed_handles_fail_with_u2002() {
    let expression = compile("1");
    jlens_expression_free(expression);
    let error = evaluate(expression, None);
    assert_eq!(code(&error), "U2002");
    let Reply::Error(error) = error else {
        unreachable!()
    };
    assert_eq!(error["token"], "expression");
    assert_eq!(error["value"], json!(expression));

    let expression = compile("1");
    let input = buffer("{}");
    jlens_buffer_free(input);
    assert_eq!(
        code(&reply(jlens_evaluate(expression, input, 0, 0, 0))),
        "U2002"
    );
    assert_eq!(code(&reply(jlens_compile(input))), "U2002");
    assert_eq!(code(&reply(jlens_evaluate(u64::MAX, 0, 0, 0, 0))), "U2002");
    assert_eq!(code(&reply(jlens_compile(0))), "U2002");
}

#[test]
fn a_panic_becomes_u2004_or_zero() {
    let caught = jlens_capi::guarded(jlens_capi::api::panicked, || panic!("on purpose"));
    let Reply::Error(error) = reply(caught) else {
        panic!()
    };
    assert_eq!(error["code"], "U2004");
    assert!(error["message"].as_str().unwrap().contains("on purpose"));
    let zero: usize = jlens_capi::guarded(jlens_capi::zero, || panic!("on purpose"));
    assert_eq!(zero, 0);
}

#[test]
fn deep_expressions_stop_with_an_error() {
    let source = format!("{}1{}", "[".repeat(5_000), "]".repeat(5_000));
    let compiled = reply(jlens_compile(buffer(&source)));
    assert!(matches!(compiled, Reply::Error(_)), "{compiled:?}");
}

#[test]
fn threads_share_expressions_and_tables() {
    let expression = compile("$count(items[price > 1])");
    let threads: Vec<_> = (0..8)
        .map(|thread| {
            std::thread::spawn(move || {
                for i in 0..200u32 {
                    let items: Vec<_> = (0..i % 7).map(|p| json!({ "price": p })).collect();
                    let expected = (i % 7).saturating_sub(2);
                    let input = json!({ "items": items }).to_string();
                    assert_eq!(
                        evaluate(expression, Some(&input)),
                        Reply::Json(expected.to_string()),
                        "thread {thread}, case {i}"
                    );
                    // Each thread compiles and frees its own too, reusing entries of the
                    // tables while the others use theirs.
                    let own = compile(&format!("{i} + {thread}"));
                    assert_eq!(evaluate(own, None), Reply::Json((i + thread).to_string()));
                    assert_eq!(jlens_expression_free(own), 1);
                    assert_eq!(code(&evaluate(own, None)), "U2002");
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(jlens_expression_free(expression), 1);
}

#[test]
fn an_expression_freed_during_an_evaluation_finishes_it() {
    // The evaluation holds its own reference; the free only removes the handle.
    let expression = compile("$count([1..2000000])");
    let running = std::thread::spawn(move || evaluate(expression, None));
    jlens_expression_free(expression);
    let result = running.join().unwrap();
    assert!(
        result == Reply::Json("2000000".into()) || code(&result) == "U2002",
        "{result:?}"
    );
}

#[test]
fn each_limit_stops_the_evaluation_with_its_code() {
    let cases = [
        (
            "$sum($map([1..300000], function($x) { $x }))",
            r#"{"timeout": 1}"#,
            "D1012",
        ),
        (
            "($f := function($n) { $n = 0 ? 0 : 1 + $f($n - 1) }; $f(100))",
            r#"{"maxDepth": 10}"#,
            "D1011",
        ),
        (
            "$sum($map([1..100], function($x) { $x }))",
            r#"{"maxSteps": 10}"#,
            "U1006",
        ),
        (
            "$join([1..100].$string(), \"\")",
            r#"{"maxStringLength": 10}"#,
            "U1001",
        ),
        ("[1..100]", r#"{"maxArrayLength": 10}"#, "U1001"),
        ("[1..100]", r#"{"maxResultSize": 10}"#, "U1001"),
    ];
    for (source, given, expected) in cases {
        let expression = compile(source);
        let options = options(given);
        assert!(
            !matches!(evaluate(expression, None), Reply::Error(_)),
            "{source} without {given}"
        );
        let error = evaluate_with(expression, None, options);
        assert_eq!(code(&error), expected, "{source} with {given}");
        let document = document("{}", None);
        let error = on_document(expression, document, options);
        assert_eq!(code(&error), expected, "{source} on a document");
        jlens_document_free(document);
        assert_eq!(jlens_options_free(options), 1);
        assert_eq!(jlens_options_free(options), 0);
        jlens_expression_free(expression);
    }
    let limited = options(r#"{"maxSteps": 10}"#);
    let Reply::Error(error) = evaluate_with(
        compile("$sum($map([1..100], function($x) { $x }))"),
        None,
        limited,
    ) else {
        panic!()
    };
    assert_eq!(error["value"], json!(10));
}

#[test]
fn the_clock_and_the_seed_are_options() {
    let fixed = options(r#"{"now": 1000, "seed": 42}"#);
    let now = compile("[$millis(), $now()]");
    assert_eq!(
        json(evaluate_with(now, None, fixed)),
        json!([1000, "1970-01-01T00:00:01.000Z"])
    );
    let random = compile("[$random(), $shuffle([1..20])]");
    let first = json(evaluate_with(random, None, fixed));
    assert_eq!(json(evaluate_with(random, None, fixed)), first);
    let document = document("[]", None);
    assert_eq!(json(on_document(random, document, fixed)), first);
    let other = options(r#"{"seed": 43, "timeout": null}"#);
    assert_ne!(json(evaluate_with(random, None, other)), first);
    jlens_document_free(document);
}

#[test]
fn options_not_in_their_form_fail_with_u2005() {
    for given in [
        r#"{"maxdepth": 10}"#,
        r#"{"timeout": -1}"#,
        r#"{"maxSteps": -1}"#,
        r#"{"seed": 1.5}"#,
        r#"{"now": "2026-10-25"}"#,
        "[]",
        "{",
    ] {
        let error = with_buffer(given, |b| jlens_options_new(b));
        assert_eq!(code(&error), "U2005", "{given}");
        let Reply::Error(error) = error else {
            unreachable!()
        };
        assert_eq!(error["token"], "options");
    }
    let bytes = buffers::insert(b"{\"seed\": \xff}".to_vec());
    assert_eq!(code(&reply(jlens_options_new(bytes))), "U2005");
    jlens_buffer_free(bytes);
    assert_eq!(code(&reply(jlens_options_new(0))), "U2002");
    // Freed options are unknown, not the defaults.
    let freed = options("{}");
    jlens_options_free(freed);
    let Reply::Error(error) = evaluate_with(compile("1"), None, freed) else {
        panic!()
    };
    assert_eq!(
        (&error["code"], &error["token"]),
        (&json!("U2002"), &json!("options"))
    );
}

#[test]
fn documents_are_evaluated_and_freed_once() {
    let text = r#"{"items":[{"price":2},{"price":3.5}]}"#;
    let document = document(text, None);
    let sum = compile("$sum(items.price)");
    assert_eq!(on_document(sum, document, 0), Reply::Json("5.5".into()));
    assert_eq!(
        on_document(compile("$"), document, 0),
        Reply::Json(text.into())
    );
    assert_eq!(
        on_document(compile("missing"), document, 0),
        Reply::Undefined
    );
    assert_eq!(jlens_document_free(document), 1);
    assert_eq!(jlens_document_free(document), 0);
    let Reply::Error(error) = on_document(sum, document, 0) else {
        panic!()
    };
    assert_eq!(
        (&error["code"], &error["token"]),
        (&json!("U2002"), &json!("document"))
    );
}

#[test]
fn a_document_that_is_not_json_or_not_utf8_fails_with_u2001() {
    let error = with_buffer("{\"a\": ", |text| jlens_document_new(text, 0));
    assert_eq!(code(&error), "U2001");
    let bytes = buffers::insert(b"\"\xff\"".to_vec());
    assert_eq!(code(&reply(jlens_document_new(bytes, 0))), "U2001");
    jlens_buffer_free(bytes);
    assert_eq!(code(&reply(jlens_document_new(0, 0))), "U2002");
}

#[test]
fn reads_are_json_and_their_union_prunes_a_document() {
    let (a, c) = (compile("a.b"), compile("$sum(c)"));
    let reads_a = json(reply(jlens_reads(a)));
    assert_eq!(
        reads_a,
        json!({"input": {"a": {"b": true}}, "bindings": {}})
    );
    let reads_c = json(reply(jlens_reads(c)));
    let all = json!([reads_a, reads_c]).to_string();
    let union = json(with_buffer(&all, |b| jlens_union_reads(b)));
    assert_eq!(union["input"], json!({"a": {"b": true}, "c": true}));
    let none = json(with_buffer("[]", |b| jlens_union_reads(b)));
    assert_eq!(none, json!({"bindings": {}}));

    let input = r#"{"a": {"b": 1, "x": 2}, "c": [1, 2], "d": 3}"#;
    let pruned = document(input, Some(&union["input"].to_string()));
    assert_eq!(on_document(a, pruned, 0), Reply::Json("1".into()));
    assert_eq!(on_document(c, pruned, 0), Reply::Json("3".into()));
    // What the read leaves out is not in the document, as with Python's `read=`.
    assert_eq!(on_document(compile("d"), pruned, 0), Reply::Undefined);
    assert_eq!(on_document(compile("a.x"), pruned, 0), Reply::Undefined);
    let whole = document(input, None);
    assert_eq!(on_document(compile("d"), whole, 0), Reply::Json("3".into()));
    // An expression that does not read its input reads no field of it.
    let constant = json(reply(jlens_reads(compile("1 + 1"))));
    assert_eq!(constant, json!({"bindings": {}}));
    jlens_document_free(pruned);
    jlens_document_free(whole);
    assert_eq!(code(&reply(jlens_reads(u64::MAX))), "U2002");
}

#[test]
fn reads_not_in_their_form_fail_with_u2005() {
    for given in ["{}", "[{\"input\": 1}]", "[{\"inputs\": true}]", "["] {
        let Reply::Error(error) = with_buffer(given, |b| jlens_union_reads(b)) else {
            panic!("{given}")
        };
        assert_eq!(
            (&error["code"], &error["token"]),
            (&json!("U2005"), &json!("reads")),
            "{given}"
        );
    }
    for given in ["false", "1", "{\"a\": 1}", "[", "\"a\""] {
        let read = buffer(given);
        let error = with_buffer("{}", |text| jlens_document_new(text, read));
        jlens_buffer_free(read);
        let Reply::Error(error) = error else {
            panic!("{given}")
        };
        assert_eq!(
            (&error["code"], &error["token"]),
            (&json!("U2005"), &json!("read")),
            "{given}"
        );
    }
    let freed = buffer("true");
    jlens_buffer_free(freed);
    let error = with_buffer("{}", |text| jlens_document_new(text, freed));
    assert_eq!(code(&error), "U2002");
}

#[test]
fn threads_share_a_document_each_with_its_own_conversion() {
    let items: Vec<_> = (0..100).map(|p| json!({ "price": p })).collect();
    let document = document(&json!({ "items": items }).to_string(), None);
    let expression = compile("$count(items[price > 49])");
    let limited = options(r#"{"maxSteps": 5}"#);
    let threads: Vec<_> = (0..8)
        .map(|_| {
            std::thread::spawn(move || {
                assert_eq!(jlens_capi::documents::cached(), 0);
                for _ in 0..100 {
                    let result = on_document(expression, document, 0);
                    assert_eq!(result, Reply::Json("50".into()));
                    assert_eq!(code(&on_document(expression, document, limited)), "U1006");
                }
                // One conversion per thread, kept between evaluations.
                assert_eq!(jlens_capi::documents::cached(), 1);
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(jlens_capi::documents::cached(), 1, "made on this thread");
    assert_eq!(jlens_document_free(document), 1);
    assert_eq!(jlens_capi::documents::cached(), 0, "dropped by the free");
}

#[test]
fn a_thread_drops_the_conversions_of_freed_documents() {
    let first = document("[1]", None);
    let expression = compile("$sum($)");
    let (made, made_rx) = std::sync::mpsc::channel();
    let (freed, freed_rx) = std::sync::mpsc::channel::<()>();
    let worker = std::thread::spawn(move || {
        assert_eq!(on_document(expression, first, 0), Reply::Json("1".into()));
        made.send(jlens_capi::documents::cached()).unwrap();
        freed_rx.recv().unwrap();
        // The next document this thread makes drops the conversion of the one freed meanwhile.
        let second = document("[2]", None);
        assert_eq!(jlens_capi::documents::cached(), 1);
        assert_eq!(on_document(expression, second, 0), Reply::Json("2".into()));
        jlens_document_free(second);
        jlens_capi::documents::cached()
    });
    assert_eq!(made_rx.recv().unwrap(), 1);
    jlens_document_free(first);
    freed.send(()).unwrap();
    assert_eq!(worker.join().unwrap(), 0);
}

#[test]
fn a_thread_drops_a_freed_documents_conversion_at_its_next_call_of_any_export() {
    let document = document(r#"{"a": 1}"#, None);
    let expression = compile("a");
    let (made, made_rx) = std::sync::mpsc::channel();
    let (freed, freed_rx) = std::sync::mpsc::channel::<()>();
    let worker = std::thread::spawn(move || {
        assert_eq!(
            on_document(expression, document, 0),
            Reply::Json("1".into())
        );
        made.send(jlens_capi::documents::cached()).unwrap();
        freed_rx.recv().unwrap();
        let kept = jlens_capi::documents::cached();
        // A buffer has nothing to do with documents; the call sweeps all the same.
        jlens_buffer_free(jlens_buffer_new(1));
        (kept, jlens_capi::documents::cached())
    });
    assert_eq!(made_rx.recv().unwrap(), 1);
    assert_eq!(jlens_document_free(document), 1);
    freed.send(()).unwrap();
    assert_eq!(
        worker.join().unwrap(),
        (1, 0),
        "kept until the call, dropped by it"
    );
}

/// Evaluates with the result as values: the decoded value, `None` when undefined.
fn values_of(expression: u64, input: Option<&str>, options: u64) -> Option<Value> {
    let input = input.map_or(0, buffer);
    let replied = reply(jlens_evaluate(expression, input, 0, options, 1));
    if input != 0 {
        assert_eq!(jlens_buffer_free(input), 1);
    }
    match replied {
        Reply::Values(bytes) => Some(values::decode(&bytes)),
        Reply::Undefined => None,
        other => panic!("not values: {other:?}"),
    }
}

#[test]
fn values_hold_each_kind_of_value() {
    let expression = compile(
        r#"{"null": null, "false": false, "true": true, "int": 5, "neg": -3, "frac": 1.5,
            "big": 1e20, "huge": 1e300, "edge": 9007199254740993, "s": "é😀",
            "list": [1, "a", null, $u], "empty": [], "object": {}, "undefined": $u,
            "rows": [{"k": 1, "j": [true]}, {"k": 2}]}"#,
    );
    let got = values_of(expression, None, 0).unwrap();
    assert_eq!(
        got,
        json!({"null": null, "false": false, "true": true, "int": 5, "neg": -3, "frac": 1.5,
               "big": 1e20, "huge": 1e300, "edge": 9_007_199_254_740_992_i64, "s": "é😀",
               "list": [1, "a", null], "empty": [], "object": {},
               "rows": [{"k": 1, "j": [true]}, {"k": 2}]})
    );
    // Integral numbers in the range of an `i64` are integers, the others floats.
    assert!(got["int"].is_i64() && got["neg"].is_i64() && got["edge"].is_i64());
    assert!(got["frac"].is_f64() && got["big"].is_f64() && got["huge"].is_f64());
    // The fields come in their order, the undefined one left out.
    let keys: Vec<&str> = got
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys[..3], ["null", "false", "true"]);
    assert!(!keys.contains(&"undefined"));
    assert_eq!(jlens_expression_free(expression), 1);
}

#[test]
fn values_write_each_key_once() {
    use jlens_capi::values::{ARRAY, INTEGER, OBJECT, STRING};
    let expression = compile(r#"[{"a": 1}, {"a": "b"}]"#);
    let bytes = match reply(jlens_evaluate(expression, 0, 0, 0, 1)) {
        Reply::Values(bytes) => bytes,
        other => panic!("not values: {other:?}"),
    };
    let mut expected = vec![ARRAY, 2, 0, 0, 0];
    // The first object: its value, then its key, new (index 0, then its text).
    expected.extend([OBJECT, 1, 0, 0, 0, INTEGER]);
    expected.extend(1i64.to_le_bytes());
    expected.extend([0, 0, 0, 0, 1, 0, 0, 0, b'a']);
    // The second: the same key, by its index alone.
    expected.extend([OBJECT, 1, 0, 0, 0, STRING, 1, 0, 0, 0, b'b', 0, 0, 0, 0]);
    assert_eq!(bytes, expected);
    assert_eq!(jlens_expression_free(expression), 1);
}

#[test]
fn values_come_from_text_documents_and_no_input_within_the_limits() {
    let expression = compile("items.name");
    let text = r#"{"items": [{"name": "x"}, {"name": "y"}]}"#;
    assert_eq!(
        values_of(expression, Some(text), 0),
        Some(json!(["x", "y"]))
    );
    assert_eq!(values_of(expression, Some("{}"), 0), None);
    let doc = document(text, None);
    match reply(jlens_evaluate_document(expression, doc, 0, 0, 1)) {
        Reply::Values(bytes) => assert_eq!(values::decode(&bytes), json!(["x", "y"])),
        other => panic!("not values: {other:?}"),
    }
    let small = options(r#"{"maxResultSize": 4}"#);
    assert_eq!(
        code(&reply(jlens_evaluate_document(
            expression, doc, 0, small, 1
        ))),
        "U1001"
    );
    // A format that is neither 0 nor 1.
    assert_eq!(
        code(&reply(jlens_evaluate_document(expression, doc, 0, 0, 2))),
        "U2005"
    );
    assert_eq!(
        code(&reply(jlens_evaluate(expression, 0, 0, 0, 7))),
        "U2005"
    );
    let failing = compile("$x()");
    assert_eq!(code(&reply(jlens_evaluate(failing, 0, 0, 0, 1))), "T1006");
    assert_eq!(jlens_options_free(small), 1);
    assert_eq!(jlens_document_free(doc), 1);
    assert_eq!(jlens_expression_free(expression), 1);
    assert_eq!(jlens_expression_free(failing), 1);
}

/// Evaluates `expression` against `input` as the host's own values, read in mode `context`
/// ([`host`]): the reply, the reader's calls and the host's nodes.
fn on_host(
    expression: u64,
    input: &Value,
    context: u64,
    result: u32,
) -> (Reply, Vec<(u32, u32)>, Vec<Value>) {
    let level = host::Host::start_in(input, context);
    let replied = reply(jlens_evaluate_source(
        expression,
        level,
        Some(host::READER),
        context,
        0,
        0,
        result,
    ));
    host::Host::finish();
    assert_eq!(jlens_buffer_free(level), 1);
    host::HOST.with_borrow_mut(|host| {
        (
            replied,
            std::mem::take(&mut host.calls),
            std::mem::take(&mut host.nodes),
        )
    })
}

/// The host's values give what their JSON text gives, as JSON text or values, whether the
/// reader reads one field or item alone or declines to.
#[test]
fn the_hosts_values_give_what_their_json_text_gives() {
    let input = json!({
        "items": [
            {"name": "pen", "price": 1.5, "tags": ["a", "b"]},
            {"name": "ink", "price": 4, "tags": []},
            {"name": "é𝄞", "price": -0.25, "stock": {"qty": 3}}
        ],
        "total": 9007199254740993u64,
        "none": null,
        "yes": true
    });
    let text = input.to_string();
    for source in [
        "$",
        "items[0].name",
        "items[-1].stock.qty",
        "($i := 1; items[$i].name)",
        "$sum(items.price)",
        "items[price > 1].name",
        "$count(items)",
        "total",
        "[none, yes]",
        "items.tags",
        "$keys($)",
        "items ~> |$|{'name': 'x'}|",
        "$string(items[0])",
        "nothing",
    ] {
        let expression = compile(source);
        let expected = evaluate(expression, Some(&text));
        for context in [host::PLAIN, host::WHOLE, host::WITH, host::SOME] {
            let (json, _, _) = on_host(expression, &input, context, 0);
            assert_eq!(json, expected, "{source} as JSON text");
            let (values, _, nodes) = on_host(expression, &input, context, 1);
            match (values, &expected) {
                (Reply::Values(bytes), Reply::Json(text)) => assert_eq!(
                    values::as_f64(values::decode_with(&bytes, &nodes)),
                    values::as_f64(serde_json::from_str(text).unwrap()),
                    "{source} as values"
                ),
                (got, expected) => assert_eq!(&got, expected, "{source} as values"),
            }
        }
        assert_eq!(jlens_expression_free(expression), 1);
    }
}

/// A path and an index read one field and one item at a time, not the levels they pass
/// through; a reader that declines has them read whole.
#[test]
fn one_field_or_item_is_read_alone() {
    let input = json!({"items": [{"name": "pen", "qty": 1}, {"name": "ink", "qty": 2}]});
    for source in ["items[0].name", "($i := 0; items[$i].name)"] {
        let expression = compile(source);
        let (reply, calls, _) = on_host(expression, &input, host::PLAIN, 1);
        assert!(matches!(reply, Reply::Values(_)), "{source}: {reply:?}");
        let ops: Vec<u32> = calls.iter().map(|&(op, _)| op).collect();
        assert!(!ops.contains(&READ), "{source}: {calls:?}");
        assert!(
            ops.contains(&LENGTH) && ops.contains(&ITEM),
            "{source}: {calls:?}"
        );
        let (_, calls, _) = on_host(expression, &input, host::WHOLE, 1);
        assert!(
            calls.iter().any(|&(op, _)| op == READ),
            "{source}: {calls:?}"
        );
        assert_eq!(jlens_expression_free(expression), 1);
    }
}

/// A small array or object written with its contents is read without calling the reader
/// again: its fields, its length, its items, and itself whole.
#[test]
fn a_node_written_with_its_contents_is_read_without_calls() {
    let input = json!({"items": [{"name": "pen", "qty": 1}, {"name": "ink", "qty": 2}]});
    for (source, expected) in [
        ("$sum(items.qty)", json!(3)),
        ("items[1].name", json!("ink")),
        ("$count(items[0].*)", json!(2)),
        ("$string(items[0])", json!(r#"{"name":"pen","qty":1}"#)),
    ] {
        let expression = compile(source);
        let (reply, calls, nodes) = on_host(expression, &input, host::WITH, 1);
        match reply {
            Reply::Values(bytes) => assert_eq!(values::decode_with(&bytes, &nodes), expected),
            other => panic!("{source}: {other:?}"),
        }
        // The input, an object of one field, is written with it; only `items` is read, and its
        // items never.
        assert!(
            calls.contains(&(READ, 1))
                && calls
                    .iter()
                    .all(|&call| call == (READ, 1) || call == (LENGTH, 1)),
            "{source}: {calls:?}"
        );
        assert_eq!(jlens_expression_free(expression), 1);
    }
}

/// An object written with some of its fields answers those, and a field it lacks, without
/// calling the reader; any other field, and the object whole, are read from the reader.
#[test]
fn an_object_written_with_some_fields_reads_the_others_from_the_reader() {
    // In mode SOME, the input is written with `a` and `ccc`, and `absent` as lacking. The keys'
    // lengths differ, so that the host tells them apart.
    let input = json!({"a": 1, "bb": 2, "ccc": 3});
    for (source, expected, called) in [
        ("a + ccc", json!(4), vec![]),
        ("absent", Value::Null, vec![]),
        ("bb", json!(2), vec![(FIELD, 0)]),
        ("$keys($)", json!(["a", "bb", "ccc"]), vec![(READ, 0)]),
    ] {
        let expression = compile(source);
        let (reply, calls, nodes) = on_host(expression, &input, host::SOME, 1);
        match reply {
            Reply::Values(bytes) => assert_eq!(values::decode_with(&bytes, &nodes), expected),
            Reply::Undefined => assert_eq!(expected, Value::Null, "{source}"),
            other => panic!("{source}: {other:?}"),
        }
        assert_eq!(calls, called, "{source}");
        assert_eq!(jlens_expression_free(expression), 1);
    }
}

/// An array or object of the input that the result holds unchanged comes back as its number,
/// so that the host returns its own; a value built is written in full.
#[test]
fn a_result_that_is_the_input_is_its_number() {
    let input = json!({"items": [{"name": "pen"}, {"name": "ink"}], "o": {"p": [1]}});
    let numbers = |source: &str| {
        let expression = compile(source);
        let (reply, _, nodes) = on_host(expression, &input, host::PLAIN, 1);
        assert_eq!(jlens_expression_free(expression), 1);
        match reply {
            Reply::Values(bytes) => (bytes, nodes),
            other => panic!("{source}: {other:?}"),
        }
    };
    let (bytes, nodes) = numbers("$");
    assert_eq!(bytes, [INPUT, 0, 0, 0, 0]);
    assert_eq!(nodes[0], input);
    let (bytes, nodes) = numbers("o.p");
    assert_eq!(bytes[0], INPUT);
    assert_eq!(values::decode_with(&bytes, &nodes), json!([1]));
    // An array built of the input's objects: the array in full, each object by its number.
    let (bytes, nodes) = numbers("items[name != 'x']");
    assert_eq!(bytes[0], ARRAY);
    assert_eq!(bytes.iter().filter(|&&b| b == INPUT).count(), 2);
    assert_eq!(values::decode_with(&bytes, &nodes), input["items"]);
    // A copy is built.
    let (bytes, _) = numbers("o ~> |$|{'q': 1}|");
    assert!(!bytes.contains(&INPUT));
    // As JSON text, the input is written in full.
    let expression = compile("$");
    let (reply, _, _) = on_host(expression, &input, host::PLAIN, 0);
    assert_eq!(json(reply), input);
    assert_eq!(jlens_expression_free(expression), 1);
}

/// A reader that fails, or returns a level not in its form, stops the evaluation with `U1004`;
/// input nested too deep for the result fails with `U1001`; no reader is `U2005` and an unknown
/// input buffer `U2002`.
#[test]
fn the_reader_fails_cleanly() {
    let input = json!({"a": {"b": 1}});
    let expression = compile("a.b");
    for garbage in [
        host::failed("no such thing"),
        vec![],
        vec![9, 0, 0, 0, 0, 1, 0, 0, 0, 99, 0, 0, 0, 0],
        vec![1, 0, 0, 0, DECLINED],
        vec![200, 0, 0, 0, 0],
        // An object written with its contents inside the contents of another.
        vec![
            28,
            0,
            0,
            0,
            VALUES,
            1,
            0,
            0,
            0,
            OBJECT_WITH,
            5,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            b'b',
            OBJECT_WITH,
            6,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ],
        // A field lacking among all the fields of an object.
        vec![
            20,
            0,
            0,
            0,
            VALUES,
            1,
            0,
            0,
            0,
            OBJECT_WITH,
            5,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            b'b',
            UNDEFINED,
        ],
        // Some fields of an object, cut short.
        vec![
            14,
            0,
            0,
            0,
            VALUES,
            1,
            0,
            0,
            0,
            OBJECT_SOME,
            5,
            0,
            0,
            0,
            2,
            0,
            0,
            0,
        ],
        // Contents cut short.
        vec![
            14, 0, 0, 0, VALUES, 1, 0, 0, 0, ARRAY_WITH, 5, 0, 0, 0, 3, 0, 0, 0,
        ],
        // A node numbered past any the host wrote: not held, so no table of its size.
        vec![
            14, 0, 0, 0, VALUES, 1, 0, 0, 0, ARRAY_WITH, 255, 255, 255, 255, 3, 0, 0, 0,
        ],
    ] {
        host::HOST.with_borrow_mut(|host| host.garbage = garbage.clone());
        let (reply, _, _) = on_host(expression, &input, host::GARBAGE, 0);
        assert_eq!(code(&reply), "U1004", "{garbage:?}: {reply:?}");
    }
    host::HOST.with_borrow_mut(|host| host.garbage = host::failed("no such thing"));
    let (reply, _, _) = on_host(expression, &input, host::GARBAGE, 1);
    match reply {
        Reply::Error(error) => {
            let message = error["message"].as_str().unwrap();
            assert!(message.ends_with("no such thing"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    // The test's host clones a level's values recursively, inside the core's recursion: a stack
    // of its own for it.
    let (stringify, whole) = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(|| {
            let mut deep = json!(1);
            for _ in 0..2000 {
                deep = Value::Array(vec![deep]);
            }
            let stringify = compile("$string($)");
            let (reply, _, _) = on_host(stringify, &deep, host::PLAIN, 0);
            assert_eq!(code(&reply), "U1001", "{reply:?}");
            let whole = compile("$");
            let (reply, _, _) = on_host(whole, &deep, host::PLAIN, 0);
            assert_eq!(code(&reply), "U1001", "{reply:?}");
            // As values, the input is returned whole, never built.
            let (reply, _, _) = on_host(whole, &deep, host::PLAIN, 1);
            assert_eq!(reply, Reply::Values(vec![INPUT, 0, 0, 0, 0]));
            // Reading an array nested more than 1,000 levels deep fails (0.26 #9).
            let count = compile("$count($.**)");
            let (reply, _, _) = on_host(count, &deep, host::PLAIN, 0);
            assert_eq!(code(&reply), "U1004", "{reply:?}");
            assert_eq!(jlens_expression_free(count), 1);
            (stringify, whole)
        })
        .unwrap()
        .join()
        .unwrap();
    let level = host::Host::start(&input);
    let no_reader = crate::reply(jlens_evaluate_source(expression, level, None, 0, 0, 0, 0));
    assert_eq!(code(&no_reader), "U2005");
    assert_eq!(jlens_buffer_free(level), 1);
    let unknown = crate::reply(jlens_evaluate_source(
        expression,
        level,
        Some(host::READER),
        0,
        0,
        0,
        0,
    ));
    assert_eq!(code(&unknown), "U2002");
    for expression in [expression, stringify, whole] {
        assert_eq!(jlens_expression_free(expression), 1);
    }
}

/// A level that holds its own node, an array or object inside itself, never makes an
/// evaluation run for ever: the node is one level deeper at each read, and one nested more
/// than 1,000 levels deep is not read (0.26 #9). Each expression of the property tests ends.
#[test]
fn a_node_inside_itself_stops_the_evaluation() {
    let input = json!({"a": [{"b": 1}]});
    let levels = [
        // Every call answers node 0, the input, as an array holding node 0 ...
        vec![10, 0, 0, 0, VALUES, 1, 0, 0, 0, ARRAY, 0, 0, 0, 0],
        // ... as an object, the value of every field ...
        vec![10, 0, 0, 0, VALUES, 1, 0, 0, 0, OBJECT, 0, 0, 0, 0],
        // ... as fields holding node 0 ...
        vec![
            25, 0, 0, 0, FIELDS, 2, 0, 0, 0, 1, 0, 0, 0, b'a', OBJECT, 0, 0, 0, 0, 1, 0, 0, 0,
            b'b', ARRAY, 0, 0, 0, 0,
        ],
        // ... or as an array written with its items, node 0 again, read without calling the
        // reader.
        vec![
            19, 0, 0, 0, VALUES, 1, 0, 0, 0, ARRAY_WITH, 0, 0, 0, 0, 1, 0, 0, 0, ARRAY, 0, 0, 0, 0,
        ],
    ];
    let expressions = [
        "$",
        "a",
        "a.b",
        "*",
        "**",
        "$.*[0]",
        "a[0]",
        "a[-1].b",
        "($i := 1; a[$i])",
        "$count(a)",
        "$string($)",
        "$keys($)",
        "[$]",
        "{'v': a}",
        "$ ~> | $ | {'x': 1} |",
        "a[b = 1]",
        "$sort(a)",
    ];
    let (done, ended) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            host::Host::know(&["a", "b", "x", "v"]);
            let mut outcomes = Vec::new();
            for (n, level) in levels.iter().enumerate() {
                for text in expressions {
                    let expression = compile(text);
                    for result in 0..2 {
                        host::HOST.with_borrow_mut(|host| host.garbage = level.clone());
                        let (reply, _, _) = on_host(expression, &input, host::GARBAGE, result);
                        let outcome = match reply {
                            Reply::Error(error) => error["code"].as_str().unwrap().to_owned(),
                            _ => "a result".to_owned(),
                        };
                        outcomes.push((n, text, result, outcome));
                    }
                    assert_eq!(jlens_expression_free(expression), 1);
                }
            }
            done.send(outcomes).unwrap();
        })
        .unwrap();
    let outcomes = ended
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("every evaluation ends");
    for (level, text, result, outcome) in outcomes {
        match outcome.as_str() {
            // An array inside itself, flattened for ever before 0.26 #9.
            _ if text == "a.b" && (level == 0 || level == 3) => {
                assert_eq!(outcome, "U1004", "level {level}")
            }
            "a result" | "U1001" | "U1004" => {}
            _ => panic!("level {level}, {text}, result {result}: {outcome}"),
        }
    }
}
