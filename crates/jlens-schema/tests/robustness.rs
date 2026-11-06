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
