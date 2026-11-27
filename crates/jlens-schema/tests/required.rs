//! The fields a JSON Schema requires, wherever its `required` is written: beside `properties`,
//! in a branch of `allOf`, beside a `$ref`, or with no `properties` at all.

use jlens_schema::{JsonType, LanguageService, Schema};
use serde_json::{Value, json};

fn load(document: Value) -> Schema {
    Schema::from_json_schema(&document).unwrap()
}

fn codes(schema: &Schema, text: &str) -> Vec<String> {
    LanguageService::new(Some(schema.clone()))
        .diagnostics(text)
        .into_iter()
        .map(|d| d.code.to_string())
        .collect()
}

fn required(schema: &Schema, path: &str) -> Option<bool> {
    let (parent, name) = match path.rsplit_once('.') {
        Some((parent, name)) => (schema.shape_at(parent)?, name),
        None => (&schema.root, path),
    };
    parent.fields.get(name).map(|f| f.required)
}

#[test]
fn a_field_that_one_branch_of_all_of_requires_is_required() {
    // Both branches list `a`; only the first requires it.
    let schema = load(json!({"allOf": [
        {"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]},
        {"properties": {"a": {"minLength": 1}, "b": {"type": "number"}}}
    ]}));
    assert_eq!(required(&schema, "a"), Some(true));
    assert_eq!(schema.root.fields["a"].presence, 1.0);
    assert_eq!(required(&schema, "b"), Some(false));
    // At any depth: every branch holds for the fields of the fields too.
    let schema = load(json!({"allOf": [
        {"properties": {"o": {"properties": {"x": {}}, "required": ["x"]}}},
        {"properties": {"o": {"properties": {"x": {"type": "string"}}}}}
    ]}));
    assert_eq!(required(&schema, "o.x"), Some(true));
}

#[test]
fn required_in_a_branch_without_properties() {
    let schema = load(json!({
        "type": "object",
        "properties": {"a": {"type": "string"}, "b": {"type": "number"}},
        "allOf": [{"required": ["a"]}]
    }));
    assert_eq!(required(&schema, "a"), Some(true));
    assert_eq!(required(&schema, "b"), Some(false));
    // The branch lists no field: the object still has those of `properties` alone.
    assert_eq!(codes(&schema, "c"), ["W0201"]);
    assert!(codes(&schema, "a & b").is_empty());

    // A mixin behind a `$ref` reads the same.
    let schema = load(json!({
        "$defs": {"identified": {"required": ["id"]}},
        "type": "object",
        "properties": {"id": {"type": "string"}, "name": {"type": "string"}},
        "allOf": [{"$ref": "#/$defs/identified"}]
    }));
    assert_eq!(required(&schema, "id"), Some(true));
    assert_eq!(codes(&schema, "nme"), ["W0201"]);
}

#[test]
fn required_beside_a_ref() {
    let schema = load(json!({
        "$defs": {"base": {"type": "object", "properties": {"id": {"type": "string"}}}},
        "$ref": "#/$defs/base",
        "required": ["id"]
    }));
    assert_eq!(required(&schema, "id"), Some(true));

    // `properties` beside the `$ref` add to what it says of a field, and keep its `required`.
    let schema = load(json!({
        "$defs": {"base": {
            "type": "object",
            "properties": {"id": {"type": "string"}},
            "required": ["id"]
        }},
        "$ref": "#/$defs/base",
        "properties": {"id": {"description": "The key"}}
    }));
    let id = &schema.root.fields["id"];
    assert!(id.required);
    assert_eq!(id.shape.types, [JsonType::String]);
    assert_eq!(id.shape.description.as_deref(), Some("The key"));
}

#[test]
fn required_without_properties() {
    // A field required and not listed is a field of unknown shape, never warned about.
    let schema = load(json!({
        "type": "object",
        "properties": {"a": {"type": "string"}},
        "required": ["a", "b"]
    }));
    assert_eq!(required(&schema, "b"), Some(true));
    assert_eq!(schema.shape_at("b").unwrap().types, []);
    assert!(codes(&schema, "b").is_empty());
    assert_eq!(codes(&schema, "c"), ["W0201"]);

    // With no `properties` at all, the object lists no field: any other may be there.
    let schema = load(json!({"type": "object", "required": ["id"]}));
    assert_eq!(required(&schema, "id"), Some(true));
    assert!(codes(&schema, "id & name").is_empty());
    let schema = load(json!({"type": "object", "allOf": [{"required": ["id"]}]}));
    assert_eq!(required(&schema, "id"), Some(true));
    assert!(codes(&schema, "id & name").is_empty());
    // But closed by `additionalProperties`, only the required field is there.
    let schema = load(json!({
        "type": "object",
        "required": ["id"],
        "additionalProperties": false
    }));
    assert!(codes(&schema, "id").is_empty());
    assert_eq!(codes(&schema, "name"), ["W0201"]);

    // A schema that gives no type says nothing of its value, `required` or not.
    let schema = load(json!({"required": ["id"]}));
    assert_eq!(schema.root.types, []);
    assert!(codes(&schema, "id & name").is_empty());
}

#[test]
fn required_in_branches_of_any_of() {
    // A field is required where every branch requires it, whether it lists it or not.
    let schema = load(json!({
        "type": "object",
        "properties": {"a": {"type": "string"}, "b": {"type": "string"}},
        "anyOf": [
            {"required": ["a"]},
            {"properties": {"a": {"minLength": 1}}, "required": ["a", "b"]}
        ]
    }));
    assert_eq!(required(&schema, "a"), Some(true));
    assert_eq!(required(&schema, "b"), Some(false));
    // One of two: neither is required, and the object keeps its fields.
    let schema = load(json!({
        "type": "object",
        "properties": {"a": {"type": "string"}, "b": {"type": "string"}},
        "oneOf": [{"required": ["a"]}, {"required": ["b"]}]
    }));
    assert_eq!(required(&schema, "a"), Some(false));
    assert_eq!(required(&schema, "b"), Some(false));
    assert_eq!(codes(&schema, "c"), ["W0201"]);
}

#[test]
fn required_fields_survive_a_round_trip() {
    for document in [
        json!({"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a", "b"]}),
        json!({"type": "object", "required": ["id"]}),
        json!({"type": "object", "properties": {"a": {}}, "allOf": [{"required": ["a"]}]}),
    ] {
        let schema = load(document);
        let back = Schema::from_json_schema(&schema.to_json_schema()).unwrap();
        assert_eq!(back, schema);
    }
}
