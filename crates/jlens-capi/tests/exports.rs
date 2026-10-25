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
    let input = input.map_or(0, buffer);
    let result = reply(jlens_evaluate(expression, input));
    if input != 0 {
        assert_eq!(jlens_buffer_free(input), 1);
    }
    result
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
    assert_eq!(code(&reply(jlens_evaluate(expression, bytes))), "U2001");
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
    assert_eq!(code(&reply(jlens_evaluate(expression, input))), "U2002");
    assert_eq!(code(&reply(jlens_compile(input))), "U2002");
    assert_eq!(code(&reply(jlens_evaluate(u64::MAX, 0))), "U2002");
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
