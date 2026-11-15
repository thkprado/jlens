//! The items of a tuple (`prefixItems`, or `items` as an array until draft-07) read by a
//! literal index: `point[1]` has the type of its position ([`Shape::prefix_items`]).
//!
//! [`Shape::prefix_items`]: jlens_schema::Shape::prefix_items

use std::collections::BTreeMap;

use jlens_schema::{CompletionKind, JsonType, LanguageService, Override, Schema};
use serde_json::{Value, json};

/// A point of two numbers that any item may follow (draft-07), an entry closed to its two
/// items, lines whose first item is a string and the rest numbers, rows that are tuples, and a
/// grid of arrays of cells that each have a pair.
fn document() -> Value {
    let pair = json!({"type": "array", "prefixItems": [{"type": "string"}, {"type": "number"}]});
    json!({
        "type": "object",
        "required": ["point", "entry", "lines", "rows", "grid"],
        "properties": {
            "point": {"type": "array", "items": [{"type": "number"}, {"type": "number"}]},
            "entry": {
                "type": "array",
                "prefixItems": [
                    {"type": "string"},
                    {
                        "type": "object",
                        "required": ["id"],
                        "properties": {
                            "id": {"type": "integer"},
                            "status": {"enum": ["open", "closed"]}
                        },
                        "additionalProperties": false
                    }
                ],
                "items": false
            },
            "lines": {
                "type": "array",
                "prefixItems": [{"type": "string"}],
                "items": {"type": "number"}
            },
            "rows": {"type": "array", "items": pair},
            "grid": {
                "type": "array",
                "items": {
                    "type": "array",
                    "items": {"type": "object", "properties": {"pair": pair}}
                }
            }
        }
    })
}

fn service() -> LanguageService {
    LanguageService::new(Some(Schema::from_json_schema(&document()).unwrap()))
}

fn codes(service: &LanguageService, text: &str) -> Vec<String> {
    service
        .diagnostics(text)
        .into_iter()
        .map(|d| d.code.to_string())
        .collect()
}

fn result(service: &LanguageService, text: &str) -> Option<Vec<JsonType>> {
    service.result_type(text).types
}

#[test]
fn a_literal_index_has_the_type_of_its_position() {
    let service = service();
    let number = Some(vec![JsonType::Number]);
    let string = Some(vec![JsonType::String]);
    // An open tuple: its items are not known, its positions are.
    assert_eq!(result(&service, "point"), Some(vec![JsonType::Array]));
    assert_eq!(result(&service, "point[1]"), number);
    assert_eq!(codes(&service, r#"point[1] > "a""#), ["W0208"]);
    assert!(codes(&service, "point[0] + point[1]").is_empty());
    // A closed one, and one with a schema for the rest.
    assert_eq!(result(&service, "entry[0]"), string);
    assert_eq!(result(&service, "entry[1]"), Some(vec![JsonType::Object]));
    assert_eq!(codes(&service, "entry[0] + 1"), ["W0209"]);
    assert_eq!(result(&service, "lines[0]"), string);
    assert_eq!(codes(&service, "lines[0] > 1"), ["W0208"]);
    // The context of an expression, a variable and a block are read the same way.
    assert_eq!(result(&service, "$.point[0]"), number);
    assert_eq!(result(&service, "($p := point; $p[1])"), number);
    assert_eq!(result(&service, "(entry)[0]"), string);
    // The body of a step over the entry is evaluated against each of its items.
    let body = service.clone().with_context_path("entry");
    assert_eq!(
        result(&body, "$[0]"),
        Some(vec![JsonType::String, JsonType::Object])
    );
}

#[test]
fn other_indexes_have_the_type_of_every_item() {
    let service = service();
    // Past the tuple, from the end, and computed: any item of an open tuple is unknown.
    for text in [
        r#"point[2] > "a""#,
        r#"point[-1] > "a""#,
        r#"point[$i] > "a""#,
        r#"point[1.5] > "a""#,
        r#"point[$ > 1][0] > "a""#,
    ] {
        assert!(
            codes(&service, text).is_empty(),
            "{text}: {:?}",
            codes(&service, text)
        );
    }
    assert_eq!(result(&service, "point[2]"), None);
    // A filter after the index keeps the item or not.
    assert_eq!(codes(&service, r#"point[1][true] > "a""#), ["W0208"]);
    // The items after the first of `lines` are numbers, which the type of every item has
    // beside the string of the first.
    let either = Some(vec![JsonType::Number, JsonType::String]);
    assert_eq!(result(&service, "lines[1]"), either);
    assert_eq!(result(&service, "lines[-1]"), either);
    assert!(codes(&service, "lines[1] > 1").is_empty());
    assert_eq!(
        result(&service, "entry[-1]"),
        Some(vec![JsonType::String, JsonType::Object])
    );
}

#[test]
fn an_index_after_an_index_reads_the_item() {
    let service = service();
    // The second row, then its second item.
    assert_eq!(result(&service, "rows[1][1]"), Some(vec![JsonType::Number]));
    assert_eq!(codes(&service, r#"rows[0][1] > "a""#), ["W0208"]);
    assert_eq!(codes(&service, "rows[0][0] + 1"), ["W0209"]);
    assert!(codes(&service, "rows[0][1] + 1").is_empty());
    assert_eq!(
        result(&service, "($r := rows; $r[0][0])"),
        Some(vec![JsonType::String])
    );
    // A step over the rows reads every item of every row.
    assert_eq!(result(&service, "rows[1]"), Some(vec![JsonType::Array]));
    assert_eq!(result(&service, "rows[0]"), Some(vec![JsonType::Array]));
    // A cell of the grid is inside two arrays: `pair` is read of every cell of a row, and
    // the index is one into the items of them all.
    assert!(codes(&service, r#"grid.pair[1] > "a""#).is_empty());
    assert!(codes(&service, "grid.pair[0] > 1").is_empty());
}

/// A predicate after a computed index reads the item the index selects where the index is a
/// number, the same for every item, as after a literal one (jsonata-js#861, after 2.2.2), and
/// the sequence it keeps otherwise: an index the typing cannot tell is either.
#[test]
fn an_index_after_a_computed_index_reads_the_item_it_selects() {
    let shape = |schema: Value| Some(Schema::from_json_schema(&schema).unwrap().root);
    let numbers = json!({"type": "array", "items": {"type": "number"}});
    let schema = json!({
        "type": "object",
        "required": ["rows"],
        "properties": {"rows": {"type": "array", "items": numbers}}
    });
    let service = LanguageService::new(Some(Schema::from_json_schema(&schema).unwrap()))
        .with_variable("arr", shape(json!({"type": "array", "items": numbers})))
        .with_variable("k", shape(json!({"type": "integer"})))
        .with_variable("i", None);
    let number = Some(vec![JsonType::Number]);
    let row = Some(vec![JsonType::Array]);
    let either = Some(vec![JsonType::Number, JsonType::Array]);
    // `$k` is a number: a row, then an item of it.
    assert_eq!(result(&service, "$arr[$k][0]"), number);
    assert_eq!(result(&service, "rows[$k][0]"), number);
    assert_eq!(result(&service, "rows[$k + 1][0]"), number);
    assert!(codes(&service, "rows[$k][0] + 1").is_empty());
    assert_eq!(codes(&service, r#"rows[$k][0] > "a""#), ["W0208"]);
    // A test keeps rows, and the index takes the first of them.
    assert_eq!(result(&service, "rows[$[0] > 1][0]"), row);
    assert_eq!(result(&service, "rows[true][0]"), row);
    // `$i` may be anything: a row, or an item of one.
    assert_eq!(result(&service, "rows[$i][0]"), either);
    assert_eq!(result(&service, "$arr[$i][0]"), either);
}

#[test]
fn completion_and_hover_follow_the_position() {
    let service = service();
    let labels = |text: &str| -> Vec<String> {
        let completions = service.complete(text, text.len());
        (completions.items.into_iter())
            .filter(|i| i.kind == CompletionKind::Field)
            .map(|i| i.label)
            .collect()
    };
    assert_eq!(labels("entry[1]."), ["id", "status"]);
    assert_eq!(labels("entry[1][id > 1]."), ["id", "status"]);
    // The first item is a string, which has no field.
    assert!(labels("entry[0].").is_empty());
    assert_eq!(codes(&service, "entry[1].nme"), ["W0201"]);
    assert!(codes(&service, "entry[1].id + 1").is_empty());
    // The values of a field of the position.
    let text = "entry[1].status = ";
    let values: Vec<String> = (service.complete(text, text.len()).items.into_iter())
        .filter(|i| i.kind == CompletionKind::Value)
        .map(|i| i.label)
        .collect();
    assert_eq!(values, ["\"open\"", "\"closed\""]);
    assert_eq!(codes(&service, r#"entry[1].status = "done""#), ["W0206"]);
    let text = "entry[1].status";
    let hover = service.hover(text, text.len() - 2).unwrap();
    assert!(
        hover.contents.starts_with("`status`: "),
        "{}",
        hover.contents
    );
    let hover = service.hover("entry[1].id", 10).unwrap();
    assert!(
        hover.contents.starts_with("`id`: integer"),
        "{}",
        hover.contents
    );
}

#[test]
fn positions_of_branches_and_references() {
    let load =
        |document: Value| LanguageService::new(Some(Schema::from_json_schema(&document).unwrap()));
    let pair = |first: &str, second: &str| json!({"type": "array", "prefixItems": [{"type": first}, {"type": second}]});
    // One branch or another: a position has the type of either.
    let service = load(json!({"properties": {"p": {"anyOf": [
        pair("number", "string"), pair("number", "boolean")
    ]}}}));
    assert_eq!(result(&service, "p[0]"), Some(vec![JsonType::Number]));
    assert_eq!(
        result(&service, "p[1]"),
        Some(vec![JsonType::Boolean, JsonType::String])
    );
    // A branch that is an array of anything tells no position; one of numbers tells them all.
    let service = load(json!({"properties": {
        "p": {"anyOf": [pair("number", "string"), {"type": "array"}]},
        "q": {"anyOf": [pair("number", "string"), {"type": "array", "items": {"type": "number"}}]}
    }}));
    assert_eq!(result(&service, "p[0]"), None);
    assert_eq!(result(&service, "q[0]"), Some(vec![JsonType::Number]));
    assert_eq!(
        result(&service, "q[1]"),
        Some(vec![JsonType::Number, JsonType::String])
    );
    // A tuple by reference, recursive too, and one that every branch of `allOf` describes.
    let service = load(json!({
        "properties": {
            "p": {"$ref": "#/$defs/pair"},
            "tree": {"$ref": "#/$defs/tree"},
            "both": {"allOf": [pair("number", "string"), {"minItems": 2}]}
        },
        "$defs": {
            "pair": pair("string", "number"),
            "tree": {"type": "array", "prefixItems": [
                {"type": "string"},
                {"type": "array", "items": {"$ref": "#/$defs/tree"}}
            ]}
        }
    }));
    assert_eq!(result(&service, "p[1]"), Some(vec![JsonType::Number]));
    assert_eq!(result(&service, "tree[0]"), Some(vec![JsonType::String]));
    assert_eq!(
        result(&service, "tree[1][0][0]"),
        Some(vec![JsonType::String])
    );
    assert_eq!(result(&service, "both[1]"), Some(vec![JsonType::String]));
}

#[test]
fn positions_survive_a_round_trip_and_take_overrides() {
    let mut schema = Schema::from_json_schema(&document()).unwrap();
    let exported = schema.to_json_schema();
    assert_eq!(
        exported["properties"]["point"],
        json!({"type": "array", "prefixItems": [{"type": "number"}, {"type": "number"}]})
    );
    assert_eq!(
        exported["properties"]["lines"]["items"]["type"],
        json!(["number", "string"])
    );
    assert_eq!(Schema::from_json_schema(&exported).unwrap(), schema);
    // The serialized form names them as JSON Schema does.
    let shape = serde_json::to_value(schema.shape_at("point").unwrap()).unwrap();
    assert_eq!(shape["prefixItems"][1]["types"], json!(["number"]));
    assert_eq!(shape.get("items"), None);
    // An override of the items of a tuple is one of each position it leads to, too.
    let overrides: BTreeMap<String, Override> = serde_json::from_value(json!({
        "entry[].status": {"enum": ["open", "closed", "held"]},
        "entry[].note": {"type": "string"},
        "rows[][]": {"description": "A cell"}
    }))
    .unwrap();
    assert!(schema.apply_overrides(&overrides).is_empty());
    let entry = schema.shape_at("entry").unwrap();
    // The first item is a string: it has no `status` to override, and gets the new field.
    assert!(!entry.prefix_items[0].fields.contains_key("status"));
    assert!(entry.prefix_items[0].fields.contains_key("note"));
    assert_eq!(
        entry.prefix_items[1].fields["status"]
            .shape
            .enumeration
            .as_ref()
            .map(Vec::len),
        Some(3)
    );
    let row = schema.shape_at("rows[]").unwrap();
    assert_eq!(row.prefix_items[1].description.as_deref(), Some("A cell"));
    let service = LanguageService::new(Some(schema));
    assert!(codes(&service, r#"entry[1].status = "held""#).is_empty());
    assert!(codes(&service, "entry[1].note & entry.note").is_empty());
}
