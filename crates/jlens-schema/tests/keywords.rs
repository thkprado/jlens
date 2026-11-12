//! The JSON Schema keywords a load reads, those it leaves out because they only narrow what a
//! place may hold, and those it reports as not read ([`LoadReport::unread`]).

use jlens_schema::{JsonType, LanguageService, LoadReport, Schema, SchemaLoader};
use proptest::prelude::*;
use serde_json::{Value, json};

fn codes(service: &LanguageService, text: &str) -> Vec<String> {
    service
        .diagnostics(text)
        .into_iter()
        .map(|d| d.code.to_string())
        .collect()
}

fn load(document: &Value) -> (LanguageService, LoadReport) {
    let (schema, report) = SchemaLoader::new().load(document).unwrap();
    (LanguageService::new(Some(schema)), report)
}

fn types(schema: &Schema, path: &str) -> Option<Vec<JsonType>> {
    schema.shape_at(path).map(|s| s.types.clone())
}

/// The schema of the property test: each keyword the load reads beyond those of 0.33.
fn order() -> Value {
    json!({
        "type": "object",
        "required": ["kind", "lines", "point", "note"],
        "properties": {
            "kind": {"enum": ["person", "company"]},
            "lines": {
                "type": "array",
                "prefixItems": [{"type": "string"}],
                "items": {"type": "number"}
            },
            "point": {"type": "array", "items": [{"type": "number"}, {"type": "number"}]},
            "note": {"type": "string", "nullable": true},
            "extra": {"type": "object", "unevaluatedProperties": {"type": "integer"}}
        },
        "if": {"properties": {"kind": {"const": "company"}}},
        "then": {"required": ["tax"], "properties": {"tax": {"type": "string"}}},
        "else": {"properties": {"age": {"type": "integer"}, "tax": {"type": "number"}}},
        "dependentSchemas": {"age": {"properties": {"guardian": {"type": "string"}}}}
    })
}

fn order_samples() -> [Value; 3] {
    [
        json!({
            "kind": "company", "tax": "X1", "lines": ["head", 1, 2.5], "point": [1, 2, "z"],
            "note": null, "extra": {"a": 1}
        }),
        json!({
            "kind": "person", "age": 30, "guardian": "Ann", "tax": 2, "lines": ["h"],
            "point": [3], "note": "n"
        }),
        json!({"kind": "person", "lines": [], "point": [], "note": "m"}),
    ]
}

#[test]
fn conditional_fields_are_optional_fields() {
    let (service, report) = load(&order());
    assert!(report.is_empty(), "{report:?}");
    for text in ["tax", "age", "guardian", "age + 1", "guardian & 'x'"] {
        assert!(
            codes(&service, text).is_empty(),
            "{text}: {:?}",
            codes(&service, text)
        );
    }
    assert_eq!(codes(&service, "tx"), ["W0201"]);
    // `tax` is a string in `then` and a number in `else`: either, when it is there.
    let schema = Schema::from_json_schema(&order()).unwrap();
    assert_eq!(
        types(&schema, "tax"),
        Some(vec![JsonType::Number, JsonType::String])
    );
    // Required in `then` only: the field may be absent.
    assert!(!schema.root.fields["tax"].required);
    // `then` without `if` holds for nothing.
    let schema = Schema::from_json_schema(&json!({
        "properties": {"a": {"type": "string"}},
        "then": {"properties": {"b": {"type": "string"}}}
    }))
    .unwrap();
    assert_eq!(types(&schema, "b"), None);
}

#[test]
fn conditional_fields_at_any_depth() {
    let schema = json!({
        "type": "object",
        "properties": {
            "address": {"type": "object", "properties": {"city": {"type": "string"}}}
        },
        "if": {"required": ["address"]},
        "then": {"properties": {"address": {"properties": {"zip": {"type": "string"}}}}},
        "dependencies": {
            "address": {"properties": {"country": {"type": "string"}}},
            "city": ["address"]
        }
    });
    let (service, report) = load(&schema);
    assert!(report.is_empty());
    for text in ["address.zip", "address.city", "country"] {
        assert!(codes(&service, text).is_empty(), "{text}");
    }
    assert_eq!(codes(&service, "address.zipp"), ["W0201"]);
    // An object whose fields were not known stays one: a condition does not close it.
    let (service, _) = load(&json!({
        "type": "object",
        "if": {"required": ["a"]},
        "then": {"properties": {"b": {"type": "string"}}}
    }));
    assert!(codes(&service, "b & c").is_empty());
    // A condition that opens the object to other fields opens it.
    let (service, _) = load(&json!({
        "type": "object",
        "properties": {"a": {"type": "string"}},
        "if": {"required": ["a"]},
        "then": {"additionalProperties": {"type": "number"}}
    }));
    assert!(codes(&service, "other + 1").is_empty());
}

#[test]
fn the_items_of_a_tuple() {
    let schema = Schema::from_json_schema(&order()).unwrap();
    // `prefixItems` and the rest, merged.
    assert_eq!(
        types(&schema, "lines[]"),
        Some(vec![JsonType::Number, JsonType::String])
    );
    // Draft-07's tuple, with no `additionalItems`: any item may follow, so the items are not
    // known, as those of `{"type": "array"}`.
    assert_eq!(types(&schema, "point[]"), None);
    for (rest, expected) in [
        (
            json!({"additionalItems": false}),
            Some(vec![JsonType::Number]),
        ),
        (
            json!({"additionalItems": {"type": "string"}}),
            Some(vec![JsonType::Number, JsonType::String]),
        ),
        (json!({"maxItems": 1}), Some(vec![JsonType::Number])),
        (json!({"maxItems": 2}), None),
        (
            json!({"unevaluatedItems": {"type": "boolean"}}),
            Some(vec![JsonType::Boolean, JsonType::Number]),
        ),
        (
            json!({"prefixItems": [{"type": "string"}], "items": false}),
            Some(vec![JsonType::String]),
        ),
    ] {
        let mut point = json!({"type": "array", "items": [{"type": "number"}]});
        if rest.get("prefixItems").is_some() {
            point = json!({"type": "array"});
        }
        point
            .as_object_mut()
            .unwrap()
            .extend(rest.as_object().unwrap().clone());
        let schema = Schema::from_json_schema(&json!({"properties": {"point": point}})).unwrap();
        assert_eq!(types(&schema, "point[]"), expected, "{point}");
    }
    // `unevaluatedItems` alone holds for every item.
    let schema = Schema::from_json_schema(&json!({
        "properties": {"tags": {"type": "array", "unevaluatedItems": {"type": "string"}}}
    }))
    .unwrap();
    assert_eq!(types(&schema, "tags[]"), Some(vec![JsonType::String]));
}

#[test]
fn nullable_adds_null() {
    let (service, report) = load(&order());
    assert!(report.is_empty());
    assert!(codes(&service, "note = null").is_empty());
    let schema = Schema::from_json_schema(&json!({
        "properties": {
            "status": {"type": "string", "enum": ["open", "closed"], "nullable": true},
            "any": {"nullable": true}
        }
    }))
    .unwrap();
    assert_eq!(
        types(&schema, "status"),
        Some(vec![JsonType::Null, JsonType::String])
    );
    let status = schema.shape_at("status").unwrap();
    assert_eq!(
        status.enumeration.as_ref().unwrap().last(),
        Some(&Value::Null)
    );
    // With no type, `nullable` says nothing: the types stay unknown.
    assert_eq!(types(&schema, "any"), Some(vec![]));
}

#[test]
fn unevaluated_properties_as_additional_properties() {
    let (service, _) = load(&order());
    assert!(codes(&service, "extra.b + 1").is_empty());
    assert_eq!(codes(&service, "extra.b > 'a'"), ["W0208"]);
    // The fields of the branches are evaluated: `false` closes the object to the others.
    let (service, _) = load(&json!({
        "allOf": [{"properties": {"a": {"type": "string"}}}],
        "properties": {"b": {"type": "string"}},
        "unevaluatedProperties": false
    }));
    assert!(codes(&service, "a & b").is_empty());
    assert_eq!(codes(&service, "c"), ["W0201"]);
}

#[test]
fn keywords_that_narrow_are_left_out() {
    // Each of these only takes values away: leaving it out cannot give a false warning.
    let narrowing = json!({
        "type": "object",
        "required": ["a"],
        "properties": {
            "a": {
                "type": "string", "minLength": 1, "maxLength": 9, "pattern": "^x",
                "not": {"const": "xy"}, "contentMediaType": "text/plain"
            },
            "n": {
                "type": "number", "multipleOf": 2, "exclusiveMinimum": 0,
                "exclusiveMaximum": 9, "deprecated": true, "readOnly": true
            },
            "list": {
                "type": "array", "items": {"type": "string"}, "maxItems": 3,
                "uniqueItems": true, "contains": {"const": "x"}, "minContains": 1
            }
        },
        "minProperties": 1, "maxProperties": 5, "propertyNames": {"pattern": "^[a-z]"},
        "dependentRequired": {"n": ["list"]}, "$comment": "c", "default": {}
    });
    let (service, report) = load(&narrowing);
    assert!(report.is_empty(), "{report:?}");
    assert!(codes(&service, "a & n & list").is_empty());
    let schema = Schema::from_json_schema(&narrowing).unwrap();
    assert_eq!(types(&schema, "a"), Some(vec![JsonType::String]));
    assert_eq!(types(&schema, "list[]"), Some(vec![JsonType::String]));
}

#[test]
fn keywords_not_read_are_reported() {
    let tree = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$dynamicAnchor": "node",
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "children": {"type": "array", "items": {"$dynamicRef": "#node"}},
            "code": {"type": ["string", "int"]},
            "legacy": {"$recursiveRef": "#"},
            "size": {"type": 5}
        }
    });
    let (service, report) = load(&tree);
    let unread: Vec<(&str, &str)> = report
        .unread
        .iter()
        .map(|u| (u.place.as_str(), u.keyword.as_str()))
        .collect();
    assert_eq!(
        unread,
        [
            ("#/properties/children/items", "$dynamicRef"),
            ("#/properties/code", "type"),
            ("#/properties/legacy", "$recursiveRef"),
            ("#/properties/size", "type"),
        ]
    );
    assert!(!report.is_empty());
    assert!(report.unresolved.is_empty());
    // Nothing is warned under them; the rest of the schema is checked as usual.
    for text in [
        "children.anything",
        "code = 1",
        "code = null",
        "legacy.x",
        "size & 1",
    ] {
        assert!(codes(&service, text).is_empty(), "{text}");
    }
    assert_eq!(codes(&service, "nme"), ["W0201"]);
    // The JSON form every package gives.
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(
        json["unread"][1],
        json!({"place": "#/properties/code", "keyword": "type"})
    );
    assert_eq!(json.get("unresolved"), None);
    // A place met twice, through two `$ref`s, is reported once, in a document given with a URI.
    let (_, report) = SchemaLoader::new()
        .load_at(
            "https://example.com/a.json",
            &json!({
                "properties": {"x": {"$ref": "#/$defs/d"}, "y": {"$ref": "#/$defs/d"}},
                "$defs": {"d": {"type": "integer32"}}
            }),
        )
        .unwrap();
    assert_eq!(report.unread.len(), 1);
    assert_eq!(
        report.unread[0].place,
        "https://example.com/a.json#/$defs/d"
    );
}

/// The types a JSON value has, as the typing names them.
fn types_of(value: &Value) -> Vec<JsonType> {
    match value {
        Value::Null => vec![JsonType::Null],
        Value::Bool(_) => vec![JsonType::Boolean],
        Value::Number(n) if n.as_f64().is_some_and(|f| f.fract() == 0.0) => {
            vec![JsonType::Integer, JsonType::Number]
        }
        Value::Number(_) => vec![JsonType::Number],
        Value::String(_) => vec![JsonType::String],
        Value::Array(_) => vec![JsonType::Array],
        Value::Object(_) => vec![JsonType::Object],
    }
}

#[rustfmt::skip]
const TOKENS: &[&str] = &[
    "kind", "tax", "age", "guardian", "lines", "point", "note", "extra", "a", ".", "[0]", "[1]",
    "[", "]", "(", ")", "1", "'s'", "null", "+", ">", "=", "and", "&", "$sum(", "$count(",
    "$exists(", "$string(", ",", " ",
];

fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(TOKENS), 0..14).prop_map(|t| t.join(""))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// What the typing says of a result holds on documents that follow a schema with the
    /// keywords of this file, and a warning that a field does not exist is never given for
    /// a field one of them has.
    #[test]
    fn the_keywords_read_type_as_their_documents(text in text()) {
        let schema = Schema::from_json_schema(&order()).unwrap();
        let service = LanguageService::new(Some(schema)).with_result_type([JsonType::Boolean]);
        let diagnostics = service.diagnostics(&text);
        for d in &diagnostics {
            prop_assert!(d.span.start <= d.span.end && d.span.end <= text.len(), "{d:?}");
            // Each name of the schema is a field of one of the samples.
            prop_assert!(d.code != "W0201" || !TOKENS[..8].contains(&&text[d.span.start..d.span.end]),
                "{d:?}");
        }
        let result = service.result_type(&text);
        if let (Some(types), Ok(expr)) = (&result.types, jlens_core::Expression::compile(&text)) {
            let options = jlens_core::Options::default().max_steps(10_000);
            let bindings = jlens_core::Bindings::new();
            for sample in &order_samples() {
                match expr.evaluate_with(Some(sample), &bindings, &options) {
                    Ok(None) => prop_assert!(result.undefined.is_some(), "undefined: {result:?}"),
                    Ok(Some(value)) => {
                        let actual = types_of(&value);
                        prop_assert!(
                            actual.iter().any(|t| types.contains(t)),
                            "{value} on {sample}: {result:?}"
                        );
                    }
                    Err(_) => {}
                }
            }
        }
    }
}
