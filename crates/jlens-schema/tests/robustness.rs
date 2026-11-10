//! Property tests: the language service never panics, whatever the text and the offset.

use jlens_schema::{CompletionKind, FunctionDecl, LanguageService, Override, Schema};
use proptest::prelude::*;
use serde_json::json;

#[rustfmt::skip]
const TOKENS: &[&str] = &[
    "customer", "login", "items", "type", "unitPrice", "`odd name`", "$", "$$", "$x", "$limit",
    "1", "'s'", "\"t\"", ".", "[", "]", "(", ")", "{", "}", ":", ",", ";", "=", "!=", "and", "in", "?",
    ":=", "~>", "&", "**", "^(", "@$v", "#$i", "function($v){", "$sum(", "$substring(",
    "$map(", "$rate(", "login =", "name in", "or", "type =", "unitPrice in [", "$summ(", "$bad(", "/* c */", "/** @label \"x\" */", "\"", "'", "`", "é", "𝄞", " ", "\n",
];

fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => prop::collection::vec(prop::sample::select(TOKENS), 0..20).prop_map(|t| t.join("")),
        1 => any::<String>(),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn the_language_service_never_panics(text in text()) {
        let mut schema = Schema::infer(&[json!({
            "customer": {"login": "1", "name": "X"},
            "items": [{"type": "a", "unitPrice": 1.5, "odd name": true}]
        })]);
        // An enumeration with values that need escaping, or cannot take single quotes, one of
        // scalars, whose values outside it are warned about, and an open domain.
        let overrides: std::collections::BTreeMap<String, Override> = serde_json::from_value(
            json!({
                "items[].type": {
                    "enum": ["a", "it's", "say \"é\"", 1, null, {"k": [1]}],
                    "domain": "types"
                },
                "login": {"enum": ["s", "é𝄞", 1, true]},
                "customer.name": {"domain": "people"}
            }),
        )
        .unwrap();
        prop_assert!(schema.apply_overrides(&overrides).is_empty());
        let paths = schema.clone();
        let service = LanguageService::new(Some(schema))
            .with_variable("limit", None)
            .with_function(
                FunctionDecl::new("rate", "A rate").signature("<s?:n>").params(["region"]),
            )
            // A host function that replaces a built-in.
            .with_function(FunctionDecl::new("sum", ""))
            // A signature that does not parse: calls are not checked against it.
            .with_function(FunctionDecl::new("bad", "").signature("<x<n>"));
        let diagnostics = service.diagnostics(&text);
        for d in &diagnostics {
            prop_assert!(d.span.start <= d.span.end && d.span.end <= text.len(), "{d:?}");
            if d.code == "W0206" {
                // The warning points at a literal.
                prop_assert!(jlens_core::parse(&text[d.span.start..d.span.end]).is_ok(), "{d:?}");
            }
        }
        // Every byte offset, including those inside characters and past the end.
        for offset in 0..=text.len() + 1 {
            let completions = service.complete(&text, offset);
            prop_assert!(completions.range.start <= completions.range.end);
            prop_assert!(completions.range.end <= text.len());
            prop_assert!(text.is_char_boundary(completions.range.start));
            prop_assert!(text.is_char_boundary(completions.range.end));
            for item in completions.items.iter().filter(|i| i.kind == CompletionKind::Value) {
                // A value is inserted as a literal that parses.
                prop_assert!(jlens_core::parse(&item.insert_text).is_ok(), "{item:?}");
            }
            if let Some(request) = &completions.value_request {
                // The request replaces what completion replaces, its prefix ends at the cursor,
                // and the values it writes are literals that parse.
                prop_assert_eq!(request.range, completions.range);
                let mut cursor = offset.min(text.len());
                while !text.is_char_boundary(cursor) {
                    cursor -= 1;
                }
                prop_assert!(text[..cursor].ends_with(&request.prefix), "{request:?}");
                // A condition names a field of the schema and the literals it is compared with.
                for value in &request.listed {
                    prop_assert!(request.in_array, "{request:?}");
                    prop_assert!(jlens_core::parse(&request.literal(value)).is_ok(), "{value}");
                }
                for condition in &request.conditions {
                    prop_assert!(paths.shape_at(&condition.path).is_some(), "{condition:?}");
                    prop_assert!(!condition.values.is_empty(), "{condition:?}");
                }
                for value in [json!("it's"), json!("say \"é𝄞\""), json!(1.5), json!(null)] {
                    let literal = request.literal(&value);
                    prop_assert!(jlens_core::parse(&literal).is_ok(), "{literal}");
                }
            }
            if let Some(hover) = service.hover(&text, offset) {
                prop_assert!(hover.range.end <= text.len());
            }
            let _ = service.signature_help(&text, offset);
        }
    }

    #[test]
    fn missing_override_paths_are_the_ones_created(
        paths in prop::collection::vec(override_path(), 0..6),
    ) {
        let mut schema = Schema::infer(&[json!({
            "customer": {"login": "1", "odd name": "X"},
            "items": [{"type": "a", "tags": [["t"]]}]
        })]);
        let overrides: std::collections::BTreeMap<String, Override> =
            paths.into_iter().map(|p| (p, Override::default())).collect();
        let missing = schema.missing_override_paths(&overrides);
        for m in &missing {
            prop_assert!(schema.shape_at(&m.path).is_none(), "{m:?}");
            if let Some(s) = &m.suggestion {
                prop_assert!(schema.shape_at(s).is_some(), "{m:?}");
            }
        }
        let failed = schema.apply_overrides(&overrides);
        for path in overrides.keys() {
            // Each path either existed, was listed (and then created), or was malformed.
            let listed = missing.iter().any(|m| &m.path == path);
            prop_assert!(!(listed && failed.contains(path)), "{path}");
        }
    }
}

/// The fields of the sample document of `what_an_expression_uses`, each with its path in the
/// answer and the JSON pointer that removes it from the sample.
const SAMPLE_FIELDS: &[(&str, &str)] = &[
    ("customer", "/customer"),
    ("customer.login", "/customer/login"),
    ("customer.`odd.name`", "/customer/odd.name"),
    ("items", "/items"),
    ("items[].type", "/items/0/type"),
    ("items[].unitPrice", "/items/0/unitPrice"),
    ("items[].tags", "/items/0/tags"),
    ("items[].tags[][].``", "/items/0/tags/0/0/"),
    ("total", "/total"),
];

fn without(sample: &serde_json::Value, pointer: &str) -> serde_json::Value {
    let mut sample = sample.clone();
    let (parent, name) = pointer.rsplit_once('/').unwrap();
    let removed = sample
        .pointer_mut(parent)
        .and_then(|p| p.as_object_mut())
        .and_then(|o| o.remove(name));
    assert!(removed.is_some(), "{pointer}");
    sample
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// What an expression uses of its schema: every path is one the schema reads, the JSON
    /// form reads back, and the answer does not depend on a field it does not list. Then what
    /// changed between two answers.
    #[test]
    fn what_an_expression_uses(text in text()) {
        let Ok(expression) = jlens_core::Expression::compile(&text) else {
            return Ok(());
        };
        let reads = expression.reads();
        let sample = json!({
            "customer": {"login": "1", "odd.name": "X"},
            "items": [{"type": "a", "unitPrice": 1.5, "tags": [[{"": 1}]]}],
            "total": 1
        });
        let service = |sample: &serde_json::Value| {
            let schema = Schema::infer(std::slice::from_ref(sample));
            let limit = schema.root.fields.get("customer").map(|f| f.shape.clone());
            (
                schema.clone(),
                LanguageService::new(Some(schema))
                    .with_variable("limit", limit)
                    .with_variable("x", None)
                    .with_function(FunctionDecl::new("rate", "A rate")),
            )
        };
        let (schema, whole) = service(&sample);
        let uses = whole.uses(&reads);
        let json = serde_json::to_string(&uses).unwrap();
        prop_assert_eq!(&serde_json::from_str::<jlens_schema::Uses>(&json).unwrap(), &uses);
        prop_assert_eq!(uses.input.is_some(), reads.input().is_some());
        prop_assert_eq!(uses.every_binding, reads.every_binding());
        prop_assert!(!uses.bindings.contains_key("rate"));
        let paths = uses.input.iter().flatten();
        for (path, entry) in paths.chain(uses.bindings.get("limit").into_iter().flatten()) {
            prop_assert!(!entry.unknown || entry.types.is_empty(), "{path}: {entry:?}");
        }
        for (path, entry) in uses.input.iter().flatten() {
            // A name that holds a backquote is not read back: `shape_at` drops the quotes.
            if entry.unknown && path.contains('`') {
                continue;
            }
            prop_assert_eq!(schema.shape_at(path).is_some(), !entry.unknown, "{}", path);
        }
        for (name, read) in reads.bindings() {
            prop_assert!(name == "rate" || uses.bindings.contains_key(name), "{name}: {read:?}");
        }
        // A field the answer does not list is not used: without it, the answer is the same.
        // The variable `limit` has the shape of `customer`, so its fields stay.
        let listed = |path: &str| uses.input.as_ref().is_some_and(|i| i.contains_key(path));
        for (path, pointer) in SAMPLE_FIELDS {
            if listed(path) || path.starts_with("customer") {
                continue;
            }
            let (_, cut) = service(&without(&sample, pointer));
            prop_assert_eq!(&cut.uses(&reads), &uses, "without {}", path);
        }
        // What changed between two schemas: nothing between equal answers, and each path of
        // the two answers is added, removed, changed or equal.
        prop_assert!(uses.changes(&uses).is_empty());
        for (path, pointer) in SAMPLE_FIELDS {
            let (_, cut) = service(&without(&sample, pointer));
            let after = cut.uses(&reads);
            let changes = uses.changes(&after);
            prop_assert_eq!(changes.is_empty(), after == uses, "without {}", path);
            let json = serde_json::to_string(&changes).unwrap();
            prop_assert_eq!(
                &serde_json::from_str::<jlens_schema::UsesChanges>(&json).unwrap(),
                &changes
            );
            prop_assert_eq!(json == "{}", changes.is_empty());
            // The other way, what was added is removed.
            let back = after.changes(&uses);
            prop_assert_eq!(&back.input.removed, &changes.input.added);
            prop_assert_eq!(&back.input.added, &changes.input.removed);
            prop_assert_eq!(
                back.input.changed.keys().collect::<Vec<_>>(),
                changes.input.changed.keys().collect::<Vec<_>>()
            );
            let empty = Default::default();
            let old = uses.input.as_ref().unwrap_or(&empty);
            let new = after.input.as_ref().unwrap_or(&empty);
            for (path, entry) in old {
                let same = new.get(path) == Some(entry);
                let listed = changes.input.removed.get(path) == Some(entry)
                    || changes.input.changed.contains_key(path);
                prop_assert_eq!(same, !listed, "{}", path);
            }
            for (path, entry) in new {
                prop_assert_eq!(
                    old.contains_key(path),
                    changes.input.added.get(path) != Some(entry),
                    "{}", path
                );
            }
        }
    }
}

#[rustfmt::skip]
const PATH_TOKENS: &[&str] = &[
    "customer", "custmer", "login", "logn", "items", "type", "tags", "`odd name`", "odd name",
    "[]", ".", "$", "`", "é", " ",
];

fn override_path() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => prop::collection::vec(prop::sample::select(PATH_TOKENS), 0..8).prop_map(|t| t.join("")),
        1 => any::<String>(),
    ]
}
