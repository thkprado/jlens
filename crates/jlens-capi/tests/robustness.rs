//! Property tests over the exports: random sequences of calls, with handles that are live,
//! freed, freed twice or never made, and bytes that are not UTF-8 or not JSON. Every call
//! returns an error or a result, never a panic (which the exports would turn into `U2004`).

use jlens_capi::exports::*;
use jlens_capi::{REPLY_ERROR, REPLY_HANDLE, REPLY_JSON, REPLY_UNDEFINED, buffers};
use proptest::prelude::*;
use serde_json::Value;

/// Expressions and inputs that are valid, nearly valid, or bytes of any kind.
fn bytes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        prop::sample::select(vec![
            "a",
            "a.b",
            "$sum(items.price)",
            "items[0].name",
            "$count($)",
            "$string($)",
            "{\"a\": 1, \"items\": [{\"price\": 2, \"name\": \"x\"}]}",
            "[1, 2, 3]",
            "\"é😀\"",
            "{\"a\": ",
            "(",
            "$x(",
            "1e400",
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
    /// Evaluates one of the expressions made against one of the buffers, or no input.
    Evaluate(usize, Option<usize>),
    FreeBuffer(usize),
    FreeExpression(usize),
    /// A handle that was never made, passed where any handle goes.
    Stranger(u32, u32, u8),
}

fn call() -> impl Strategy<Value = Call> {
    prop_oneof![
        3 => bytes().prop_map(Call::Buffer),
        1 => any::<u8>().prop_map(Call::Zeroed),
        3 => any::<usize>().prop_map(Call::Compile),
        4 => (any::<usize>(), any::<Option<usize>>()).prop_map(|(e, i)| Call::Evaluate(e, i)),
        2 => any::<usize>().prop_map(Call::FreeBuffer),
        2 => any::<usize>().prop_map(Call::FreeExpression),
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

/// Reads and frees a reply, and returns the code of its error, if any.
fn reply(handle: u64) -> Result<Option<String>, TestCaseError> {
    prop_assert_ne!(handle, 0, "a reply is always made");
    let bytes = buffers::take(handle).expect("the reply is live");
    let (status, rest) = bytes.split_first().expect("a status byte");
    match *status {
        REPLY_UNDEFINED => prop_assert!(rest.is_empty()),
        REPLY_JSON => {
            serde_json::from_slice::<Value>(rest).expect("JSON text");
        }
        REPLY_HANDLE => prop_assert_eq!(rest.len(), 8),
        REPLY_ERROR => {
            let error: Value = serde_json::from_slice(rest).expect("an envelope");
            let code = error["code"].as_str().expect("a code").to_owned();
            prop_assert!(error["message"].is_string());
            prop_assert_ne!(&code, "U2004", "a panic: {}", error);
            return Ok(Some(code));
        }
        status => prop_assert!(false, "unknown status {}", status),
    }
    Ok(None)
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
                    let reply_handle = jlens_compile(handle);
                    let bytes = buffers::with(reply_handle, |b| b.clone()).unwrap();
                    let code = reply(reply_handle)?;
                    match (live, utf8) {
                        (false, _) => prop_assert_eq!(code.as_deref(), Some("U2002")),
                        (true, Some(false)) => prop_assert_eq!(code.as_deref(), Some("U2003")),
                        _ => {
                            if bytes[0] == REPLY_HANDLE {
                                let expression = u64::from_le_bytes(bytes[1..].try_into().unwrap());
                                expressions.push(Made { handle: expression, live: true });
                            } else {
                                prop_assert_eq!(bytes[0], REPLY_ERROR);
                                prop_assert!(code.unwrap().starts_with('S'));
                            }
                        }
                    }
                }
                Call::Evaluate(expression, input) => {
                    let Some(expression) = pick(&expressions, expression) else { continue };
                    let input = input.and_then(|i| pick(&buffers_made, i));
                    let code = reply(jlens_evaluate(expression.handle, input.map_or(0, |i| i.handle)))?;
                    if !expression.live || input.is_some_and(|i| !i.live) {
                        prop_assert_eq!(code.as_deref(), Some("U2002"));
                    }
                }
                Call::FreeBuffer(index) => {
                    let len = buffers_made.len().max(1);
                    let Some(made) = buffers_made.get_mut(index % len) else { continue };
                    prop_assert_eq!(jlens_buffer_free(made.handle), u32::from(made.live));
                    prop_assert_eq!(jlens_buffer_len(made.handle), 0);
                    made.live = false;
                }
                Call::FreeExpression(index) => {
                    let len = expressions.len().max(1);
                    let Some(made) = expressions.get_mut(index % len) else { continue };
                    prop_assert_eq!(jlens_expression_free(made.handle), u32::from(made.live));
                    made.live = false;
                }
                Call::Stranger(generation, index, which) => {
                    let handle = stranger(generation, index);
                    let some = pick(&buffers_made, index as usize).map_or(0, |m| m.handle);
                    match which % 7 {
                        0 => prop_assert_eq!(jlens_buffer_free(handle), 0),
                        1 => prop_assert_eq!(jlens_buffer_address(handle), 0),
                        2 => prop_assert_eq!(jlens_buffer_len(handle), 0),
                        3 => prop_assert_eq!(jlens_expression_free(handle), 0),
                        4 => prop_assert_eq!(reply(jlens_compile(handle))?, Some("U2002".to_owned())),
                        5 => prop_assert_eq!(reply(jlens_evaluate(handle, some))?, Some("U2002".to_owned())),
                        _ => {
                            let Some(expression) = pick(&expressions, index as usize) else { continue };
                            let code = reply(jlens_evaluate(expression.handle, handle))?;
                            prop_assert_eq!(code.as_deref(), Some("U2002"));
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
    }

    #[test]
    fn handles_of_one_kind_passed_as_the_other_fail_cleanly(source in bytes(), input in bytes()) {
        // A buffer's handle may name a live expression of another test, and the other way
        // round: the tables are separate. Whatever it names, the call does not panic.
        let source = buffers::insert(source);
        let input = buffers::insert(input);
        reply(jlens_evaluate(source, input))?;
        reply(jlens_evaluate(input, 0))?;
        let compiled = jlens_compile(source);
        let bytes = buffers::with(compiled, |b| b.clone()).unwrap();
        reply(compiled)?;
        if bytes[0] == REPLY_HANDLE {
            let expression = u64::from_le_bytes(bytes[1..].try_into().unwrap());
            reply(jlens_compile(expression))?;
            reply(jlens_evaluate(expression, expression))?;
            prop_assert_eq!(jlens_expression_free(expression), 1);
        }
        prop_assert_eq!(jlens_buffer_free(source), 1);
        prop_assert_eq!(jlens_buffer_free(input), 1);
    }
}
