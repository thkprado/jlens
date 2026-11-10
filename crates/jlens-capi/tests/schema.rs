//! Schemas and language services through the exports (ADR 0023), called as the host calls
//! them.

use jlens_capi::exports::*;
use jlens_capi::{REPLY_ERROR, REPLY_HANDLE, REPLY_JSON, buffers};
use serde_json::{Value, json};

#[derive(Debug, PartialEq)]
enum Reply {
    Json(Value),
    Error(Value),
    Handle(u64),
}

/// Reads and frees a reply.
fn reply(handle: u64) -> Reply {
    let bytes = buffers::take(handle).expect("a reply");
    let (status, rest) = bytes.split_first().expect("a status byte");
    match *status {
        REPLY_JSON => Reply::Json(serde_json::from_slice(rest).unwrap()),
        REPLY_ERROR => Reply::Error(serde_json::from_slice(rest).unwrap()),
        REPLY_HANDLE => Reply::Handle(u64::from_le_bytes(rest.try_into().unwrap())),
        status => panic!("unknown status {status}"),
    }
}

/// Calls `export` with a buffer holding `bytes`, and frees the buffer.
fn with_buffer(bytes: &[u8], export: impl FnOnce(u64) -> u64) -> Reply {
    let buffer = buffers::insert(bytes.to_vec());
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

/// The code and the token of an error reply.
fn error(reply: Reply) -> (String, Option<String>) {
    match reply {
        Reply::Error(error) => (
            error["code"].as_str().expect("a code").to_owned(),
            error["token"].as_str().map(str::to_owned),
        ),
        other => panic!("not an error: {other:?}"),
    }
}

fn schema(document: &Value) -> u64 {
    handle(with_buffer(document.to_string().as_bytes(), |buffer| {
        jlens_schema_new(buffer)
    }))
}

fn service(schema: u64, declarations: Option<&Value>) -> Reply {
    match declarations {
        Some(declarations) => with_buffer(declarations.to_string().as_bytes(), |buffer| {
            jlens_service_new(schema, buffer)
        }),
        None => reply(jlens_service_new(schema, 0)),
    }
}

/// The diagnostics of `text`, each as its code and the UTF-16 offsets of its span.
fn diagnostics(service: u64, text: &str) -> Vec<(String, u64, u64)> {
    let Reply::Json(Value::Array(list)) = with_buffer(text.as_bytes(), |buffer| {
        jlens_service_diagnostics(service, buffer)
    }) else {
        panic!("{text}: not an array");
    };
    list.iter()
        .map(|d| {
            assert!(d["message"].is_string(), "{d}");
            assert!(matches!(d["severity"].as_str(), Some("error" | "warning")));
            assert_eq!(d.as_object().map(|d| d.len()), Some(4), "{d}");
            (
                d["code"].as_str().expect("a code").to_owned(),
                d["span"]["start"].as_u64().expect("a start"),
                d["span"]["end"].as_u64().expect("an end"),
            )
        })
        .collect()
}

fn order() -> Value {
    json!({
        "type": "object",
        "properties": {
            "status": {"type": "string", "enum": ["open", "closed"]},
            "total": {"type": "number"},
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {"name": {"type": "string"}, "price": {"type": "number"}}
                }
            }
        }
    })
}

fn codes(found: &[(String, u64, u64)]) -> Vec<&str> {
    found.iter().map(|(code, ..)| code.as_str()).collect()
}

#[test]
fn a_schema_and_a_service_are_shared_by_threads() {
    fn shared<T: Send + Sync>() {}
    shared::<jlens_schema::Schema>();
    shared::<jlens_schema::LanguageService>();

    let schema = schema(&order());
    let service = handle(service(schema, None));
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                for _ in 0..50 {
                    assert_eq!(codes(&diagnostics(service, "items.nme")), ["W0201"]);
                    assert!(diagnostics(service, "$sum(items.price) > total").is_empty());
                }
            });
        }
    });
    assert_eq!(jlens_service_free(service), 1);
    assert_eq!(jlens_schema_free(schema), 1);
}

#[test]
fn diagnostics_are_those_of_the_schema() {
    let schema = schema(&order());
    let service = handle(service(schema, None));
    assert!(diagnostics(service, "items[price > 10].name").is_empty());
    assert_eq!(
        diagnostics(service, "items.nme"),
        [("W0201".to_owned(), 6, 9)]
    );
    assert_eq!(
        codes(&diagnostics(service, "status = \"shipped\"")),
        ["W0206"]
    );
    assert_eq!(codes(&diagnostics(service, "$nope(total)")), ["W0202"]);
    assert_eq!(codes(&diagnostics(service, "$sum()")), ["W0203"]);
    // A syntax error is a diagnostic, not a failure of the call.
    let syntax = diagnostics(service, "items[");
    assert!(
        syntax.iter().any(|(code, ..)| code.starts_with('S')),
        "{syntax:?}"
    );
    assert!(
        diagnostics(service, "")
            .iter()
            .all(|(code, ..)| !code.starts_with('U'))
    );
    assert_eq!(jlens_service_free(service), 1);
    assert_eq!(jlens_schema_free(schema), 1);
}

#[test]
fn spans_count_utf16_code_units() {
    let schema = schema(&order());
    let service = handle(service(schema, None));
    // "é" is two bytes and one unit, "😀" four bytes and two units.
    assert_eq!(
        diagnostics(service, "\"é😀\" & items.nme"),
        [("W0201".to_owned(), 14, 17)]
    );
    assert_eq!(jlens_service_free(service), 1);
    assert_eq!(jlens_schema_free(schema), 1);
}

#[test]
fn a_service_outlives_the_schemas_it_was_made_from() {
    let document = schema(&order());
    let customer = schema(&json!({
        "type": "object",
        "properties": {"name": {"type": "string"}}
    }));
    let declarations = json!({"variables": {"customer": customer, "$limit": null}});
    let service = handle(service(document, Some(&declarations)));
    assert_eq!(jlens_schema_free(document), 1);
    assert_eq!(jlens_schema_free(customer), 1);
    assert_eq!(jlens_schema_free(customer), 0, "freed once");
    assert_eq!(codes(&diagnostics(service, "items.nme")), ["W0201"]);
    assert_eq!(codes(&diagnostics(service, "$customer.nme")), ["W0201"]);
    assert!(diagnostics(service, "$customer.name & $limit").is_empty());
    // With variables declared, another is not known.
    assert_eq!(codes(&diagnostics(service, "$other")), ["W0204"]);
    assert_eq!(jlens_service_free(service), 1);
    assert_eq!(jlens_service_free(service), 0, "freed once");
    assert_eq!(
        error(with_buffer(b"a", |buffer| jlens_service_diagnostics(
            service, buffer
        ))),
        ("U2002".to_owned(), Some("service".to_owned()))
    );
}

#[test]
fn declared_functions_are_known_with_their_signatures() {
    let declarations = json!({"functions": [
        {"name": "$discount", "signature": "<n:n>", "params": ["total"], "summary": "The discount."},
        {"name": "tax"}
    ]});
    let service = handle(service(0, Some(&declarations)));
    assert!(diagnostics(service, "$discount(1) + $tax(2, 3)").is_empty());
    assert_eq!(codes(&diagnostics(service, "$discount()")), ["W0203"]);
    assert_eq!(codes(&diagnostics(service, "$discount(\"a\")")), ["W0205"]);
    assert_eq!(codes(&diagnostics(service, "$rebate(1)")), ["W0202"]);
    // No schema: no field is unknown.
    assert!(diagnostics(service, "anything.at.all").is_empty());
    assert_eq!(jlens_service_free(service), 1);
}

#[test]
fn a_service_of_nothing_still_checks_the_syntax_and_the_functions() {
    let service = handle(service(0, None));
    assert!(diagnostics(service, "$sum(a.b)").is_empty());
    assert_eq!(codes(&diagnostics(service, "$nope()")), ["W0202"]);
    assert_eq!(jlens_service_free(service), 1);
}

#[test]
fn failures_use_the_codes_of_the_c_api() {
    let code = |reply| error(reply).0;

    // A schema.
    assert_eq!(
        code(with_buffer(b"{\"type\": ", |buffer| jlens_schema_new(
            buffer
        ))),
        "U2001"
    );
    assert_eq!(
        code(with_buffer(&[0xff, 0xfe], |buffer| jlens_schema_new(
            buffer
        ))),
        "U2001"
    );
    let refused = with_buffer(b"[1]", |buffer| jlens_schema_new(buffer));
    assert_eq!(
        error(refused),
        ("U2005".to_owned(), Some("schema".to_owned()))
    );
    // A `$ref` that leads nowhere leaves its place unknown: the schema loads.
    let unresolved = schema(&json!({"$ref": "https://example.com/other.json"}));
    assert_eq!(jlens_schema_free(unresolved), 1);
    assert_eq!(error(reply(jlens_schema_new(0))).0, "U2002");
    assert_eq!(jlens_schema_free(0), 0);

    // A service.
    let schema = schema(&order());
    assert_eq!(
        error(reply(jlens_service_new(schema ^ (1 << 40), 0))),
        ("U2002".to_owned(), Some("schema".to_owned()))
    );
    assert_eq!(
        error(reply(jlens_service_new(schema, schema ^ (1 << 40)))),
        ("U2002".to_owned(), Some("buffer".to_owned()))
    );
    let new = |text: &[u8]| with_buffer(text, |buffer| jlens_service_new(schema, buffer));
    assert_eq!(code(new(b"{\"variables\": ")), "U2001");
    for declarations in [
        &b"[]"[..],
        b"{\"variable\": {}}",
        b"{\"variables\": []}",
        b"{\"variables\": {\"x\": \"1\"}}",
        b"{\"variables\": {\"x\": -1}}",
        b"{\"variables\": {\"x\": 1.5}}",
        b"{\"functions\": {}}",
        b"{\"functions\": [{\"name\": \"f\", \"arity\": 1}]}",
        b"{\"functions\": [{\"name\": 1}]}",
    ] {
        assert_eq!(
            error(new(declarations)),
            ("U2005".to_owned(), Some("service".to_owned())),
            "{}",
            String::from_utf8_lossy(declarations)
        );
    }
    let stranger = json!({"variables": {"x": schema ^ (1 << 40)}});
    assert_eq!(
        error(service(schema, Some(&stranger))),
        ("U2002".to_owned(), Some("schema".to_owned()))
    );
    // Members that are null declare nothing.
    let nothing = json!({"variables": null, "functions": null});
    let service = handle(service(schema, Some(&nothing)));

    // Diagnostics.
    assert_eq!(
        error(reply(jlens_service_diagnostics(service, 0))),
        ("U2002".to_owned(), Some("buffer".to_owned()))
    );
    assert_eq!(
        code(with_buffer(&[b'a', 0xff], |buffer| {
            jlens_service_diagnostics(service, buffer)
        })),
        "U2003"
    );
    assert_eq!(jlens_service_free(service), 1);
    assert_eq!(jlens_schema_free(schema), 1);
}

fn compile(text: &str) -> u64 {
    handle(with_buffer(text.as_bytes(), |buffer| jlens_compile(buffer)))
}

fn json(reply: Reply) -> Value {
    match reply {
        Reply::Json(value) => value,
        other => panic!("not JSON: {other:?}"),
    }
}

fn changes(old: &Value, new: &Value) -> Reply {
    let old = buffers::insert(old.to_string().into_bytes());
    let result = with_buffer(new.to_string().as_bytes(), |new| {
        jlens_uses_changes(old, new)
    });
    assert_eq!(jlens_buffer_free(old), 1);
    result
}

/// The cases every package answers with the same JSON (roadmap 0.33 #7).
#[test]
fn uses_and_changes_are_those_of_the_shared_cases() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/uses/cases.json"
    );
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let make = |schema_of: &str, variables: &str| {
        let input = schema(&fixture[schema_of]);
        let mut schemas = vec![input];
        let mut declared = serde_json::Map::new();
        for (name, document) in fixture[variables].as_object().unwrap() {
            let value = if document.is_null() {
                Value::Null
            } else {
                schemas.push(schema(document));
                json!(schemas[schemas.len() - 1])
            };
            declared.insert(name.clone(), value);
        }
        let declarations = json!({"variables": declared, "functions": fixture["functions"]});
        let service = handle(service(input, Some(&declarations)));
        for schema in schemas {
            assert_eq!(jlens_schema_free(schema), 1);
        }
        service
    };
    let old = make("schema", "variables");
    let new = make("newSchema", "newVariables");
    for case in fixture["cases"].as_array().unwrap() {
        let text = case["expression"].as_str().unwrap();
        let expression = compile(text);
        let uses = json(reply(jlens_service_uses(old, expression)));
        assert_eq!(uses, case["uses"], "{text}");
        let after = json(reply(jlens_service_uses(new, expression)));
        assert_eq!(json(changes(&uses, &after)), case["changes"], "{text}");
        assert_eq!(json(changes(&uses, &uses)), json!({}), "{text}");
        assert_eq!(jlens_expression_free(expression), 1);
    }
    assert_eq!(jlens_service_free(old), 1);
    assert_eq!(jlens_service_free(new), 1);
}

#[test]
fn uses_and_changes_fail_with_the_codes_of_the_c_api() {
    let service = handle(service(0, None));
    let expression = compile("a.b");
    assert_eq!(
        json(reply(jlens_service_uses(service, expression))),
        json!({
            "input": {
                "$": {"unknown": true},
                "a": {"unknown": true},
                "a.b": {"unknown": true, "whole": true}
            },
            "bindings": {}
        })
    );
    assert_eq!(
        error(reply(jlens_service_uses(service ^ (1 << 40), expression))),
        ("U2002".to_owned(), Some("service".to_owned()))
    );
    assert_eq!(
        error(reply(jlens_service_uses(service, expression ^ (1 << 40)))),
        ("U2002".to_owned(), Some("expression".to_owned()))
    );
    assert_eq!(jlens_expression_free(expression), 1);
    assert_eq!(jlens_service_free(service), 1);

    let none = json!({"bindings": {}});
    assert_eq!(json(changes(&none, &json!({}))), json!({}));
    for (text, code, token) in [
        (&b"{\"bindings\": "[..], "U2001", None),
        (&[0xff, 0xfe][..], "U2001", None),
        (b"[]", "U2005", Some("uses")),
        (b"{\"inputs\": {}}", "U2005", Some("uses")),
        (b"{\"input\": {\"a\": []}}", "U2005", Some("uses")),
        (
            b"{\"bindings\": {\"x\": {\"a\": []}}}",
            "U2005",
            Some("uses"),
        ),
        (
            b"{\"input\": {\"a\": {\"types\": [\"text\"]}}}",
            "U2005",
            Some("uses"),
        ),
    ] {
        let other = buffers::insert(none.to_string().into_bytes());
        for first in [true, false] {
            let failed = with_buffer(text, |buffer| match first {
                true => jlens_uses_changes(buffer, other),
                false => jlens_uses_changes(other, buffer),
            });
            assert_eq!(
                error(failed),
                (code.to_owned(), token.map(str::to_owned)),
                "{}",
                String::from_utf8_lossy(text)
            );
        }
        assert_eq!(jlens_buffer_free(other), 1);
    }
    assert_eq!(
        error(reply(jlens_uses_changes(0, 0))),
        ("U2002".to_owned(), Some("buffer".to_owned()))
    );
}
