//! Property tests: the language service never panics, whatever the text and the offset.

use jlens_schema::{CompletionKind, FunctionDecl, JsonType, LanguageService, Override, Schema};
use proptest::prelude::*;
use serde_json::json;

#[rustfmt::skip]
const TOKENS: &[&str] = &[
    "customer", "login", "items", "type", "unitPrice", "`odd name`", "$", "$$", "$x", "$limit",
    "1", "'s'", "\"t\"", ".", "[", "]", "(", ")", "{", "}", ":", ",", ";", "=", "!=", "and", "in", "?",
    ":=", "~>", "&", "**", "^(", "@$v", "#$i", "function($v){", "$sum(", "$substring(",
    "$map(", "$rate(", "%", "%.", "login =", "name in", "or", "type =", "unitPrice in [", "$summ(", "$bad(", "/* c */", "/** @label \"x\" */", "\"", "'", "`", "é", "𝄞", " ", "\n",
];

/// The types a JSON value has, as the typing names them.
fn types_of(value: &serde_json::Value) -> Vec<JsonType> {
    match value {
        serde_json::Value::Null => vec![JsonType::Null],
        serde_json::Value::Bool(_) => vec![JsonType::Boolean],
        serde_json::Value::Number(n) if n.as_f64().is_some_and(|f| f.fract() == 0.0) => {
            vec![JsonType::Integer, JsonType::Number]
        }
        serde_json::Value::Number(_) => vec![JsonType::Number],
        serde_json::Value::String(_) => vec![JsonType::String],
        serde_json::Value::Array(_) => vec![JsonType::Array],
        serde_json::Value::Object(_) => vec![JsonType::Object],
    }
}

fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => prop::collection::vec(prop::sample::select(TOKENS), 0..20).prop_map(|t| t.join("")),
        1 => any::<String>(),
    ]
}

#[rustfmt::skip]
const OPEN_TOKENS: &[&str] = &[
    "meta", "totals", "labels", "sealed", "regions", "a", "b", "usd", "count", "`x-a`", "n1",
    "flag", "north", "qty", "sub", ".", "*", "**", "[", "]", "[0]", "(", ")", "1", "'s'", "+",
    ">", "=", "and", "&", "$sum(", "$count(", "$exists(", "$keys(", ",", "%.", " ",
];

fn open_text() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(OPEN_TOKENS), 0..16).prop_map(|t| t.join(""))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn the_language_service_never_panics(text in text()) {
        let sample = json!({
            "customer": {"login": "1", "name": "X"},
            "items": [{"type": "a", "unitPrice": 1.5, "odd name": true}]
        });
        let mut schema = Schema::infer(std::slice::from_ref(&sample));
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
            .with_function(FunctionDecl::new("bad", "").signature("<x<n>"))
            .with_result_type([JsonType::Boolean]);
        let diagnostics = service.diagnostics(&text);
        for d in &diagnostics {
            prop_assert!(d.span.start <= d.span.end && d.span.end <= text.len(), "{d:?}");
            if d.code == "W0206" {
                // The warning points at a literal.
                prop_assert!(jlens_core::parse(&text[d.span.start..d.span.end]).is_ok(), "{d:?}");
            }
        }
        // What the typing says of the result points into the text, and holds for the sample
        // the schema was inferred from.
        let result = service.result_type(&text);
        for span in [result.undefined, result.sequence].into_iter().flatten() {
            prop_assert!(span.end <= text.len(), "{result:?}");
            prop_assert!(text.is_char_boundary(span.start) && text.is_char_boundary(span.end));
        }
        if let (Some(types), Ok(expr)) = (&result.types, jlens_core::Expression::compile(&text)) {
            let options = jlens_core::Options::default().max_steps(10_000);
            let bindings = jlens_core::Bindings::new();
            if let Ok(value) = expr.evaluate_with(Some(&sample), &bindings, &options) {
                match value {
                    None => prop_assert!(result.undefined.is_some(), "undefined: {result:?}"),
                    Some(value) => {
                        let actual = types_of(&value);
                        prop_assert!(actual.iter().any(|t| types.contains(t)), "{value}: {result:?}");
                    }
                }
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

    /// An expression typed as the body of `items` is told what it is told inside
    /// `items.( … )`, at the offsets of its own text, and completion never panics.
    #[test]
    fn a_body_is_checked_as_inside_its_step(text in text()) {
        let schema = Schema::infer(&[json!({
            "customer": {"login": "1", "name": "X"},
            "items": [{"type": "a", "unitPrice": 1.5, "tags": ["x"]}]
        })]);
        let body = LanguageService::new(Some(schema.clone())).with_context_path("items[]");
        let found: Vec<_> = body
            .diagnostics(&text)
            .into_iter()
            .map(|d| (d.code, d.span.start, d.span.end))
            .collect();
        for (_, start, end) in &found {
            prop_assert!(start <= end && *end <= text.len());
        }
        let alone = jlens_core::parse_recover_as_body(&text, 1);
        let wrapped = format!("items.({text})");
        if !alone.has_errors() && !jlens_core::parse_recover(&wrapped).has_errors() {
            let expected: Vec<_> = LanguageService::new(Some(schema))
                .diagnostics(&wrapped)
                .into_iter()
                .map(|d| (d.code, d.span.start - 7, d.span.end - 7))
                .collect();
            prop_assert_eq!(found, expected);
        }
        for offset in 0..=text.len() + 1 {
            let completions = body.complete(&text, offset);
            prop_assert!(completions.range.end <= text.len());
            if let Some(hover) = body.hover(&text, offset) {
                prop_assert!(hover.range.end <= text.len());
            }
        }
    }

    /// Objects open to other fields (`additionalProperties`, `patternProperties`, or no field
    /// listed): what the typing says of a result holds on documents that follow the schema,
    /// and nothing panics.
    #[test]
    fn an_open_object_is_typed_as_its_documents(text in open_text()) {
        let schema = Schema::from_json_schema(&json!({
            "type": "object",
            "required": ["meta", "totals", "labels", "sealed"],
            "properties": {
                "meta": {"type": "object"},
                "totals": {
                    "type": "object",
                    "required": ["count"],
                    "properties": {"count": {"type": "integer"}},
                    "additionalProperties": {"type": "number"}
                },
                "labels": {
                    "type": "object",
                    "patternProperties": {"^x-": {"type": "string"}, "^n": {"type": "number"}},
                    "additionalProperties": {"type": "boolean"}
                },
                "sealed": {
                    "type": "object",
                    "properties": {"a": {"type": "number"}},
                    "additionalProperties": false
                },
                "regions": {"type": "object", "additionalProperties": {"$ref": "#/$defs/region"}}
            },
            "$defs": {"region": {
                "type": "object",
                "required": ["qty"],
                "properties": {"qty": {"type": "number"}, "sub": {"$ref": "#/$defs/region"}}
            }}
        }))
        .unwrap();
        let samples = [
            json!({
                "meta": {"a": "s", "b": [1, {"a": true}]},
                "totals": {"count": 2, "usd": 1.5, "eur": 3},
                "labels": {"x-a": "s", "n1": 2, "flag": true},
                "sealed": {"a": 1},
                "regions": {"north": {"qty": 1, "sub": {"qty": 3}}, "south": {"qty": 2.5}}
            }),
            json!({"meta": {}, "totals": {"count": 0}, "labels": {}, "sealed": {}}),
        ];
        let service = LanguageService::new(Some(schema)).with_result_type([JsonType::Boolean]);
        for d in service.diagnostics(&text) {
            prop_assert!(d.span.start <= d.span.end && d.span.end <= text.len(), "{d:?}");
        }
        let result = service.result_type(&text);
        if let (Some(types), Ok(expr)) = (&result.types, jlens_core::Expression::compile(&text)) {
            let options = jlens_core::Options::default().max_steps(10_000);
            let bindings = jlens_core::Bindings::new();
            for sample in &samples {
                match expr.evaluate_with(Some(sample), &bindings, &options) {
                    Ok(None) => prop_assert!(result.undefined.is_some(), "undefined: {result:?}"),
                    Ok(Some(value)) => {
                        let actual = types_of(&value);
                        prop_assert!(actual.iter().any(|t| types.contains(t)), "{value}: {result:?}");
                    }
                    Err(_) => {}
                }
            }
        }
        for offset in 0..=text.len() + 1 {
            prop_assert!(service.complete(&text, offset).range.end <= text.len());
            if let Some(hover) = service.hover(&text, offset) {
                prop_assert!(hover.range.end <= text.len());
            }
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
        // With the declared function reading its arguments alone, no path of the schema is
        // added. A field the schema does not have is: `$rate(type)` names `type`, which the
        // whole input, listed by what the schema has under it, does not.
        let narrow = whole.uses_of(&expression);
        prop_assert!(!narrow.bindings.contains_key("rate"));
        for (path, entry) in narrow.input.iter().flatten() {
            prop_assert!(
                entry.unknown || uses.input.as_ref().is_some_and(|i| i.contains_key(path)),
                "{}",
                path
            );
        }
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// The fields of a schema (roadmap 0.37 #9) are what an expression that reads the whole
    /// input uses of it, less the hidden fields, whatever the overrides; each tells what the
    /// answer of `uses` tells, and its JSON form reads back.
    #[test]
    fn the_fields_are_what_a_whole_read_uses(
        overrides in prop::collection::vec((override_path(), any::<bool>(), any::<bool>()), 0..6),
    ) {
        let mut schema = Schema::infer(&[json!({
            "customer": {"login": "1", "odd name": "X"},
            "items": [{"type": "a", "tags": [["t"]]}]
        })]);
        let overrides: std::collections::BTreeMap<String, Override> = overrides
            .into_iter()
            .map(|(path, hidden, label)| {
                let o = Override {
                    hidden: Some(hidden),
                    label: label.then(|| "L".to_owned()),
                    ..Override::default()
                };
                (path, o)
            })
            .collect();
        schema.apply_overrides(&overrides);
        let any_hidden = overrides.values().any(|o| o.hidden == Some(true));
        let service = LanguageService::new(Some(schema.clone()));
        let reads = jlens_core::Expression::compile("$").unwrap().reads();
        let uses = service.uses(&reads).input.unwrap_or_default();
        let fields = service.fields();
        prop_assert_eq!(&fields, &schema.fields());
        let paths: Vec<&str> = fields.iter().map(|f| f.path.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort_unstable();
        prop_assert_eq!(&paths, &sorted);
        for field in &fields {
            let Some(entry) = uses.get(&field.path) else {
                return Err(TestCaseError::fail(format!("{} is not used", field.path)));
            };
            prop_assert_eq!(&field.types, &entry.types, "{}", field.path);
            prop_assert_eq!(&field.enumeration, &entry.enumeration, "{}", field.path);
            prop_assert_eq!(field.closed, entry.closed, "{}", field.path);
            // An unknown place keeps whether it is required, which `uses` leaves out.
            if !entry.unknown {
                prop_assert_eq!(field.required, entry.required, "{}", field.path);
            }
            let json = serde_json::to_value(field).unwrap();
            prop_assert_eq!(&serde_json::from_value::<jlens_schema::ListedField>(json).unwrap(), field);
        }
        if !any_hidden {
            prop_assert_eq!(paths.len(), uses.len());
        }
    }
}

#[rustfmt::skip]
const RENAME_TOKENS: &[&str] = &[
    "customer", "login", "vip", "items", "type", "unitPrice", "qty", "total", "`odd name`", "$",
    "$$", ".", "[", "]", "[0]", "(", ")", "{", "}", ":", ",", ";", "=", "!=", "and", "or", "?",
    ":=", "$c", "&", "+", "*", "**", "^(", "@$v", "#$i", "%.", "$v", "$sum(", "$count(",
    "$keys(", "$lookup(", "$string(", "$exists(", "$map(", "function($v){", "'login'", "'type'",
    "«{", "}»", "[customer, items]", "(total ? customer : items).type", " ",
];

/// The renames a table may hold, each a field and its new last name.
const RENAMES: &[(&[&str], &str)] = &[
    (&["customer"], "client"),
    (&["customer", "login"], "user"),
    (&["customer", "odd name"], "plain"),
    (&["customer", "vip"], "gold flag"),
    (&["items"], "lines"),
    (&["items", "type"], "kind"),
    (&["items", "qty"], "quantity"),
    (&["total"], "sum"),
];

/// A JSON value with each key at a renamed path given its new name.
fn renamed_keys(
    value: &serde_json::Value,
    path: &mut Vec<String>,
    table: &std::collections::HashMap<Vec<String>, String>,
) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .map(|item| renamed_keys(item, path, table))
            .collect(),
        serde_json::Value::Object(members) => {
            let mut out = serde_json::Map::new();
            for (key, member) in members {
                path.push(key.clone());
                let name = table.get(path.as_slice()).unwrap_or(key).clone();
                out.insert(name, renamed_keys(member, path, table));
                path.pop();
            }
            serde_json::Value::Object(out)
        }
        other => other.clone(),
    }
}

fn holds_object(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(_) => true,
        serde_json::Value::Array(items) => items.iter().any(holds_object),
        _ => false,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// A rename: an empty table gives the text back, the rewritten text parses, and when the
    /// rename is sure of every place, the rewritten expression gives on the input with its
    /// keys renamed what the expression gives on the input.
    #[test]
    fn a_rename_keeps_what_the_expression_gives(
        tokens in prop::collection::vec(prop::sample::select(RENAME_TOKENS), 0..16),
        chosen in prop::collection::vec(any::<bool>(), RENAMES.len()),
    ) {
        let text = tokens.join("");
        let Ok(expression) = jlens_core::Expression::compile(&text) else {
            return Ok(());
        };
        let service = LanguageService::new(None);
        let none: [(&str, &str); 0] = [];
        let same = service.rename(&expression, none).unwrap();
        prop_assert_eq!(&same.text, &text);
        prop_assert!(same.edits.is_empty() && same.unsure.is_empty());

        let renames: std::collections::HashMap<Vec<String>, String> = RENAMES
            .iter()
            .zip(&chosen)
            .filter(|(_, chosen)| **chosen)
            .map(|((path, name), _)| {
                (path.iter().map(|s| (*s).to_owned()).collect(), (*name).to_owned())
            })
            .collect();
        let write = |path: &[String]| {
            path.iter()
                .map(|name| if name.contains(' ') { format!("`{name}`") } else { name.clone() })
                .collect::<Vec<_>>()
                .join(".")
        };
        let table: Vec<(String, String)> = renames
            .keys()
            .map(|old| {
                let new: Vec<String> = (1..=old.len())
                    .map(|i| renames.get(&old[..i]).unwrap_or(&old[i - 1]).clone())
                    .collect();
                (write(old), write(&new))
            })
            .collect();
        let renamed = service.rename(&expression, table.iter().map(|(a, b)| (a, b))).unwrap();
        let rewritten = jlens_core::Expression::compile(&renamed.text);
        prop_assert!(rewritten.is_ok(), "{} gave {}", text, renamed.text);
        if !renamed.unsure.is_empty() {
            return Ok(());
        }
        let input = json!({
            "customer": {"login": "1", "odd name": "X", "vip": true, "type": "c"},
            "items": [
                {"type": "a", "unitPrice": 1.5, "qty": 2},
                {"type": "b", "unitPrice": 2, "qty": 1}
            ],
            "total": 3
        });
        let moved = renamed_keys(&input, &mut Vec::new(), &renames);
        let options = jlens_core::Options::default().max_steps(10_000);
        let bindings = jlens_core::Bindings::new();
        let before = expression.evaluate_with(Some(&input), &bindings, &options);
        let after = rewritten.unwrap().evaluate_with(Some(&moved), &bindings, &options);
        match (&before, &after) {
            (Ok(a), Ok(b)) => {
                if !a.as_ref().is_some_and(holds_object) {
                    prop_assert_eq!(a, b, "{} gave {}", text, renamed.text);
                }
            }
            (Err(a), Err(b)) => prop_assert_eq!(a.code(), b.code(), "{} gave {}", text, renamed.text),
            _ => prop_assert!(false, "{} gave {}: {:?} then {:?}", text, renamed.text, before, after),
        }
    }
}
