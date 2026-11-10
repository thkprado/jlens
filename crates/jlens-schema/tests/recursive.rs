//! A recursive JSON Schema: its shapes are followed as deep as an expression goes.

use std::collections::BTreeMap;

use jlens_core::Expression;
use jlens_schema::{LanguageService, Override, Schema};
use proptest::prelude::*;
use serde_json::{Value, json};

/// A tree whose nodes are the document itself.
fn tree() -> Value {
    json!({
        "type": "object",
        "required": ["name"],
        "properties": {
            "name": {"type": "string"},
            "kind": {"type": "string", "enum": ["leaf", "branch"]},
            "children": {"type": "array", "items": {"$ref": "#"}}
        }
    })
}

/// A linked list through a definition, with the forms a reference takes beside other keywords.
fn list() -> Value {
    json!({
        "type": "object",
        "properties": {
            "head": {"$ref": "#/$defs/node", "description": "The first node"},
            "plain": {"$ref": "#/$defs/point"}
        },
        "$defs": {
            "point": {"type": "object", "properties": {"x": {"type": "number"}}},
            "base": {
                "type": "object",
                "properties": {"id": {"type": "integer"}, "parent": {"$ref": "#/$defs/node"}}
            },
            "node": {
                "allOf": [{"$ref": "#/$defs/base"}],
                "type": "object",
                "description": "A node",
                "properties": {
                    "value": {"type": "number"},
                    "next": {"anyOf": [{"$ref": "#/$defs/node"}, {"type": "null"}]}
                }
            }
        }
    })
}

fn service(doc: &Value) -> LanguageService {
    LanguageService::new(Some(Schema::from_json_schema(doc).unwrap()))
}

fn codes(service: &LanguageService, text: &str) -> Vec<String> {
    service
        .diagnostics(text)
        .into_iter()
        .map(|d| d.code.to_string())
        .collect()
}

fn labels(service: &LanguageService, text: &str) -> Vec<String> {
    service
        .complete(text, text.len())
        .items
        .into_iter()
        .filter(|i| i.kind == jlens_schema::CompletionKind::Field)
        .map(|i| i.label)
        .collect()
}

fn uses(service: &LanguageService, text: &str) -> Value {
    let reads = Expression::compile(text).unwrap().reads();
    serde_json::to_value(service.uses(&reads)).unwrap()
}

#[test]
fn the_document_as_its_own_items() {
    let schema = Schema::from_json_schema(&tree()).unwrap();
    assert_eq!(schema.root.references, ["#"]);
    assert_eq!(schema.definitions.len(), 1);
    let service = LanguageService::new(Some(schema));
    let deep = "children.".repeat(40);
    assert_eq!(codes(&service, &format!("{deep}name")), [] as [&str; 0]);
    assert_eq!(codes(&service, &format!("{deep}nane")), ["W0201"]);
    assert_eq!(codes(&service, "nane"), ["W0201"]);
    assert_eq!(
        codes(&service, &format!("{deep}children[kind = 'leef']")),
        ["W0206"]
    );
    assert_eq!(labels(&service, &deep), ["name", "kind", "children"]);
    assert_eq!(labels(&service, ""), ["name", "kind", "children"]);
    let hover = service.hover("children.children", 10).unwrap();
    assert_eq!(hover.contents, "`children`: array<object>");
    // Every value below, wherever it is.
    assert_eq!(codes(&service, "**.name"), [] as [&str; 0]);
    assert_eq!(codes(&service, "**.nane"), ["W0201"]);
    assert_eq!(codes(&service, "children.*.nane"), ["W0201"]);
}

#[test]
fn a_definition_that_refers_to_itself() {
    let schema = Schema::from_json_schema(&list()).unwrap();
    // Only what is recursive is a definition: the others are copied in place, as before.
    assert_eq!(
        schema.definitions.keys().collect::<Vec<_>>(),
        ["node", "base"]
    );
    assert!(schema.root.fields["plain"].shape.references.is_empty());
    assert!(schema.shape_at("plain.x").is_some());
    let service = LanguageService::new(Some(schema.clone()));
    // A node has the fields of what it extends and its own, at any depth.
    assert_eq!(
        labels(&service, "head.next.next.parent."),
        ["value", "next", "id", "parent"]
    );
    assert_eq!(
        codes(&service, "head.next.next.parent.next.valu"),
        ["W0201"]
    );
    assert_eq!(
        codes(&service, "head.next.next.value + head.parent.id"),
        [] as [&str; 0]
    );
    // The description beside the reference is the field's; the definition gives the rest.
    let hover = service.hover("head", 1).unwrap();
    assert_eq!(hover.contents, "`head`: object\n\nThe first node");
    let hover = service.hover("head.next", 6).unwrap();
    assert_eq!(hover.contents, "`next`: object | null\n\nA node");
    assert!(schema.shape_at("head.next.next.parent.id").is_some());
    assert!(schema.shape_at("head.next.missing").is_none());
}

#[test]
fn definitions_that_refer_to_each_other() {
    let service = service(&json!({
        "$ref": "#/definitions/a",
        "definitions": {
            "a": {"type": "object", "properties": {"b": {"$ref": "#/definitions/b"}}},
            "b": {"type": "object", "properties": {
                "a": {"$ref": "#/definitions/a"},
                "n": {"type": "number"}
            }},
            // References that lead to no shape leave their place unknown.
            "loop": {"$ref": "#/definitions/pool"},
            "pool": {"anyOf": [{"$ref": "#/definitions/loop"}]},
            "holder": {"type": "object", "properties": {"x": {"$ref": "#/definitions/loop"}}}
        }
    }));
    assert_eq!(codes(&service, "b.a.b.a.b.n"), [] as [&str; 0]);
    assert_eq!(codes(&service, "b.a.b.a.n"), ["W0201"]);
    let unknown = LanguageService::new(Some(
        Schema::from_json_schema(&json!({
            "$ref": "#/definitions/holder",
            "definitions": {
                "loop": {"$ref": "#/definitions/pool"},
                "pool": {"anyOf": [{"$ref": "#/definitions/loop"}]},
                "holder": {"type": "object", "properties": {"x": {"$ref": "#/definitions/loop"}}}
            }
        }))
        .unwrap(),
    ));
    assert_eq!(codes(&unknown, "x.anything.at.all"), [] as [&str; 0]);
    assert_eq!(codes(&unknown, "y"), ["W0201"]);
    assert_eq!(
        uses(&unknown, "x.a")["input"],
        json!({
            "$": {"types": ["object"]},
            "x": {"unknown": true},
            "x.a": {"unknown": true, "whole": true}
        })
    );
}

#[test]
fn what_an_expression_uses_of_it() {
    let service = service(&tree());
    assert_eq!(
        uses(&service, "children.children[kind = 'leaf'].name")["input"],
        json!({
            "$": {"types": ["object"]},
            "children": {"types": ["array"], "required": false},
            "children[]": {"types": ["object"]},
            "children[].children": {"types": ["array"], "required": false},
            "children[].children[]": {"types": ["object"]},
            "children[].children[].kind": {
                "whole": true, "types": ["string"], "enum": ["leaf", "branch"], "closed": true,
                "required": false
            },
            "children[].children[].name": {"whole": true, "types": ["string"], "required": true}
        })
    );
    // A value read whole is listed down to where the schema repeats.
    assert_eq!(
        uses(&service, "$string($)")["input"],
        json!({
            "$": {"whole": true, "types": ["object"]},
            "children": {"types": ["array"], "required": false},
            "children[]": {"types": ["object"]},
            "kind": {
                "types": ["string"], "enum": ["leaf", "branch"], "closed": true,
                "required": false
            },
            "name": {"types": ["string"], "required": true}
        })
    );
    let list = LanguageService::new(Some(Schema::from_json_schema(&list()).unwrap()));
    let all = uses(&list, "head.next.*")["input"].clone();
    assert_eq!(
        all.as_object().unwrap().keys().collect::<Vec<_>>(),
        [
            "$",
            "head",
            "head.next",
            "head.next.id",
            "head.next.next",
            "head.next.parent",
            "head.next.value"
        ]
    );
    assert_eq!(
        all["head.next"],
        json!({"whole": true, "types": ["null", "object"], "required": false})
    );
    // A change inside the definition shows in what is listed.
    let mut changed = self::list();
    changed["$defs"]["node"]["properties"]["value"]["type"] = json!("string");
    let changed = LanguageService::new(Some(Schema::from_json_schema(&changed).unwrap()));
    let reads = Expression::compile("head.next.*").unwrap().reads();
    let changes = serde_json::to_value(list.uses(&reads).changes(&changed.uses(&reads))).unwrap();
    assert_eq!(
        changes["input"]["changed"],
        json!({"head.next.value": {"types": {"old": ["number"], "new": ["string"]}}})
    );
}

#[test]
fn written_back_and_read_again() {
    for doc in [tree(), list()] {
        let schema = Schema::from_json_schema(&doc).unwrap();
        let written = schema.to_json_schema();
        let again = Schema::from_json_schema(&written).unwrap();
        assert_eq!(again, schema, "{written}");
        assert_eq!(again.to_json_schema(), written);
    }
    let written = Schema::from_json_schema(&tree()).unwrap().to_json_schema();
    assert_eq!(written["$ref"], "#/$defs/#");
    assert_eq!(
        written["$defs"]["#"]["properties"]["children"]["items"],
        json!({"$ref": "#/$defs/#"})
    );
}

#[test]
fn overrides_and_variables() {
    let mut schema = Schema::from_json_schema(&tree()).unwrap();
    let overrides: BTreeMap<String, Override> = serde_json::from_value(json!({
        "children[].children[].name": {"enum": ["a", "b"]},
        "children[].weight": {"type": "number"}
    }))
    .unwrap();
    let missing = schema.missing_override_paths(&overrides);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].path, "children[].weight");
    assert!(schema.apply_overrides(&overrides).is_empty());
    // The definition changed: every node has what the overrides say.
    assert!(schema.shape_at("weight").is_some());
    assert!(
        schema
            .shape_at("children[].children[].children[].weight")
            .is_some()
    );
    let service =
        LanguageService::new(Some(Schema::default())).with_variable_schema("tree", schema.clone());
    assert_eq!(codes(&service, "$tree.children.name = 'c'"), ["W0206"]);
    assert_eq!(codes(&service, "$tree.children.children.wait"), ["W0201"]);
    assert_eq!(
        uses(&service, "$tree.children.weight")["bindings"]["tree"]["children[].weight"],
        json!({"whole": true, "types": ["number"], "required": false})
    );
    // A shape alone has no definitions: what it refers to is unknown.
    let alone =
        LanguageService::new(Some(Schema::default())).with_variable("tree", Some(schema.root));
    assert_eq!(codes(&alone, "$tree.children.wait"), [] as [&str; 0]);
}

#[test]
fn many_definitions_that_all_refer_to_each_other() {
    let names: Vec<String> = (0..60).map(|i| format!("d{i}")).collect();
    let mut definitions = serde_json::Map::new();
    for name in &names {
        let properties: serde_json::Map<String, Value> = names
            .iter()
            .map(|other| (other.clone(), json!({"$ref": format!("#/$defs/{other}")})))
            .collect();
        definitions.insert(
            name.clone(),
            json!({"type": "object", "properties": properties}),
        );
    }
    let doc = json!({"$ref": "#/$defs/d0", "$defs": definitions});
    let schema = Schema::from_json_schema(&doc).unwrap();
    assert_eq!(schema.definitions.len(), 60);
    let service = LanguageService::new(Some(schema));
    assert_eq!(codes(&service, "d1.d2.d3.d59.d0.d7"), [] as [&str; 0]);
    assert_eq!(codes(&service, "d1.d2.d3.d59.d0.d77"), ["W0201"]);
    assert_eq!(codes(&service, "**.d77"), ["W0201"]);
    // Read whole, what is under each definition is listed once.
    let listed = uses(&service, "d1.d2")["input"].as_object().unwrap().len();
    assert_eq!(listed, 3 + 60 * 60);
}

/// A schema of a few definitions whose places refer to each other at random.
fn document() -> impl Strategy<Value = Value> {
    let target = prop::sample::select(vec![
        "#",
        "#/$defs/a",
        "#/$defs/b",
        "#/$defs/c",
        "#/$defs/a/properties/x",
        "#/properties/p/items",
        "#/$defs/missing",
    ]);
    let place = (target, 0..7u8).prop_map(|(target, form)| match form {
        0 => json!({"$ref": target}),
        1 => json!({"$ref": target, "description": "d", "type": "object"}),
        2 => json!({"anyOf": [{"$ref": target}, {"type": "null"}]}),
        3 => json!({"type": "array", "items": {"$ref": target}}),
        4 => json!({"allOf": [{"$ref": target}], "properties": {"own": {"type": "string"}}}),
        5 => json!({"type": "string", "enum": ["a", "b"]}),
        _ => json!({"type": "array", "items": {"type": "array", "items": {"$ref": target}}}),
    });
    prop::collection::vec(place, 9).prop_map(|p| {
        json!({
            "type": "object",
            "properties": {"p": p[0], "q": p[1], "r": p[2]},
            "$defs": {
                "a": {"type": "object", "properties": {"x": p[3], "y": p[4]}},
                "b": p[5],
                "c": {"oneOf": [p[6], p[7]], "properties": {"z": p[8]}}
            }
        })
    })
}

fn expression() -> impl Strategy<Value = String> {
    let step = prop::sample::select(vec![
        "p",
        "q",
        "r",
        "x",
        "y",
        "z",
        "own",
        "*",
        "**",
        "p[x = 'a']",
        "$",
        "q.y",
        "nope",
    ]);
    prop::collection::vec(step, 1..12).prop_map(|steps| steps.join("."))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// Whatever refers to whatever, the schema loads or fails with a message, and the service
    /// answers: no stack overflow, no endless walk.
    #[test]
    fn a_schema_of_references_never_hangs(doc in document(), text in expression()) {
        let Ok(schema) = Schema::from_json_schema(&doc) else {
            return Ok(());
        };
        // What is written back reads as the same schema, but for the definitions that
        // nothing refers to, which are not written.
        let again = Schema::from_json_schema(&schema.to_json_schema()).unwrap();
        let twice = Schema::from_json_schema(&again.to_json_schema()).unwrap();
        prop_assert_eq!(&twice, &again);
        let service = LanguageService::new(Some(schema.clone()))
            .with_variable_schema("v", schema.clone());
        for d in service.diagnostics(&text) {
            prop_assert!(d.span.end <= text.len());
        }
        service.complete(&format!("{text}."), text.len() + 1);
        service.hover(&text, text.len() / 2);
        let reads = Expression::compile(&text).unwrap().reads();
        let uses = service.uses(&reads);
        // Every path listed is one the schema reads, unless it says nothing of it.
        for (path, used) in uses.input.iter().flatten() {
            prop_assert!(used.unknown || schema.shape_at(path).is_some(), "{}", path);
        }
        let whole = Expression::compile("$eval('1')").unwrap().reads();
        prop_assert!(service.uses(&whole).input.is_some());
    }
}
