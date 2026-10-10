//! Property tests: no text, edit or position makes the server fail or panic.

use std::time::Duration;

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use proptest::prelude::*;
use serde_json::{Value, json};

#[rustfmt::skip]
const TOKENS: &[&str] = &[
    "customer", "items", "type", "$", "$$", "$x", "1", "'s'", "\"t\"", ".", "[", "]", "(", ")",
    "{", "}", ":", ",", ";", "=", "and", "~>", "$sum(", "$substring(", "function($v){", "/* c */",
    "/** @label \"x\" */", "`", "é", "𝄞", " ", "\n", "\r\n",
];

fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(TOKENS), 0..16).prop_map(|t| t.concat())
}

fn position() -> impl Strategy<Value = Value> {
    (0u32..6, 0u32..40).prop_map(|(line, character)| json!({"line": line, "character": character}))
}

#[derive(Debug, Clone)]
enum Step {
    Change(Value, Value, String),
    Replace(String),
    Request(&'static str, Value),
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        3 => (position(), position(), text()).prop_map(|(a, b, t)| Step::Change(a, b, t)),
        1 => text().prop_map(Step::Replace),
        4 => (
            prop::sample::select(&["textDocument/completion", "textDocument/hover", "textDocument/signatureHelp"][..]),
            position(),
        )
            .prop_map(|(m, p)| Step::Request(m, p)),
    ]
}

/// Runs the steps against a fresh server and returns the error responses it sent.
fn run(start: &str, steps: &[Step], utf8: bool) -> Vec<String> {
    let (server, client) = Connection::memory();
    let handle = std::thread::spawn(move || jlens_lsp::run(&server));
    let send = |m: Message| client.sender.send(m).unwrap();
    let encodings = if utf8 {
        json!(["utf-8"])
    } else {
        json!(["utf-16"])
    };
    send(
        Request::new(
            RequestId::from(0),
            "initialize".into(),
            json!({"capabilities": {"general": {"positionEncodings": encodings}}}),
        )
        .into(),
    );
    send(Notification::new("initialized".into(), json!({})).into());
    let uri = "untitled:rule";
    send(Notification::new(
        "textDocument/didOpen".into(),
        json!({"textDocument": {"uri": uri, "languageId": "jsonata", "version": 1, "text": start}}),
    )
    .into());
    let mut requests = 0;
    for (version, step) in steps.iter().enumerate() {
        let version = version as i32 + 2;
        match step {
            Step::Change(a, b, t) => send(Notification::new(
                "textDocument/didChange".into(),
                json!({"textDocument": {"uri": uri, "version": version},
                       "contentChanges": [{"range": {"start": a, "end": b}, "text": t}]}),
            )
            .into()),
            Step::Replace(t) => send(Notification::new(
                "textDocument/didChange".into(),
                json!({"textDocument": {"uri": uri, "version": version}, "contentChanges": [{"text": t}]}),
            )
            .into()),
            Step::Request(method, position) => {
                requests += 1;
                send(Request::new(
                    RequestId::from(requests),
                    (*method).into(),
                    json!({"textDocument": {"uri": uri}, "position": position}),
                )
                .into());
            }
        }
    }
    send(Request::new(RequestId::from(10_000), "shutdown".into(), Value::Null).into());
    let mut errors = Vec::new();
    loop {
        match client
            .receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("the server answers")
        {
            Message::Response(Response { id, .. }) if id == RequestId::from(10_000) => break,
            Message::Response(Response {
                response_result: Err(e),
                ..
            }) => errors.push(e.message),
            _ => {}
        }
    }
    send(Notification::new("exit".into(), Value::Null).into());
    let result = handle.join().expect("the server does not panic");
    assert!(result.is_ok(), "{result:?}");
    errors
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn the_server_survives_any_session(start in text(), steps in prop::collection::vec(step(), 0..12), utf8 in any::<bool>()) {
        let errors = run(&start, &steps, utf8);
        prop_assert!(errors.is_empty(), "{errors:?}");
    }
}
