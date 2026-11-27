//! The fields of a schema, listed ([`Schema::fields`]): what completion and hover tell of each,
//! with its own `x-` keywords.

use std::collections::BTreeMap;

use jlens_schema::{JsonType, ListedField, Override, Schema, SchemaLoader};
use serde_json::{Value, json};

fn listed(schema: &Schema) -> BTreeMap<String, ListedField> {
    schema
        .fields()
        .into_iter()
        .map(|f| (f.path.clone(), f))
        .collect()
}

/// An order whose fields carry an application's own keys, a customer behind a `$ref` with
/// keys of its own and of its place, a hidden field with fields under it, and an unknown place.
fn order() -> Value {
    json!({
        "$defs": {
            "customer": {
                "type": "object",
                "title": "Customer",
                "x-entity": "customer",
                "required": ["name"],
                "properties": {"name": {"type": "string", "x-attribute-id": 2}}
            }
        },
        "type": "object",
        "title": "Order",
        "x-entity": "order",
        "required": ["id"],
        "properties": {
            "id": {"type": "integer", "description": "The key", "x-attribute-id": 1},
            "customer": {"$ref": "#/$defs/customer", "x-attribute-id": 3, "x-entity": "buyer"},
            "audit": {
                "type": "object",
                "x-jlens-hidden": true,
                "properties": {"by": {"type": "string"}}
            },
            "carrier": {"$ref": "carrier.json", "description": "Who ships it"},
            "code": {"type": ["string", "null"], "x-jlens-domain": "codes"},
            "odd name": {"anyOf": [{"type": "number"}, {"type": "string"}]}
        }
    })
}

/// The case of `fixtures/fields/cases.json`, which every package lists with the same JSON.
#[test]
fn the_shared_case() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/fields/cases.json"
    );
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let (schema, _) = SchemaLoader::new().load(&fixture["schema"]).unwrap();
    let service = jlens_schema::LanguageService::new(Some(schema));
    assert_eq!(
        serde_json::to_value(service.fields()).unwrap(),
        fixture["fields"]
    );
    assert!(jlens_schema::LanguageService::new(None).fields().is_empty());
}

#[test]
fn each_field_with_what_completion_and_hover_tell() {
    let (schema, _) = SchemaLoader::new().load(&order()).unwrap();
    let fields = listed(&schema);
    assert_eq!(
        fields.keys().collect::<Vec<_>>(),
        [
            "$",
            "`odd name`",
            "carrier",
            "code",
            "customer",
            "customer.name",
            "id"
        ]
    );
    let root = &fields["$"];
    assert_eq!(
        (root.label.as_deref(), root.required),
        (Some("Order"), None)
    );
    assert_eq!(root.extensions["x-entity"], "order");

    let id = &fields["id"];
    assert_eq!(id.types, [JsonType::Integer]);
    assert_eq!(
        (id.required, id.description.as_deref()),
        (Some(true), Some("The key"))
    );
    assert_eq!(id.extensions["x-attribute-id"], 1);

    // The place's own keys first, then the definition's; its label is the definition's.
    let customer = &fields["customer"];
    assert_eq!(customer.label.as_deref(), Some("Customer"));
    assert_eq!(
        serde_json::to_value(&customer.extensions).unwrap(),
        json!({"x-attribute-id": 3, "x-entity": "buyer"})
    );
    assert_eq!(fields["customer.name"].required, Some(true));

    // A `$ref` not followed: unknown, but what its place says is still listed.
    let carrier = &fields["carrier"];
    assert_eq!(
        (carrier.type_summary.as_str(), carrier.types.len()),
        ("unknown", 0)
    );
    assert_eq!(carrier.description.as_deref(), Some("Who ships it"));

    let code = &fields["code"];
    assert_eq!(code.type_summary, "string | null");
    assert_eq!(code.domain.as_deref(), Some("codes"));
    assert_eq!(fields["`odd name`"].type_summary, "number | string");
}

#[test]
fn overrides_are_in_and_hidden_fields_are_out() {
    let mut schema = Schema::from_json_schema(&order()).unwrap();
    let overrides: BTreeMap<String, Override> = serde_json::from_value(json!({
        "id": {"label": "Order number"},
        "code": {"hidden": true},
        "audit": {"hidden": false},
        "items[].sku": {"type": "string", "enum": ["a", "b"]}
    }))
    .unwrap();
    assert!(schema.apply_overrides(&overrides).is_empty());
    let fields = listed(&schema);
    assert_eq!(fields["id"].label.as_deref(), Some("Order number"));
    assert!(!fields.contains_key("code"));
    assert!(fields.contains_key("audit.by"));
    let sku = &fields["items[].sku"];
    assert_eq!(sku.enumeration, Some(vec![json!("a"), json!("b")]));
    assert!(sku.closed);
    assert_eq!(fields["items"].type_summary, "array<object>");
}

#[test]
fn a_recursive_schema_down_to_where_it_repeats() {
    let schema = Schema::from_json_schema(&json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "children": {"type": "array", "items": {"$ref": "#"}}
        }
    }))
    .unwrap();
    let paths: Vec<String> = schema.fields().into_iter().map(|f| f.path).collect();
    assert_eq!(paths, ["$", "children", "children[]", "name"]);
}

#[test]
fn fields_required_and_not_listed() {
    let schema = Schema::from_json_schema(&json!({
        "type": "object",
        "properties": {"a": {"type": "string"}},
        "allOf": [{"required": ["a", "b"]}]
    }))
    .unwrap();
    let fields = listed(&schema);
    assert_eq!(fields["a"].required, Some(true));
    assert_eq!(
        (fields["b"].required, fields["b"].type_summary.as_str()),
        (Some(true), "unknown")
    );
}

#[test]
fn the_keywords_of_an_application_survive_a_round_trip() {
    let document = json!({
        "type": "object",
        "x-entity": {"name": "order", "version": 2},
        "properties": {"id": {"type": "string", "x-attribute-id": "a1", "x-jlens-other": true}}
    });
    let schema = Schema::from_json_schema(&document).unwrap();
    assert_eq!(schema.root.extensions["x-entity"]["version"], 2);
    // Those of jlens that it does not read are kept too; those it reads are not extensions.
    let id = &schema.root.fields["id"].shape;
    assert_eq!(
        id.extensions.keys().collect::<Vec<_>>(),
        ["x-attribute-id", "x-jlens-other"]
    );
    let exported = schema.to_json_schema();
    assert_eq!(exported["properties"]["id"]["x-attribute-id"], "a1");
    assert_eq!(Schema::from_json_schema(&exported).unwrap(), schema);
    let hidden = Schema::from_json_schema(&json!({"x-jlens-hidden": true, "x-a": 1})).unwrap();
    assert_eq!(hidden.root.extensions.keys().collect::<Vec<_>>(), ["x-a"]);
    // The JSON form of a listed field reads back.
    for field in schema.fields() {
        let value = serde_json::to_value(&field).unwrap();
        assert_eq!(serde_json::from_value::<ListedField>(value).unwrap(), field);
    }
}

/// The example of the schema guide, and of the WebAssembly and Java guides.
#[test]
fn the_example_of_the_guides() {
    let schema = Schema::from_json_schema(&json!({
        "type": "object",
        "required": ["status"],
        "properties": {
            "status": {"type": "string", "title": "Status", "enum": ["open", "closed"],
                       "x-attribute-id": "A17"},
            "lines": {"type": "array", "items": {"type": "object", "properties": {
                "sku": {"type": "string", "x-jlens-domain": "products"}}}},
            "internal": {"type": "string", "x-jlens-hidden": true}
        }
    }))
    .unwrap();
    assert_eq!(
        serde_json::to_value(schema.fields()).unwrap(),
        json!([
            {"path": "$", "typeSummary": "object", "types": ["object"]},
            {"path": "lines", "typeSummary": "array<object>", "types": ["array"], "required": false},
            {"path": "lines[]", "typeSummary": "object", "types": ["object"]},
            {"path": "lines[].sku", "typeSummary": "string", "types": ["string"], "required": false,
             "domain": "products"},
            {"path": "status", "typeSummary": "string", "types": ["string"], "required": true,
             "label": "Status", "enum": ["open", "closed"], "closed": true,
             "extensions": {"x-attribute-id": "A17"}}
        ])
    );
}
