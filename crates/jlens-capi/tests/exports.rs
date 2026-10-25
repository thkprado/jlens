//! The exports, called as the host calls them. The host writes buffers through their address;
//! these tests fill them with `buffers::insert`, which needs no pointer.

use jlens_capi::exports::*;
use jlens_capi::{REPLY_ERROR, REPLY_HANDLE, REPLY_JSON, REPLY_UNDEFINED, buffers};
use serde_json::{Value, json};

#[derive(Debug, PartialEq)]
enum Reply {
    Undefined,
    Json(String),
    Error(Value),
    Handle(u64),
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
    let result = reply(jlens_evaluate(expression, input, options));
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
    reply(jlens_evaluate_document(expression, document, options))
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
    assert_eq!(code(&reply(jlens_evaluate(expression, bytes, 0))), "U2001");
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
    assert_eq!(code(&reply(jlens_evaluate(expression, input, 0))), "U2002");
    assert_eq!(code(&reply(jlens_compile(input))), "U2002");
    assert_eq!(code(&reply(jlens_evaluate(u64::MAX, 0, 0))), "U2002");
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
