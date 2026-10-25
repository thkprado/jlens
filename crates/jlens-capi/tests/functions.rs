//! Bindings and host functions, called as the host calls them (0.25 #7). The host's function
//! is a Rust `extern "C"` function here, which reads and writes buffers by handle.

use std::cell::Cell;

use jlens_capi::exports::*;
use jlens_capi::values::{ARRAY, Encoder, UNDEFINED};
use jlens_capi::{REPLY_ERROR, REPLY_HANDLE, REPLY_JSON, REPLY_UNDEFINED, REPLY_VALUES, buffers};
use serde_json::{Value, json};

mod values;

/// The functions of [`bindings`], by index; `function` runs them.
const FUNCTIONS: &str = r#"[
    {"name": "add", "arity": 2},
    {"name": "apply", "signature": "<fx?:x>"},
    {"name": "fail"},
    {"name": "pass", "arity": 1},
    {"name": "keep", "arity": 1},
    {"name": "args"}
]"#;

/// The context the bindings are made with, which `function` checks.
const CONTEXT: u64 = 7;

thread_local! {
    /// The frame `$keep` was called in.
    static KEPT: Cell<u64> = const { Cell::new(0) };
}

extern "C" fn function(context: u64, index: u32, frame: u64, arguments: u64) -> u64 {
    assert_eq!(context, CONTEXT);
    let args = buffers::with(arguments, |bytes| values::decode(bytes)).expect("the arguments");
    let args = args.as_array().expect("an array").clone();
    let reply = match index {
        // $add($a, $b)
        0 => values(&json!(
            args[0].as_f64().unwrap() + args[1].as_f64().unwrap()
        )),
        // $apply($f, $x): $f($x), or $f() without $x
        1 => apply(frame, args.get(1)),
        // $fail(): an exception, kept by the host as number 3
        2 => error(json!({"code": "Boom", "message": "The rate service is down", "exception": 3})),
        // $pass($f): $f(), its error let through
        3 => apply(frame, None),
        // $keep($f): $f(), its frame kept to call after it returned
        4 => {
            KEPT.set(frame);
            apply(frame, None)
        }
        // $args(...): the arguments as they came, markers included
        _ => values(&Value::Array(args)),
    };
    buffers::insert(reply)
}

/// Calls argument 0 of call `frame`, with `argument` when it is given, and replies with what
/// the call replied, or with the failure it gave.
fn apply(frame: u64, argument: Option<&Value>) -> Vec<u8> {
    let mut encoder = Encoder::new(vec![ARRAY]);
    match argument {
        Some(Value::Object(marker)) if marker.contains_key("undefined") => {
            encoder.count(1);
            encoder.out.push(UNDEFINED);
        }
        Some(value) => {
            encoder.count(1);
            encoder.json(value);
        }
        None => encoder.count(0),
    }
    let arguments = buffers::insert(encoder.out);
    let reply = buffers::take(jlens_call(frame, 0, arguments)).expect("a reply");
    assert_eq!(jlens_buffer_free(arguments), 1);
    match reply[0] {
        REPLY_ERROR => {
            let envelope: Value = serde_json::from_slice(&reply[1..]).unwrap();
            error(json!({"failure": envelope["failure"]}))
        }
        _ => reply,
    }
}

fn values(value: &Value) -> Vec<u8> {
    let mut encoder = Encoder::new(vec![REPLY_VALUES]);
    encoder.json(value);
    encoder.out
}

fn error(envelope: Value) -> Vec<u8> {
    let mut reply = vec![REPLY_ERROR];
    reply.extend_from_slice(envelope.to_string().as_bytes());
    reply
}

#[derive(Debug, PartialEq)]
enum Reply {
    Undefined,
    Json(Value),
    Error(Value),
    Handle(u64),
}

fn reply(handle: u64) -> Reply {
    let bytes = buffers::take(handle).expect("a reply");
    match bytes[0] {
        REPLY_UNDEFINED => Reply::Undefined,
        REPLY_JSON => Reply::Json(serde_json::from_slice(&bytes[1..]).unwrap()),
        REPLY_ERROR => Reply::Error(serde_json::from_slice(&bytes[1..]).unwrap()),
        REPLY_HANDLE => Reply::Handle(u64::from_le_bytes(bytes[1..].try_into().unwrap())),
        status => panic!("unexpected status {status}"),
    }
}

fn buffer(bytes: &[u8]) -> u64 {
    buffers::insert(bytes.to_vec())
}

/// Makes bindings: a reply with their handle, or the error.
fn make(functions: Option<&str>, variables: Option<&Value>) -> Reply {
    let functions = functions.map_or(0, |text| buffer(text.as_bytes()));
    let variables = variables.map_or(0, |value| {
        let mut encoder = Encoder::new(Vec::new());
        encoder.json(value);
        buffers::insert(encoder.out)
    });
    let made = reply(jlens_bindings_new(
        functions,
        variables,
        Some(function),
        CONTEXT,
    ));
    for buffer in [functions, variables] {
        if buffer != 0 {
            assert_eq!(jlens_buffer_free(buffer), 1);
        }
    }
    made
}

fn bindings() -> u64 {
    match make(
        Some(FUNCTIONS),
        Some(&json!({"rate": 0.5, "add": "replaced"})),
    ) {
        Reply::Handle(handle) => handle,
        other => panic!("not bindings: {other:?}"),
    }
}

fn evaluate(source: &str, bindings: u64, options: u64) -> Reply {
    let text = buffer(source.as_bytes());
    let expression = match reply(jlens_compile(text)) {
        Reply::Handle(handle) => handle,
        other => panic!("{source}: {other:?}"),
    };
    assert_eq!(jlens_buffer_free(text), 1);
    let result = reply(jlens_evaluate(expression, 0, bindings, options, 0));
    assert_eq!(jlens_expression_free(expression), 1);
    result
}

fn ok(source: &str, bindings: u64) -> Value {
    match evaluate(source, bindings, 0) {
        Reply::Json(value) => value,
        other => panic!("{source}: {other:?}"),
    }
}

fn failed(source: &str, bindings: u64) -> Value {
    match evaluate(source, bindings, 0) {
        Reply::Error(error) => error,
        other => panic!("{source}: {other:?}"),
    }
}

#[test]
fn bindings_hold_variables_and_host_functions() {
    let bindings = bindings();
    assert_eq!(ok("$add(1, $rate)", bindings), json!(1.5));
    // The function was bound after the variable of the same name, and replaced it.
    assert_eq!(ok("$type($add)", bindings), json!("function"));
    // The arity decides what `$map` passes: the index, to a function of two parameters.
    assert_eq!(ok("$map([1, 2], $add)", bindings), json!([1, 3]));
    assert_eq!(ok("$add(?, 1)(41)", bindings), json!(42));
    assert_eq!(
        ok(
            "$args(1, 'a', missing, {'k': [1]}, function($x, $y) { 0 }, $sum)",
            bindings
        ),
        json!([1, "a", {"undefined": true}, {"k": [1]}, {"function": 2}, {"function": 1}])
    );
    assert_eq!(evaluate("$args()[0]", bindings, 0), Reply::Undefined);
    let variables_only = match make(None, Some(&json!({"x": {"y": [1, 2]}}))) {
        Reply::Handle(handle) => handle,
        other => panic!("{other:?}"),
    };
    assert_eq!(ok("$x.y", variables_only), json!([1, 2]));
    assert_eq!(jlens_bindings_free(variables_only), 1);
    assert_eq!(jlens_bindings_free(bindings), 1);
    assert_eq!(jlens_bindings_free(bindings), 0);
    let freed = failed("$rate", bindings);
    assert_eq!(
        (&freed["code"], &freed["token"]),
        (&json!("U2002"), &json!("bindings"))
    );
}

#[test]
fn host_functions_call_the_functions_they_receive() {
    let bindings = bindings();
    assert_eq!(
        ok("$apply(function($x) { $x * 2 }, 21)", bindings),
        json!(42)
    );
    assert_eq!(ok("$apply($uppercase, 'a')", bindings), json!("A"));
    assert_eq!(ok("$apply($substring(?, 1), 'abc')", bindings), json!("bc"));
    assert_eq!(
        ok("($n := 5; $apply(function() { $n + 1 }))", bindings),
        json!(6)
    );
    // Called with no argument, a function gets the context `null`.
    assert_eq!(ok("$pass($string)", bindings), json!("null"));
    assert_eq!(ok("$apply($add(?, 1), 2)", bindings), json!(3));
    // Undefined goes through as undefined.
    assert_eq!(
        ok("$apply(function($x) { $exists($x) }, missing)", bindings),
        json!(false)
    );
    // The signature is checked, as for a built-in function.
    let typed = failed("$apply(1)", bindings);
    assert_eq!(
        (&typed["code"], &typed["token"]),
        (&json!("T0410"), &json!("apply"))
    );
    // A recursion through a host function ends with D1011.
    let shallow = {
        let text = buffer(br#"{"maxDepth": 50}"#);
        let made = reply(jlens_options_new(text));
        assert_eq!(jlens_buffer_free(text), 1);
        match made {
            Reply::Handle(handle) => handle,
            other => panic!("{other:?}"),
        }
    };
    let deep = evaluate(
        "($f := function($n) { $apply($f, $n + 1) }; $f(0))",
        bindings,
        shallow,
    );
    assert!(matches!(deep, Reply::Error(e) if e["code"] == "D1011"));
    assert_eq!(jlens_options_free(shallow), 1);
    assert_eq!(jlens_bindings_free(bindings), 1);
}

#[test]
fn errors_of_host_functions_keep_their_exception_and_position() {
    let bindings = bindings();
    let error = failed("1 + $fail()", bindings);
    assert_eq!(
        (&error["code"], &error["message"], &error["hostException"]),
        (
            &json!("Boom"),
            &json!("The rate service is down"),
            &json!(3)
        )
    );
    assert_eq!(
        (&error["position"], &error["token"]),
        (&json!(10), &json!("fail"))
    );
    // An error of a function a host function called, let through, is that error, with its
    // position inside the lambda.
    let through = failed("$pass(function() { 1 + 'a' })", bindings);
    assert_eq!(
        (&through["code"], &through["position"]),
        (&json!("T2002"), &json!(22))
    );
    assert_eq!(through.get("hostException"), None);
    let nested = failed("$pass(function() { $fail() })", bindings);
    assert_eq!(
        (&nested["code"], &nested["hostException"]),
        (&json!("Boom"), &json!(3))
    );
    assert_eq!(nested["position"], json!(24));
    // `$eval` wraps it, as any error.
    assert_eq!(failed("$eval('$fail()')", bindings)["code"], json!("D3121"));
    assert_eq!(jlens_bindings_free(bindings), 1);
}

#[test]
fn a_function_called_after_its_host_function_returned_fails_with_u1005() {
    let bindings = bindings();
    assert_eq!(ok("$keep(function() { 1 })", bindings), json!(1));
    let kept = KEPT.get();
    assert_ne!(kept, 0);
    let late = reply(jlens_call(kept, 0, 0));
    assert!(matches!(late, Reply::Error(e) if e["code"] == "U1005"));
    // From another thread, the frame is not there either.
    let other = std::thread::spawn(move || buffers::take(jlens_call(kept, 0, 0)).unwrap());
    assert_eq!(other.join().unwrap()[0], REPLY_ERROR);
    assert_eq!(jlens_bindings_free(bindings), 1);
}

#[test]
fn bindings_not_in_their_form_fail_cleanly() {
    let code = |made: Reply| match made {
        Reply::Error(error) => (error["code"].clone(), error["token"].clone()),
        other => panic!("{other:?}"),
    };
    let bad_signature = make(Some(r#"[{"name": "f", "signature": "<n<s>:n>"}]"#), None);
    assert!(code(bad_signature).0.as_str().unwrap().starts_with("S04"));
    let not_json = make(Some("[{"), None);
    assert_eq!(code(not_json), (json!("U2005"), json!("functions")));
    let unknown_field = make(Some(r#"[{"name": "f", "arty": 1}]"#), None);
    assert_eq!(code(unknown_field), (json!("U2005"), json!("functions")));
    let not_an_object = make(None, Some(&json!([1])));
    assert_eq!(code(not_an_object), (json!("U2005"), json!("variables")));
    let garbage = buffer(&[OBJECT_TAG, 9]);
    let cut = reply(jlens_bindings_new(0, garbage, None, 0));
    assert_eq!(code(cut), (json!("U2005"), json!("values")));
    assert_eq!(jlens_buffer_free(garbage), 1);
    let functions = buffer(br#"[{"name": "f"}]"#);
    let no_function = reply(jlens_bindings_new(functions, 0, None, 0));
    assert_eq!(code(no_function), (json!("U2005"), json!("functions")));
    assert_eq!(jlens_buffer_free(functions), 1);
    let unknown = reply(jlens_bindings_new(u64::MAX, 0, Some(function), 0));
    assert_eq!(code(unknown), (json!("U2002"), json!("buffer")));
    // A variable nested too deep to keep.
    let mut deep = Vec::new();
    for _ in 0..1_001 {
        deep.extend_from_slice(&[ARRAY, 1, 0, 0, 0]);
    }
    deep.push(0);
    let mut object = vec![OBJECT_TAG, 1, 0, 0, 0];
    object.extend(deep);
    object.extend_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0, b'x']);
    let variables = buffer(&object);
    let too_deep = reply(jlens_bindings_new(0, variables, None, 0));
    assert_eq!(code(too_deep).0, json!("U1001"));
    assert_eq!(jlens_buffer_free(variables), 1);
}

const OBJECT_TAG: u8 = jlens_capi::values::OBJECT;

#[test]
fn bindings_serve_every_thread() {
    let bindings = bindings();
    let threads: Vec<_> = (0..4)
        .map(|n| std::thread::spawn(move || ok(&format!("$apply($add(?, {n}), 1)"), bindings)))
        .collect();
    let results: Vec<Value> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(results, [json!(1), json!(2), json!(3), json!(4)]);
    assert_eq!(jlens_bindings_free(bindings), 1);
}
