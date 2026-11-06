//! Property tests: the language service never panics, whatever the text and the offset.

use jlens_schema::{CompletionKind, FunctionDecl, LanguageService, Override, Schema};
use proptest::prelude::*;
use serde_json::json;

#[rustfmt::skip]
const TOKENS: &[&str] = &[
    "customer", "login", "items", "type", "unitPrice", "`odd name`", "$", "$$", "$x", "$limit",
    "1", "'s'", "\"t\"", ".", "[", "]", "(", ")", "{", "}", ":", ",", ";", "=", "!=", "and", "in", "?",
    ":=", "~>", "&", "**", "^(", "@$v", "#$i", "function($v){", "$sum(", "$substring(",
    "$map(", "$rate(", "$summ(", "$bad(", "/* c */", "/** @label \"x\" */", "\"", "'", "`", "é", "𝄞", " ", "\n",
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
        // An enumeration with values that need escaping, or cannot take single quotes.
        let overrides: std::collections::BTreeMap<String, Override> = serde_json::from_value(
            json!({"items[].type": {"enum": ["a", "it's", "say \"é\"", 1, null, {"k": [1]}]}}),
        )
        .unwrap();
        prop_assert!(schema.apply_overrides(&overrides).is_empty());
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
            if let Some(hover) = service.hover(&text, offset) {
                prop_assert!(hover.range.end <= text.len());
            }
            let _ = service.signature_help(&text, offset);
        }
    }
}
