//! A JSON Schema whose `$ref`s lead to other documents, to an `$id` or to an `$anchor`; the
//! report of those that lead nowhere; and a load bounded by the size of its schema.

use jlens_core::Expression;
use jlens_schema::{LanguageService, LoadReport, Schema, SchemaLoader};
use proptest::prelude::*;
use serde_json::{Value, json};

fn codes(service: &LanguageService, text: &str) -> Vec<String> {
    service
        .diagnostics(text)
        .into_iter()
        .map(|d| d.code.to_string())
        .collect()
}

fn uses(service: &LanguageService, text: &str) -> Value {
    let reads = Expression::compile(text).unwrap().reads();
    serde_json::to_value(service.uses(&reads)).unwrap()
}

fn order() -> Value {
    json!({
        "$id": "https://example.com/schemas/order.json",
        "type": "object",
        "properties": {
            "customer": {"$ref": "common.json#/$defs/person"},
            "billing": {"$ref": "/schemas/common.json#address"},
            "carrier": {"$ref": "https://carriers.example/carrier.json"},
            "items": {"type": "array", "items": {"$ref": "#/$defs/item"}}
        },
        "$defs": {
            "item": {
                "type": "object",
                "properties": {"sku": {"type": "string"}, "origin": {"$ref": "common.json#address"}}
            }
        }
    })
}

fn common() -> Value {
    json!({
        "$defs": {
            "person": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "address": {"$ref": "#/$defs/address"},
                    "manager": {"$ref": "#/$defs/person"}
                }
            },
            "address": {
                "$anchor": "address",
                "type": "object",
                "properties": {"city": {"type": "string"}, "country": {"$ref": "countries.json"}}
            }
        }
    })
}

#[test]
fn a_document_given_by_its_uri() {
    let (common, countries) = (common(), json!({"type": "string", "enum": ["BR", "PT"]}));
    let loader = SchemaLoader::new()
        .document("https://example.com/schemas/common.json", &common)
        .document("https://example.com/schemas/countries.json", &countries);
    let (schema, report) = loader.load(&order()).unwrap();
    // The carrier's document was not given: the rest is read.
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        json!({"unresolved": [{
            "place": "#/properties/carrier",
            "reference": "https://carriers.example/carrier.json",
            "uri": "https://carriers.example/carrier.json",
            "document": "https://carriers.example/carrier.json"
        }]})
    );
    assert_eq!(
        report.documents(),
        ["https://carriers.example/carrier.json"]
    );
    let service = LanguageService::new(Some(schema.clone()));
    assert_eq!(codes(&service, "customer.address.city"), [] as [&str; 0]);
    assert_eq!(codes(&service, "customer.address.town"), ["W0201"]);
    assert_eq!(codes(&service, "billing.country = 'XX'"), ["W0206"]);
    assert_eq!(codes(&service, "items.origin.city"), [] as [&str; 0]);
    assert_eq!(codes(&service, "items.origin.town"), ["W0201"]);
    // A person refers to itself in its own document.
    assert_eq!(
        codes(&service, "customer.manager.manager.address.town"),
        ["W0201"]
    );
    // Nothing is known under the carrier, and nothing is warned.
    assert_eq!(codes(&service, "carrier.anything.at.all"), [] as [&str; 0]);
    assert_eq!(codes(&service, "carier"), ["W0201"]);
    assert_eq!(
        uses(&service, "carrier.name")["input"],
        json!({
            "$": {"types": ["object"]},
            "carrier": {"unknown": true},
            "carrier.name": {"unknown": true, "whole": true}
        })
    );

    // With the document, the place is read, and the answer about it changes.
    let carrier = json!({"type": "object", "properties": {"name": {"type": "string"}}});
    let loader = loader.document("https://carriers.example/carrier.json", &carrier);
    let (whole, report) = loader.load(&order()).unwrap();
    assert!(report.is_empty());
    assert_eq!(report, LoadReport::default());
    let whole = LanguageService::new(Some(whole));
    assert_eq!(codes(&whole, "carrier.nane"), ["W0201"]);
    let reads = Expression::compile("carrier.name").unwrap().reads();
    let changes = serde_json::to_value(service.uses(&reads).changes(&whole.uses(&reads))).unwrap();
    assert_eq!(
        changes["input"]["changed"]["carrier.name"]["unknown"],
        json!({"old": true, "new": false})
    );
}

#[test]
fn documents_that_are_missing() {
    // Alone, the schema loads: what it has of its own is checked.
    let (schema, report) = SchemaLoader::new().load(&order()).unwrap();
    assert_eq!(schema, Schema::from_json_schema(&order()).unwrap());
    assert_eq!(
        report.documents(),
        [
            "https://example.com/schemas/common.json",
            "https://carriers.example/carrier.json"
        ]
    );
    let places: Vec<&str> = report.unresolved.iter().map(|u| u.place.as_str()).collect();
    assert_eq!(
        places,
        [
            "#/properties/customer",
            "#/properties/billing",
            "#/properties/carrier",
            "#/$defs/item/properties/origin"
        ]
    );
    let service = LanguageService::new(Some(schema));
    assert_eq!(codes(&service, "items.sku"), [] as [&str; 0]);
    assert_eq!(codes(&service, "items.skew"), ["W0201"]);
    assert_eq!(codes(&service, "items.origin.town"), [] as [&str; 0]);

    // A document that is there, without the place: no document to give.
    let common = common();
    let (schema, report) = SchemaLoader::new()
        .document("https://example.com/schemas/common.json", &common)
        .load_at(
            "https://example.com/schemas/other.json",
            &json!({
                "type": "object",
                "properties": {
                    "a": {"$ref": "common.json#/$defs/nobody"},
                    "b": {"$ref": "common.json#nowhere"},
                    "c": {"$ref": "#/$defs/missing"},
                    "d": {"type": "string"}
                }
            }),
        )
        .unwrap();
    assert_eq!(report.unresolved.len(), 3);
    assert!(report.documents().is_empty());
    assert_eq!(
        report.unresolved[2].place,
        "https://example.com/schemas/other.json#/properties/c"
    );
    assert_eq!(
        report.unresolved[2].uri,
        "https://example.com/schemas/other.json#/$defs/missing"
    );
    let service = LanguageService::new(Some(schema));
    assert_eq!(codes(&service, "a.x & b.y & c.z & d"), [] as [&str; 0]);
    assert_eq!(codes(&service, "e"), ["W0201"]);
}

/// A place that might have the fields of a schema that is missing gets no warning, whatever is
/// written beside the `$ref`.
#[test]
fn nothing_is_warned_beside_a_missing_reference() {
    let schema = Schema::from_json_schema(&json!({
        "type": "object",
        "properties": {
            "beside": {
                "$ref": "base.json",
                "type": "object",
                "properties": {"own": {"type": "string"}}
            },
            "all": {
                "allOf": [{"$ref": "base.json"}, {"properties": {"own": {"type": "string"}}}]
            },
            "any": {"anyOf": [{"$ref": "base.json"}, {"type": "null"}]},
            "list": {"type": "array", "items": {"$ref": "base.json"}},
            "known": {"type": "object", "properties": {"own": {"type": "string"}}}
        }
    }))
    .unwrap();
    let service = LanguageService::new(Some(schema));
    for place in ["beside", "all", "any", "list"] {
        assert_eq!(
            codes(&service, &format!("{place}.inherited")),
            [] as [&str; 0]
        );
    }
    assert_eq!(codes(&service, "known.inherited"), ["W0201"]);
    // What is known beside it is still completed.
    let text = "beside.";
    let labels: Vec<String> = service
        .complete(text, text.len())
        .items
        .into_iter()
        .filter(|i| i.kind == jlens_schema::CompletionKind::Field)
        .map(|i| i.label)
        .collect();
    assert_eq!(labels, ["own"]);
}

#[test]
fn ids_and_anchors() {
    // A document of several schemas, each with its own `$id`: a `$ref` is resolved against
    // the `$id` of the schema it is in, and a pointer starts at that schema.
    let bundle = json!({
        "$id": "https://example.com/root.json",
        "type": "object",
        "properties": {
            "a": {"$ref": "parts/a.json"},
            "b": {"$ref": "https://example.com/parts/b.json#/$defs/inner"},
            "c": {"$ref": "#here"},
            "d": {"$ref": "parts/a.json#local"},
            "old": {"$ref": "#legacy"}
        },
        "$defs": {
            "a": {
                "$id": "parts/a.json",
                "type": "object",
                "properties": {
                    "sibling": {"$ref": "b.json"},
                    "own": {"$ref": "#/$defs/leaf"},
                    "anchored": {"$ref": "#local"}
                },
                "$defs": {
                    "leaf": {"type": "object", "properties": {"leaf": {"type": "number"}}},
                    "other": {
                        "$anchor": "local",
                        "type": "object",
                        "properties": {"local": {"type": "boolean"}}
                    }
                }
            },
            "b": {
                "$id": "https://example.com/parts/b.json",
                "type": "object",
                "properties": {"b": {"type": "string"}},
                "$defs": {
                    "inner": {"type": "object", "properties": {"inner": {"type": "string"}}}
                }
            },
            "c": {
                "$anchor": "here",
                "type": "object",
                "properties": {"here": {"type": "string"}}
            },
            "old": {
                "$id": "#legacy",
                "type": "object",
                "properties": {"legacy": {"type": "string"}}
            }
        }
    });
    let (schema, report) = SchemaLoader::new().load(&bundle).unwrap();
    assert!(report.is_empty(), "{report:?}");
    for path in [
        "a.sibling.b",
        "a.own.leaf",
        "a.anchored.local",
        "b.inner",
        "c.here",
        "d.local",
        "old.legacy",
    ] {
        assert!(schema.shape_at(path).is_some(), "{path}");
    }
    assert!(schema.definitions.is_empty());
    // An anchor belongs to the schema it is in: the root has no `local`.
    let mut wrong = bundle.clone();
    wrong["properties"]["c"]["$ref"] = json!("#local");
    wrong["properties"]["e"] = json!({"$ref": "#/$defs/a/$defs/leaf"});
    wrong["$defs"]["a"]["properties"]["own"]["$ref"] = json!("#/$defs/a");
    let (schema, report) = SchemaLoader::new().load(&wrong).unwrap();
    let missing: Vec<&str> = report.unresolved.iter().map(|u| u.uri.as_str()).collect();
    assert_eq!(
        missing,
        [
            "https://example.com/parts/a.json#/$defs/a",
            "https://example.com/root.json#local"
        ]
    );
    assert!(schema.shape_at("e.leaf").is_some());

    // A document is known by its own `$id` as well as by the URI it was given with, and a
    // property named as a keyword is not one.
    let named = json!({
        "$id": "urn:example:named",
        "type": "object",
        "properties": {
            "$ref": {"type": "string"},
            "$id": {"type": "string"},
            "enum": {"$ref": "#/$defs/v"},
            "default": {"$ref": "#/$defs/v"}
        },
        "$defs": {"v": {"type": "object", "properties": {"v": {"type": "number"}}}}
    });
    let (schema, report) = SchemaLoader::new()
        .document("file:///schemas/named.json", &named)
        .load(&json!({
            "type": "object",
            "properties": {
                "one": {"$ref": "urn:example:named"},
                "two": {"$ref": "file:///schemas/named.json#/properties/enum"}
            }
        }))
        .unwrap();
    assert!(report.is_empty(), "{report:?}");
    for path in [
        "one.`$ref`",
        "one.`$id`",
        "one.enum.v",
        "one.default.v",
        "two.v",
    ] {
        assert!(schema.shape_at(path).is_some(), "{path}");
    }
}

#[test]
fn recursion_through_documents() {
    let node = json!({
        "type": "object",
        "properties": {
            "value": {"type": "number"},
            "children": {"type": "array", "items": {"$ref": "tree.json#/$defs/tree"}}
        }
    });
    let tree = json!({
        "$defs": {"tree": {"anyOf": [{"$ref": "node.json"}, {"type": "null"}]}},
        "type": "object",
        "properties": {"root": {"$ref": "#/$defs/tree"}, "again": {"$ref": "tree.json"}}
    });
    let (schema, report) = SchemaLoader::new()
        .document("node.json", &node)
        .load_at("tree.json", &tree)
        .unwrap();
    assert!(report.is_empty(), "{report:?}");
    let service = LanguageService::new(Some(schema.clone()));
    assert_eq!(
        codes(&service, "root.children.children.children.valu"),
        ["W0201"]
    );
    assert_eq!(
        codes(&service, "again.again.root.children.value"),
        [] as [&str; 0]
    );
    // Written back, it is one document that reads as the same schema.
    let written = schema.to_json_schema();
    assert_eq!(Schema::from_json_schema(&written).unwrap(), schema);
}

#[test]
fn what_is_missing_is_written_back_as_it_was() {
    let doc = json!({
        "type": "object",
        "properties": {
            "a": {"$ref": "other.json#/$defs/a"},
            "b": {"anyOf": [{"$ref": "other.json"}, {"$ref": "#/$defs/none"}, {"type": "null"}]}
        }
    });
    let (schema, report) = SchemaLoader::new().load(&doc).unwrap();
    let written = schema.to_json_schema();
    assert_eq!(
        written["properties"]["a"],
        json!({"$ref": "other.json#/$defs/a"})
    );
    let (again, same) = SchemaLoader::new().load(&written).unwrap();
    assert_eq!(again, schema);
    assert_eq!(same.documents(), report.documents());
    // Read again with the document, the place is known.
    let other =
        json!({"$defs": {"a": {"type": "object", "properties": {"x": {"type": "number"}}}}});
    let (known, _) = SchemaLoader::new()
        .document("other.json", &other)
        .load(&written)
        .unwrap();
    assert!(known.shape_at("a.x").is_some());
}

/// Definitions that each refer twice to the next: copied in place, level `n` is 2ⁿ shapes.
fn doubling(levels: usize) -> Value {
    let mut definitions = serde_json::Map::new();
    for level in 0..levels {
        let next = format!("#/$defs/d{}", level + 1);
        definitions.insert(
            format!("d{level}"),
            json!({"type": "object", "properties": {"l": {"$ref": next}, "r": {"$ref": next}}}),
        );
    }
    definitions.insert(
        format!("d{levels}"),
        json!({"type": "object", "properties": {"leaf": {"type": "string"}}}),
    );
    json!({"$ref": "#/$defs/d0", "$defs": definitions})
}

#[test]
fn a_load_bounded_by_the_size_of_its_schema() {
    // A few levels are copied in place, as always.
    let small = Schema::from_json_schema(&doubling(8)).unwrap();
    assert!(small.definitions.is_empty());
    assert!(small.shape_at("l.r.l.r.l.r.l.r.leaf").is_some());

    // Many are not: the targets that would be copied too often are kept once.
    for levels in [20, 30, 64, 400] {
        let schema = Schema::from_json_schema(&doubling(levels)).unwrap();
        assert!(!schema.definitions.is_empty(), "{levels}");
        assert!(schema.definitions.len() <= levels, "{levels}");
        let service = LanguageService::new(Some(schema.clone()));
        let down = "l.r.".repeat(levels / 2);
        assert_eq!(codes(&service, &format!("{down}leaf")), [] as [&str; 0]);
        assert_eq!(codes(&service, &format!("{down}leef")), ["W0201"]);
        assert_eq!(codes(&service, "l.r.leaf"), ["W0201"]);
        // Written back, it reads as a schema that answers the same.
        let again = Schema::from_json_schema(&schema.to_json_schema()).unwrap();
        assert!(again.shape_at(&format!("{down}leaf")).is_some());
        // Read whole, each definition is listed once.
        let listed = uses(&service, "$string($)")["input"]
            .as_object()
            .unwrap()
            .len();
        assert!(listed < 200_000, "{levels}: {listed}");
    }
}

/// A target inside another, each read by one `$ref` alone, doubles as well.
#[test]
fn targets_inside_each_other() {
    let mut doc = json!({"type": "object", "properties": {"leaf": {"type": "string"}}});
    for depth in (0..40).rev() {
        let inside = format!("#{}", "/properties/in".repeat(depth + 1));
        doc = json!({
            "type": "object",
            "properties": {"in": doc, "ref": {"$ref": inside}}
        });
    }
    let schema = Schema::from_json_schema(&doc).unwrap();
    let path = "ref.in.".repeat(20);
    assert!(schema.shape_at(&format!("{path}leaf")).is_some());
    assert!(schema.shape_at(&format!("{path}loaf")).is_none());
}

/// Two documents whose places refer to each other at random, by pointer, `$id` and anchor.
fn documents() -> impl Strategy<Value = (Value, Value)> {
    let target = prop::sample::select(vec![
        "#",
        "#/$defs/a",
        "#b",
        "b.json",
        "other.json",
        "other.json#/$defs/x",
        "other.json#y",
        "main.json#/$defs/a/properties/p",
        "urn:example:inner",
        "urn:example:inner#/properties/i",
        "missing.json#/a",
        "#/$defs/missing",
        "other.json#missing",
        "%zz#%FF",
    ]);
    let place = (target, 0..5u8).prop_map(|(target, form)| match form {
        0 => json!({"$ref": target}),
        1 => json!({"$ref": target, "type": "object", "properties": {"own": {"type": "string"}}}),
        2 => json!({"anyOf": [{"$ref": target}, {"type": "null"}]}),
        3 => json!({"type": "array", "items": {"$ref": target}}),
        _ => json!({"type": "string", "enum": ["a", "b"]}),
    });
    prop::collection::vec(place, 10).prop_map(|p| {
        let main = json!({
            "type": "object",
            "properties": {"p": p[0], "q": p[1], "r": p[2]},
            "$defs": {
                "a": {"type": "object", "properties": {"p": p[3], "q": p[4]}},
                "b": {"$anchor": "b", "$id": "b.json", "properties": {"x": p[5]}},
                "inner": {"$id": "urn:example:inner", "properties": {"i": p[6]}}
            }
        });
        let other = json!({
            "type": "object",
            "properties": {"x": p[7]},
            "$defs": {"x": p[8], "y": {"$anchor": "y", "properties": {"y": p[9]}}}
        });
        (main, other)
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// Whatever refers to whatever, the load ends, reports only what leads nowhere, and the
    /// service answers.
    #[test]
    fn references_between_documents_never_hang(
        (main, other) in documents(),
        steps in prop::collection::vec(
            prop::sample::select(vec!["p", "q", "r", "x", "y", "i", "own", "*", "**", "nope"]),
            1..10,
        ),
    ) {
        let text = steps.join(".");
        let loader = SchemaLoader::new().document("other.json", &other);
        for loader in [loader, SchemaLoader::new()] {
            let Ok((schema, report)) = loader.load_at("main.json", &main) else {
                continue;
            };
            for unresolved in &report.unresolved {
                prop_assert!(unresolved.place.contains("#/"), "{:?}", unresolved);
            }
            let service = LanguageService::new(Some(schema.clone()));
            for d in service.diagnostics(&text) {
                prop_assert!(d.span.end <= text.len());
            }
            service.complete(&format!("{text}."), text.len() + 1);
            let reads = Expression::compile(&text).unwrap().reads();
            for (path, used) in service.uses(&reads).input.iter().flatten() {
                prop_assert!(used.unknown || schema.shape_at(path).is_some(), "{}", path);
            }
            // Written back, it reads as the same schema once more.
            let again = Schema::from_json_schema(&schema.to_json_schema()).unwrap();
            let twice = Schema::from_json_schema(&again.to_json_schema()).unwrap();
            prop_assert_eq!(&twice, &again);
        }
    }
}

#[test]
fn a_load_asks_for_the_documents_it_misses_and_for_theirs() {
    let main = json!({"properties": {"a": {"$ref": "a.json"}, "gone": {"$ref": "gone.json"}}});
    let mut asked = Vec::new();
    let (schema, report) = SchemaLoader::new()
        .load_asking("https://example.com/s/main.json", &main, |uri, relative| {
            asked.push(uri.to_owned());
            Ok(match relative {
                Some("a.json") => Some(json!({"properties": {"b": {"$ref": "sub/b.json"}}})),
                Some("sub/b.json") => Some(json!({
                    "properties": {
                        "back": {"$ref": "../a.json"},
                        "gone": {"$ref": "../gone.json"},
                        "n": {"type": "number"}
                    }
                })),
                _ => None,
            })
        })
        .unwrap();
    // Each document once, though two places name `gone.json` and two loads miss it.
    assert_eq!(
        asked,
        [
            "https://example.com/s/a.json",
            "https://example.com/s/gone.json",
            "https://example.com/s/sub/b.json"
        ]
    );
    assert_eq!(report.documents(), ["https://example.com/s/gone.json"]);
    let service = LanguageService::new(Some(schema));
    let codes = |text: &str| -> Vec<String> {
        service
            .diagnostics(text)
            .into_iter()
            .map(|d| d.code)
            .collect()
    };
    assert_eq!(codes("a.b.back.b.m"), ["W0201"]);
    assert!(codes("a.b.back.b.n").is_empty());
    assert!(codes("a.b.gone.anything").is_empty());
}

#[test]
fn a_load_that_asks_fails_with_the_answer_s_error_and_stops_asking() {
    let main = json!({"$ref": "a.json"});
    let failed = SchemaLoader::new().load_asking("", &main, |_, _| Err("unreadable".into()));
    assert_eq!(failed.unwrap_err(), "unreadable");

    // More documents than a load asks for: it ends, and reports those it did not ask for.
    let branches: Vec<_> = (0..300)
        .map(|i| json!({"$ref": format!("d{i}.json")}))
        .collect();
    let mut asked = 0;
    let (_, report) = SchemaLoader::new()
        .load_asking("", &json!({"anyOf": branches}), |_, _| {
            asked += 1;
            Ok(Some(json!({"type": "string"})))
        })
        .unwrap();
    assert_eq!(asked, 256);
    assert_eq!(report.documents().len(), 44);
}
