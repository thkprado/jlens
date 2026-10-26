//! Audited evaluations (0.25 #8): the reply's result, error, audit tree and explanation, as
//! the WebAssembly and Python suites check theirs.

use jlens_capi::exports::*;
use jlens_capi::values::Encoder;
use jlens_capi::{REPLY_ERROR, REPLY_HANDLE, REPLY_VALUES, buffers};
use serde_json::{Value, json};

mod values;

/// The rule of the WebAssembly and Python suites.
const RULE: &str = r#"
/** @label "Minimum total" @message "Order total below minimum" */ $sum(items.(price * qty)) >= $minimum
and /** @label "Has items" */ $count(items) > 0
"#;

fn order() -> Value {
    json!({"items": [{"price": 100, "qty": 2}, {"price": 50, "qty": 1}]})
}

fn compiled(source: &str) -> u64 {
    let buffer = buffers::insert(source.as_bytes().to_vec());
    let reply = buffers::take(jlens_compile(buffer)).unwrap();
    assert_eq!(jlens_buffer_free(buffer), 1);
    assert_eq!(reply[0], REPLY_HANDLE, "{source}");
    u64::from_le_bytes(reply[1..].try_into().unwrap())
}

fn values_buffer(value: &Value) -> u64 {
    let mut encoder = Encoder::new(Vec::new());
    encoder.json(value);
    buffers::insert(encoder.out)
}

/// Bindings of variables only.
fn variables(variables: &Value) -> u64 {
    let buffer = values_buffer(variables);
    let reply = buffers::take(jlens_bindings_new(0, buffer, None, 0)).unwrap();
    assert_eq!(jlens_buffer_free(buffer), 1);
    assert_eq!(reply[0], REPLY_HANDLE);
    u64::from_le_bytes(reply[1..].try_into().unwrap())
}

/// The reply of an audited evaluation: the object of values, or the error envelope.
fn audited(
    source: &str,
    input: Option<&Value>,
    bindings: Option<&Value>,
    audit: Option<&str>,
) -> Result<Value, Value> {
    let expression = compiled(source);
    let input = input.map_or(0, values_buffer);
    let bindings = bindings.map_or(0, variables);
    let audit = audit.map_or(0, |text| buffers::insert(text.as_bytes().to_vec()));
    let reply = buffers::take(jlens_evaluate_audited(
        expression, input, bindings, 0, audit,
    ));
    for buffer in [input, audit] {
        if buffer != 0 {
            assert_eq!(jlens_buffer_free(buffer), 1);
        }
    }
    if bindings != 0 {
        assert_eq!(jlens_bindings_free(bindings), 1);
    }
    assert_eq!(jlens_expression_free(expression), 1);
    let reply = reply.expect("a reply");
    match reply[0] {
        REPLY_VALUES => Ok(values::decode(&reply[1..])),
        REPLY_ERROR => Err(serde_json::from_slice(&reply[1..]).unwrap()),
        status => panic!("unexpected status {status}"),
    }
}

#[test]
fn an_audit_has_the_result_the_tree_and_the_explanation() {
    let audit = audited(RULE, Some(&order()), Some(&json!({"minimum": 60000})), None).unwrap();
    assert_eq!(audit["result"], json!(false));
    assert!(audit.get("error").is_none());
    assert_eq!(audit["audit"]["header"]["level"], "decisions");
    assert_eq!(audit["audit"]["header"]["result"], json!(false));
    assert!(audit["audit"]["root"].is_object());
    let failed: Vec<&str> = audit["explanation"]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|line| line["passed"] == json!(false))
        .map(|line| line["text"].as_str().unwrap())
        .collect();
    assert!(
        failed.iter().any(|text| text.starts_with("Minimum total")),
        "{failed:?}"
    );
    let text = audit["explanationText"].as_str().unwrap();
    assert!(text.starts_with("result: false"), "{text}");
    assert!(text.contains("✘ Minimum total"), "{text}");

    let full = r#"{"level": "full", "limits": {"maxNodes": 5}}"#;
    let full = audited(
        RULE,
        Some(&order()),
        Some(&json!({"minimum": 1})),
        Some(full),
    )
    .unwrap();
    assert_eq!(full["result"], json!(true));
    assert_eq!(full["audit"]["nodeCount"], json!(5));
    assert_eq!(full["audit"]["truncated"], json!(true));
    let mut stack = vec![&full["audit"]["root"]];
    let mut nodes = 0;
    while let Some(node) = stack.pop() {
        nodes += 1;
        stack.extend(node["children"].as_array().into_iter().flatten());
    }
    assert_eq!(nodes, 5, "the tree holds the nodes it counts");
    let off = r#"{"level": "off"}"#;
    let off = audited(
        RULE,
        Some(&order()),
        Some(&json!({"minimum": 1})),
        Some(off),
    )
    .unwrap();
    assert!(off["audit"].get("root").is_none());
    // No input at all.
    let none = audited("$exists($)", None, None, None).unwrap();
    assert_eq!(none["result"], json!(false));
}

#[test]
fn the_error_of_the_evaluation_is_in_the_reply() {
    let failing = audited("$x.y + 1", None, Some(&json!({"x": {"y": "a"}})), None).unwrap();
    assert!(failing.get("result").is_none());
    let envelope: Value = serde_json::from_str(failing["error"].as_str().unwrap()).unwrap();
    assert_eq!(envelope["code"], "T2001");
    assert_eq!(envelope["value"], "a");
    assert_eq!(failing["audit"]["header"]["error"]["code"], "T2001");
    // Offsets in UTF-16.
    let failing = audited(
        "'𝄞' & $x.y + 1",
        None,
        Some(&json!({"x": {"y": "a"}})),
        None,
    );
    let envelope: Value =
        serde_json::from_str(failing.unwrap()["error"].as_str().unwrap()).unwrap();
    assert_eq!(envelope["position"], json!(13));
}

#[test]
fn audit_spans_are_utf16_offsets() {
    let source = "'𝄞' = 'x' or /** @label \"B\" */ $b";
    let audit = audited(source, None, Some(&json!({"b": true})), None).unwrap();
    let mut spans = Vec::new();
    let mut pending = vec![&audit["audit"]["root"]];
    while let Some(node) = pending.pop() {
        spans.push((
            node["source"].as_str().unwrap().to_owned(),
            node["span"]["start"].clone(),
            node["span"]["end"].clone(),
        ));
        if let Some(children) = node["children"].as_array() {
            pending.extend(children);
        }
    }
    let comparison = spans.iter().find(|(s, ..)| s == "'𝄞' = 'x'").unwrap();
    assert_eq!((&comparison.1, &comparison.2), (&json!(0), &json!(10)));
    let labelled = spans.iter().find(|(s, ..)| s == "$b").unwrap();
    assert_eq!((&labelled.1, &labelled.2), (&json!(32), &json!(34)));
    // A captured value's own `span` field is data, left as it is.
    let value = json!({"span": {"start": 4, "end": 9}});
    let full = audited(
        "($e := 'é'; $x)",
        None,
        Some(&json!({"x": value})),
        Some(r#"{"level": "full"}"#),
    )
    .unwrap();
    let mut outputs = Vec::new();
    let mut pending = vec![&full["audit"]["root"]];
    while let Some(node) = pending.pop() {
        if node["source"] == "$x" {
            outputs.push(node["output"].clone());
        }
        if let Some(children) = node["children"].as_array() {
            pending.extend(children);
        }
    }
    assert_eq!(outputs, vec![value]);
}

#[test]
fn audit_options_not_in_their_form_fail_with_u2005() {
    for options in [
        r#"{"level": "everything"}"#,
        r#"{"limits": {"maxNodez": 1}}"#,
        r#"{"limits": {"maxNodes": "1"}}"#,
        r#"{"limits": 1}"#,
        r#"{"levels": "full"}"#,
        "[]",
        "{",
    ] {
        let error = audited("1", None, None, Some(options)).unwrap_err();
        assert_eq!(
            (&error["code"], &error["token"]),
            (&json!("U2005"), &json!("audit")),
            "{options}"
        );
    }
    // An unknown handle comes first.
    let expression = compiled("1");
    let audit = buffers::insert(b"not JSON".to_vec());
    let reply = buffers::take(jlens_evaluate_audited(expression, 0, 0, 0, audit + 1)).unwrap();
    let error: Value = serde_json::from_slice(&reply[1..]).unwrap();
    assert_eq!(error["code"], "U2002");
    assert_eq!(jlens_buffer_free(audit), 1);
    let reply = buffers::take(jlens_evaluate_audited(expression + 1, 0, 0, 0, 0)).unwrap();
    let error: Value = serde_json::from_slice(&reply[1..]).unwrap();
    assert_eq!(error["code"], "U2002");
    // Input that is not values.
    let input = buffers::insert(vec![99]);
    let reply = buffers::take(jlens_evaluate_audited(expression, input, 0, 0, 0)).unwrap();
    let error: Value = serde_json::from_slice(&reply[1..]).unwrap();
    assert_eq!(error["code"], "U2005");
    assert_eq!(jlens_buffer_free(input), 1);
    assert_eq!(jlens_expression_free(expression), 1);
}
