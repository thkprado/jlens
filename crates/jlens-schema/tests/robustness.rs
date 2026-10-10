//! Property tests: the language service never panics, whatever the text and the offset.

use jlens_schema::{LanguageService, Schema};
use proptest::prelude::*;
use serde_json::json;

#[rustfmt::skip]
const TOKENS: &[&str] = &[
    "customer", "login", "items", "type", "unitPrice", "`odd name`", "$", "$$", "$x", "$limit",
    "1", "'s'", "\"t\"", ".", "[", "]", "(", ")", "{", "}", ":", ",", ";", "=", "and", "in", "?",
    ":=", "~>", "&", "**", "^(", "@$v", "#$i", "function($v){", "$sum(", "$substring(",
    "$map(", "/* c */", "/** @label \"x\" */", "\"", "'", "`", "é", "𝄞", " ", "\n",
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
        let schema = Schema::infer(&[json!({
            "customer": {"login": "1", "name": "X"},
            "items": [{"type": "a", "unitPrice": 1.5, "odd name": true}]
        })]);
        let service = LanguageService::new(Some(schema)).with_variable("limit", None);
        let diagnostics = service.diagnostics(&text);
        for d in &diagnostics {
            prop_assert!(d.span.start <= d.span.end && d.span.end <= text.len(), "{d:?}");
        }
        // Every byte offset, including those inside characters and past the end.
        for offset in 0..=text.len() + 1 {
            let completions = service.complete(&text, offset);
            prop_assert!(completions.range.start <= completions.range.end);
            prop_assert!(completions.range.end <= text.len());
            if let Some(hover) = service.hover(&text, offset) {
                prop_assert!(hover.range.end <= text.len());
            }
            let _ = service.signature_help(&text, offset);
        }
    }
}
