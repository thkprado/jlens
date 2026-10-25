//! Property tests over the exports: random sequences of calls, with handles that are live,
//! freed, freed twice or never made, and bytes that are not UTF-8 or not JSON. Every call
//! returns an error or a result, never a panic (which the exports would turn into `U2004`).

use jlens_capi::exports::*;
use jlens_capi::{REPLY_ERROR, REPLY_HANDLE, REPLY_JSON, REPLY_UNDEFINED, REPLY_VALUES, buffers};
use proptest::prelude::*;
use serde_json::Value;

mod host;
mod values;

/// Expressions, inputs, options and reads that are valid, nearly valid, or bytes of any kind.
fn bytes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        prop::sample::select(vec![
            "a",
            "a.b",
            "$sum(items.price)",
            "items[0].name",
            "$count($)",
            "$string($)",
            "$random()",
            "{\"a\": 1, \"items\": [{\"price\": 2, \"name\": \"x\"}]}",
            "[1, 2, 3]",
            "\"é😀\"",
            "{\"a\": ",
            "(",
            "$x(",
            "1e400",
            "{\"maxSteps\": 3}",
            "{\"timeout\": 1000, \"seed\": 7, \"now\": 0}",
            "{\"maxDepth\": 2, \"maxResultSize\": 4}",
            "{\"timeout\": -1}",
            "{\"maxSteps\": null}",
            "true",
            "{\"a\": true, \"items\": {\"price\": true}}",
            "{\"a\": {}}",
            "[{\"input\": true}]",
            "[{\"input\": {\"a\": true}}, {\"bindings\": {}}]",
            "[{\"input\": 1}]",
        ])
        .prop_map(|s| s.as_bytes().to_vec()),
        "[ -~]{0,24}".prop_map(String::into_bytes),
        prop::collection::vec(any::<u8>(), 0..24),
    ]
}

#[derive(Debug, Clone)]
enum Call {
    /// A buffer holding these bytes.
    Buffer(Vec<u8>),
    /// A zeroed buffer of this length, through `jlens_buffer_new`.
    Zeroed(u8),
    /// Compiles one of the buffers made (by index, modulo their number).
    Compile(usize),
    /// Makes options from one of the buffers.
    Options(usize),
    /// Makes a document from one of the buffers, pruned by another or not.
    Document(usize, Option<usize>),
    /// Evaluates one of the expressions against one of the buffers, or no input, with one of
    /// the options made, or none.
    Evaluate(usize, Option<usize>, Option<usize>),
    /// Evaluates one of the expressions against one of the documents.
    EvaluateDocument(usize, usize, Option<usize>),
    /// What one of the expressions reads.
    Reads(usize),
    /// The union of the reads in one of the buffers.
    UnionReads(usize),
    FreeBuffer(usize),
    FreeExpression(usize),
    FreeOptions(usize),
    FreeDocument(usize),
    /// A handle that was never made, passed where any handle goes.
    Stranger(u32, u32, u8),
}

fn call() -> impl Strategy<Value = Call> {
    prop_oneof![
        3 => bytes().prop_map(Call::Buffer),
        1 => any::<u8>().prop_map(Call::Zeroed),
        3 => any::<usize>().prop_map(Call::Compile),
        2 => any::<usize>().prop_map(Call::Options),
        2 => (any::<usize>(), any::<Option<usize>>()).prop_map(|(t, r)| Call::Document(t, r)),
        4 => (any::<usize>(), any::<Option<usize>>(), any::<Option<usize>>())
            .prop_map(|(e, i, o)| Call::Evaluate(e, i, o)),
        3 => (any::<usize>(), any::<usize>(), any::<Option<usize>>())
            .prop_map(|(e, d, o)| Call::EvaluateDocument(e, d, o)),
        1 => any::<usize>().prop_map(Call::Reads),
        1 => any::<usize>().prop_map(Call::UnionReads),
        2 => any::<usize>().prop_map(Call::FreeBuffer),
        2 => any::<usize>().prop_map(Call::FreeExpression),
        1 => any::<usize>().prop_map(Call::FreeOptions),
        1 => any::<usize>().prop_map(Call::FreeDocument),
        1 => (any::<u32>(), any::<u32>(), any::<u8>()).prop_map(|(g, i, w)| Call::Stranger(g, i, w)),
    ]
}

/// A handle made by the test, and whether it is still live.
struct Made {
    handle: u64,
    live: bool,
}

fn pick(made: &[Made], index: usize) -> Option<&Made> {
    (!made.is_empty()).then(|| &made[index % made.len()])
}

/// The handle of `index` among `made`, if any, and whether it is live; 0 and live for none.
fn optional(made: &[Made], index: Option<usize>) -> (u64, bool) {
    index
        .and_then(|i| pick(made, i))
        .map_or((0, true), |m| (m.handle, m.live))
}

/// Frees the handle of `index` among `made` with `free`, which returns 1 only for a live one.
fn free(
    made: &mut [Made],
    index: usize,
    free: extern "C" fn(u64) -> u32,
) -> Result<(), TestCaseError> {
    let len = made.len().max(1);
    if let Some(made) = made.get_mut(index % len) {
        prop_assert_eq!(free(made.handle), u32::from(made.live));
        made.live = false;
    }
    Ok(())
}

/// What a reply holds: an error's code, a handle, or neither.
#[derive(Debug, PartialEq)]
enum Replied {
    Value,
    Handle(u64),
    Error(String),
}

/// Reads and frees a reply, checking its form.
fn reply(handle: u64) -> Result<Replied, TestCaseError> {
    prop_assert_ne!(handle, 0, "a reply is always made");
    let bytes = buffers::take(handle).expect("the reply is live");
    let (status, rest) = bytes.split_first().expect("a status byte");
    match *status {
        REPLY_UNDEFINED => prop_assert!(rest.is_empty()),
        REPLY_JSON => {
            serde_json::from_slice::<Value>(rest).expect("JSON text");
        }
        REPLY_VALUES => {
            values::decode(rest);
        }
        REPLY_HANDLE => {
            let handle = u64::from_le_bytes(rest.try_into().expect("8 bytes"));
            prop_assert_ne!(handle, 0);
            return Ok(Replied::Handle(handle));
        }
        REPLY_ERROR => {
            let error: Value = serde_json::from_slice(rest).expect("an envelope");
            let code = error["code"].as_str().expect("a code").to_owned();
            prop_assert!(error["message"].is_string());
            prop_assert_ne!(&code, "U2004", "a panic: {}", error);
            return Ok(Replied::Error(code));
        }
        status => prop_assert!(false, "unknown status {}", status),
    }
    Ok(Replied::Value)
}

fn unknown() -> Replied {
    Replied::Error("U2002".to_owned())
}

/// A handle no table has made: its generation is beyond any this test reaches.
fn stranger(generation: u32, index: u32) -> u64 {
    (u64::from(generation | 0x8000_0000) << 32) | u64::from(index)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn calls_in_any_order_fail_cleanly(calls in prop::collection::vec(call(), 1..40)) {
        let mut buffers_made: Vec<Made> = Vec::new();
        let mut expressions: Vec<Made> = Vec::new();
        let mut options: Vec<Made> = Vec::new();
        let mut documents: Vec<Made> = Vec::new();
        for call in calls {
            match call {
                Call::Buffer(bytes) => {
                    let handle = buffers::insert(bytes);
                    prop_assert_ne!(handle, 0);
                    buffers_made.push(Made { handle, live: true });
                }
                Call::Zeroed(len) => {
                    let handle = jlens_buffer_new(len.into());
                    prop_assert_ne!(handle, 0);
                    prop_assert_eq!(jlens_buffer_len(handle), usize::from(len));
                    prop_assert_ne!(jlens_buffer_address(handle), 0);
                    buffers_made.push(Made { handle, live: true });
                }
                Call::Compile(index) => {
                    let Some(source) = pick(&buffers_made, index) else { continue };
                    let (handle, live) = (source.handle, source.live);
                    let utf8 = buffers::with(handle, |b| std::str::from_utf8(b).is_ok());
                    match (reply(jlens_compile(handle))?, live, utf8) {
                        (replied, false, _) => prop_assert_eq!(replied, unknown()),
                        (replied, true, Some(false)) => {
                            prop_assert_eq!(replied, Replied::Error("U2003".to_owned()));
                        }
                        (Replied::Handle(handle), ..) => {
                            expressions.push(Made { handle, live: true });
                        }
                        (Replied::Error(code), ..) => prop_assert!(code.starts_with('S')),
                        (Replied::Value, ..) => prop_assert!(false, "a value from compile"),
                    }
                }
                Call::Options(index) => {
                    let Some(source) = pick(&buffers_made, index) else { continue };
                    match (reply(jlens_options_new(source.handle))?, source.live) {
                        (replied, false) => prop_assert_eq!(replied, unknown()),
                        (Replied::Handle(handle), true) => options.push(Made { handle, live: true }),
                        (replied, true) => {
                            prop_assert_eq!(replied, Replied::Error("U2005".to_owned()));
                        }
                    }
                }
                Call::Document(text, read) => {
                    let Some(text) = pick(&buffers_made, text) else { continue };
                    let (read, read_live) = optional(&buffers_made, read);
                    match (reply(jlens_document_new(text.handle, read))?, text.live && read_live) {
                        (replied, false) => prop_assert_eq!(replied, unknown()),
                        (Replied::Handle(handle), true) => documents.push(Made { handle, live: true }),
                        (Replied::Error(code), true) => {
                            prop_assert!(code == "U2001" || code == "U2005", "{}", code);
                        }
                        (Replied::Value, true) => prop_assert!(false, "a value from a document"),
                    }
                }
                Call::Evaluate(expression, input, given) => {
                    let Some(expression) = pick(&expressions, expression) else { continue };
                    let (input, input_live) = optional(&buffers_made, input);
                    let (given, given_live) = optional(&options, given);
                    let replied = reply(jlens_evaluate(expression.handle, input, given, 0))?;
                    if !expression.live || !input_live || !given_live {
                        prop_assert_eq!(replied, unknown());
                    }
                }
                Call::EvaluateDocument(expression, document, given) => {
                    let Some(expression) = pick(&expressions, expression) else { continue };
                    let Some(document) = pick(&documents, document) else { continue };
                    let (given, given_live) = optional(&options, given);
                    let replied =
                        reply(jlens_evaluate_document(expression.handle, document.handle, given, 0))?;
                    if !expression.live || !document.live || !given_live {
                        prop_assert_eq!(replied, unknown());
                    } else {
                        prop_assert!(!matches!(replied, Replied::Handle(_)));
                    }
                }
                Call::Reads(index) => {
                    let Some(expression) = pick(&expressions, index) else { continue };
                    let replied = reply(jlens_reads(expression.handle))?;
                    match expression.live {
                        true => prop_assert_eq!(replied, Replied::Value),
                        false => prop_assert_eq!(replied, unknown()),
                    }
                }
                Call::UnionReads(index) => {
                    let Some(reads) = pick(&buffers_made, index) else { continue };
                    match (reply(jlens_union_reads(reads.handle))?, reads.live) {
                        (replied, false) => prop_assert_eq!(replied, unknown()),
                        (Replied::Value, true) => {}
                        (replied, true) => {
                            prop_assert_eq!(replied, Replied::Error("U2005".to_owned()));
                        }
                    }
                }
                Call::FreeBuffer(index) => {
                    free(&mut buffers_made, index, jlens_buffer_free)?;
                    if let Some(made) = pick(&buffers_made, index) {
                        prop_assert_eq!(jlens_buffer_len(made.handle), 0);
                    }
                }
                Call::FreeExpression(index) => free(&mut expressions, index, jlens_expression_free)?,
                Call::FreeOptions(index) => free(&mut options, index, jlens_options_free)?,
                Call::FreeDocument(index) => free(&mut documents, index, jlens_document_free)?,
                Call::Stranger(generation, index, which) => {
                    let handle = stranger(generation, index);
                    let some = pick(&buffers_made, index as usize).map_or(0, |m| m.handle);
                    let expression = pick(&expressions, index as usize).map(|m| m.handle);
                    let document = pick(&documents, index as usize).map(|m| m.handle);
                    match which % 16 {
                        0 => prop_assert_eq!(jlens_buffer_free(handle), 0),
                        1 => prop_assert_eq!(jlens_buffer_address(handle), 0),
                        2 => prop_assert_eq!(jlens_buffer_len(handle), 0),
                        3 => prop_assert_eq!(jlens_expression_free(handle), 0),
                        4 => prop_assert_eq!(jlens_options_free(handle), 0),
                        5 => prop_assert_eq!(jlens_document_free(handle), 0),
                        6 => prop_assert_eq!(reply(jlens_compile(handle))?, unknown()),
                        7 => prop_assert_eq!(reply(jlens_evaluate(handle, some, 0, 0))?, unknown()),
                        8 => prop_assert_eq!(reply(jlens_options_new(handle))?, unknown()),
                        9 => prop_assert_eq!(reply(jlens_document_new(handle, 0))?, unknown()),
                        10 => prop_assert_eq!(reply(jlens_document_new(some, handle))?, unknown()),
                        11 => prop_assert_eq!(reply(jlens_reads(handle))?, unknown()),
                        12 => prop_assert_eq!(reply(jlens_union_reads(handle))?, unknown()),
                        13 => {
                            let Some(document) = document else { continue };
                            let replied = reply(jlens_evaluate_document(handle, document, 0, 0))?;
                            prop_assert_eq!(replied, unknown());
                        }
                        14 => {
                            let Some(expression) = expression else { continue };
                            prop_assert_eq!(reply(jlens_evaluate(expression, handle, 0, 0))?, unknown());
                            let replied = reply(jlens_evaluate_document(expression, handle, 0, 0))?;
                            prop_assert_eq!(replied, unknown());
                        }
                        _ => {
                            let Some(expression) = expression else { continue };
                            prop_assert_eq!(reply(jlens_evaluate(expression, 0, handle, 0))?, unknown());
                        }
                    }
                }
            }
        }
        for made in buffers_made.iter().filter(|m| m.live) {
            prop_assert_eq!(jlens_buffer_free(made.handle), 1);
        }
        for made in expressions.iter().filter(|m| m.live) {
            prop_assert_eq!(jlens_expression_free(made.handle), 1);
        }
        for made in options.iter().filter(|m| m.live) {
            prop_assert_eq!(jlens_options_free(made.handle), 1);
        }
        for made in documents.iter().filter(|m| m.live) {
            prop_assert_eq!(jlens_document_free(made.handle), 1);
        }
        prop_assert_eq!(jlens_capi::documents::cached(), 0, "every conversion dropped");
    }

    #[test]
    fn handles_of_one_kind_passed_as_another_fail_cleanly(source in bytes(), input in bytes()) {
        // A handle of one kind may name a live object of another kind, made by another test:
        // the tables are separate. Whatever it names, the call does not panic.
        let source = buffers::insert(source);
        let input = buffers::insert(input);
        reply(jlens_evaluate(source, input, input, 0))?;
        reply(jlens_evaluate(source, input, 0, 1))?;
        reply(jlens_evaluate(source, input, 0, u32::MAX))?;
        reply(jlens_evaluate(input, 0, source, 0))?;
        reply(jlens_evaluate_document(source, input, 0, 0))?;
        reply(jlens_reads(source))?;
        if let Replied::Handle(options) = reply(jlens_options_new(input))? {
            reply(jlens_evaluate(source, input, options, 0))?;
            reply(jlens_document_new(options, options))?;
            prop_assert_eq!(jlens_options_free(options), 1);
        }
        if let Replied::Handle(document) = reply(jlens_document_new(input, 0))? {
            reply(jlens_evaluate(document, document, document, 0))?;
            reply(jlens_union_reads(document))?;
            prop_assert_eq!(jlens_document_free(document), 1);
        }
        if let Replied::Handle(expression) = reply(jlens_compile(source))? {
            reply(jlens_compile(expression))?;
            reply(jlens_evaluate(expression, expression, expression, 0))?;
            reply(jlens_evaluate(expression, 0, 0, 1))?;
            reply(jlens_evaluate_document(expression, expression, expression, 0))?;
            prop_assert_eq!(jlens_expression_free(expression), 1);
        }
        prop_assert_eq!(jlens_buffer_free(source), 1);
        prop_assert_eq!(jlens_buffer_free(input), 1);
    }

    #[test]
    fn values_and_json_text_give_the_same_result(input in json(), expression in prop::sample::select(vec![
        "$", "$.*", "**", "$string($)", "$keys($)", "[$]", "{'v': $}", "$ ~> | $ | {'x': 1} |",
    ])) {
        let text = input.to_string();
        let expression = {
            let source = buffers::insert(expression.as_bytes().to_vec());
            let Replied::Handle(handle) = reply(jlens_compile(source))? else {
                return Err(TestCaseError::fail("compiles"));
            };
            prop_assert_eq!(jlens_buffer_free(source), 1);
            handle
        };
        let buffer = buffers::insert(text.into_bytes());
        let as_json = buffers::take(jlens_evaluate(expression, buffer, 0, 0)).unwrap();
        let as_values = buffers::take(jlens_evaluate(expression, buffer, 0, 1)).unwrap();
        match (as_json[0], as_values[0]) {
            (REPLY_JSON, REPLY_VALUES) => {
                let json: Value = serde_json::from_slice(&as_json[1..]).unwrap();
                prop_assert_eq!(values::as_f64(values::decode(&as_values[1..])), values::as_f64(json));
            }
            (json, values) => prop_assert_eq!(json, values, "the same kind of reply"),
        }
        prop_assert_eq!(jlens_buffer_free(buffer), 1);
        prop_assert_eq!(jlens_expression_free(expression), 1);
    }
}

/// JSON values of every kind: numbers integral or not, small and large, strings beyond ASCII,
/// keys repeated across objects, nested arrays and objects.
fn json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        any::<f64>()
            .prop_filter("finite", |f| f.is_finite())
            .prop_map(Value::from),
        prop::sample::select(vec![
            0.5,
            -0.0,
            1e20,
            1e21,
            2f64.powi(63) - 1024.0,
            2f64.powi(63),
            -2f64.powi(63)
        ])
        .prop_map(Value::from),
        r"[a-c]{0,3}|\PC{0,4}".prop_map(Value::String),
    ];
    leaf.prop_recursive(4, 48, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::vec((r"[a-d]|\PC{1,3}", inner), 0..6)
                .prop_map(|fields| Value::Object(fields.into_iter().collect())),
        ]
    })
}

/// Evaluates `expression` against `input` as the host's own values, read in mode `context`:
/// the reply's bytes, and the host's nodes.
fn on_host(expression: u64, input: &Value, context: u64, result: u32) -> (Vec<u8>, Vec<Value>) {
    let level = host::Host::start(input);
    let reply = jlens_evaluate_source(expression, level, Some(host::READER), context, 0, result);
    host::Host::finish();
    assert_eq!(jlens_buffer_free(level), 1);
    let nodes = host::HOST.with_borrow_mut(|host| std::mem::take(&mut host.nodes));
    (buffers::take(reply).expect("a reply"), nodes)
}

fn compiled(source: &str) -> u64 {
    let buffer = buffers::insert(source.as_bytes().to_vec());
    let reply = buffers::take(jlens_compile(buffer)).unwrap();
    assert_eq!(jlens_buffer_free(buffer), 1);
    assert_eq!(reply[0], REPLY_HANDLE, "{source}");
    u64::from_le_bytes(reply[1..].try_into().unwrap())
}

/// The expressions of the tests over the host's values, and the keys they name, which the
/// test's reader must know to find a field ([`host`]).
const EXPRESSIONS: &[&str] = &[
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

proptest! {
    /// The host's values give what their JSON text gives, as JSON text or as values, whether
    /// the reader reads one field or item alone or not.
    #[test]
    fn the_hosts_values_give_what_their_json_text_gives(
        input in json(),
        expression in prop::sample::select(EXPRESSIONS),
    ) {
        host::Host::know(&["a", "b", "x", "v"]);
        let expression = compiled(expression);
        let text = buffers::insert(input.to_string().into_bytes());
        let as_text = buffers::take(jlens_evaluate(expression, text, 0, 0)).unwrap();
        prop_assert_eq!(jlens_buffer_free(text), 1);
        for context in [host::PLAIN, host::WHOLE] {
            let (as_json, _) = on_host(expression, &input, context, 0);
            prop_assert_eq!(&as_json, &as_text);
            let (as_values, nodes) = on_host(expression, &input, context, 1);
            match (as_text[0], as_values[0]) {
                (REPLY_JSON, REPLY_VALUES) => {
                    let json: Value = serde_json::from_slice(&as_text[1..]).unwrap();
                    let values = values::decode_with(&as_values[1..], &nodes);
                    prop_assert_eq!(values::as_f64(values), values::as_f64(json));
                }
                (text, values) => prop_assert_eq!(text, values, "the same kind of reply"),
            }
        }
        prop_assert_eq!(jlens_expression_free(expression), 1);
    }

    /// A reader that returns any bytes, levels nearly in their form included, makes the
    /// evaluation fail with `U1004` or give a result, never panic.
    #[test]
    fn a_reader_returning_any_bytes_fails_cleanly(
        garbage in prop_oneof![
            bytes(),
            (any::<u8>(), any::<u32>(), bytes()).prop_map(|(kind, count, rest)| {
                let mut level = Vec::new();
                level.extend_from_slice(&((5 + rest.len()) as u32).to_le_bytes());
                level.push(kind % 5);
                level.extend_from_slice(&(count % 4).to_le_bytes());
                level.extend(rest);
                level
            }),
        ],
        expression in prop::sample::select(EXPRESSIONS),
        result in 0u32..2,
    ) {
        host::HOST.with_borrow_mut(|host| host.garbage = garbage);
        let expression = compiled(expression);
        let input = serde_json::json!({"a": [{"b": 1}, {"b": [2]}]});
        let (reply, nodes) = on_host(expression, &input, host::GARBAGE, result);
        match reply[0] {
            REPLY_ERROR => {
                let error: Value = serde_json::from_slice(&reply[1..]).expect("an envelope");
                prop_assert_ne!(&error["code"], "U2004", "a panic: {}", error);
            }
            REPLY_VALUES => {
                values::decode_with(&reply[1..], &nodes);
            }
            REPLY_JSON => {
                serde_json::from_slice::<Value>(&reply[1..]).expect("JSON text");
            }
            status => prop_assert_eq!(status, REPLY_UNDEFINED),
        }
        prop_assert_eq!(jlens_expression_free(expression), 1);
    }
}
