//! The bodies of the fuzz targets. Each `fuzz_targets/*.rs` calls one with libFuzzer's input,
//! and the `replay` program calls them with saved inputs, without libFuzzer.

use std::collections::HashMap;

use jlens_core::ast::Span;
use jlens_core::audit::AuditOptions;
use jlens_core::reads::Read;
use jlens_core::{Bindings, Document, Expression};
use jlens_schema::{
    FunctionDecl, JsonType, LanguageService, ListedField, Renamed, Schema, SchemaLoader, Uses,
};
use serde_json::json;

use crate::{bindings, default_input, on_small_stack, options, record};

/// The body of a target: it runs one input, and panics when what must hold does not.
pub type Target = fn(&[u8]);

/// Every target, by name.
pub const TARGETS: &[(&str, Target)] = &[
    ("parse", parse),
    ("parse_recover", parse_recover),
    ("evaluate", evaluate),
    ("regex", regex),
    ("pictures", pictures),
    ("language_service", language_service),
    ("json_schema", json_schema),
    ("parse_cut", parse_cut),
    ("rename", rename),
];

/// The longest prefix of `data` that is UTF-8: what libFuzzer passes to a target that takes a
/// `&str` (`arbitrary_take_rest`).
fn utf8_prefix(data: &[u8]) -> &str {
    std::str::from_utf8(data)
        .unwrap_or_else(|e| std::str::from_utf8(&data[..e.valid_up_to()]).expect("a valid prefix"))
}

/// Strict parsing never panics.
pub fn parse(data: &[u8]) {
    let source = utf8_prefix(data);
    record(source.as_bytes());
    on_small_stack(|| {
        let _ = jlens_core::parse(source);
    });
}

/// Recovering parsing never panics, and its diagnostics lie within the source.
pub fn parse_recover(data: &[u8]) {
    let source = utf8_prefix(data);
    record(source.as_bytes());
    on_small_stack(|| {
        let ast = jlens_core::parse_recover(source);
        for d in &ast.diagnostics {
            assert!(
                d.span.start <= d.span.end && d.span.end <= source.len(),
                "{d:?}"
            );
        }
    });
}

/// Compiling and evaluating never panic, under limits.
///
/// The input is an expression, optionally followed by a NUL byte and the JSON input document.
/// Without a document, or when it is not JSON, the evaluation reads a fixed one.
pub fn evaluate(data: &[u8]) {
    record(data);
    let (source, input) = match data.iter().position(|&b| b == 0) {
        Some(nul) => (&data[..nul], serde_json::from_slice(&data[nul + 1..]).ok()),
        None => (data, None),
    };
    let Ok(source) = std::str::from_utf8(source) else {
        return;
    };
    let input = input.unwrap_or_else(default_input);
    on_small_stack(|| {
        let Ok(expr) = Expression::compile(source) else {
            return;
        };
        let options = options();
        let bindings = bindings();
        let _ = expr.evaluate_with(Some(&input), &bindings, &options);
        let _ = expr.evaluate_with(None, &bindings, &options);
        let document = Document::new(&input);
        let _ = expr.evaluate_document(&document, &bindings, &options);
        // The document bound to a variable as well, as the packages bind documents (0.35 #7).
        let _ = expr.evaluate_document_with(&document, &[("doc", &document)], &bindings, &options);
        let _ = expr.evaluate_audited(Some(&input), &bindings, &options, &AuditOptions::full());
    });
}

/// How many times the `regex` target asks to go on, across the matches of one input: about a
/// million backtracking steps (see `regress::INTERRUPT_INTERVAL`).
const POLLS: u32 = 1000;

/// Compiling a regular expression and matching it never panic, and matches lie within the text.
///
/// The first byte of the input chooses the flags, one bit each for `i`, `m`, `s`, `u` and `v`.
/// The rest is the pattern, a NUL byte, and the text to search.
pub fn regex(data: &[u8]) {
    record(data);
    let Some((&bits, rest)) = data.split_first() else {
        return;
    };
    let (pattern, text) = match rest.iter().position(|&b| b == 0) {
        Some(nul) => (&rest[..nul], &rest[nul + 1..]),
        None => (rest, &[][..]),
    };
    let (Ok(pattern), Ok(text)) = (std::str::from_utf8(pattern), std::str::from_utf8(text)) else {
        return;
    };
    let flags: String = "imsuv"
        .chars()
        .enumerate()
        .filter(|(i, _)| bits & (1 << i) != 0)
        .map(|(_, c)| c)
        .collect();
    on_small_stack(|| {
        let Ok(re) = regress::Regex::with_flags(pattern, flags.as_str()) else {
            return;
        };
        let mut polls = 0;
        let mut stop = || {
            polls += 1;
            polls > POLLS
        };
        let mut start = 0;
        while start <= text.len() {
            let Ok(Some(m)) = re.find_from_interruptible(text, start, &mut stop) else {
                return;
            };
            for range in m.groups().flatten() {
                assert!(
                    range.start <= range.end && range.end <= text.len(),
                    "{range:?}"
                );
                assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
            }
            for (_, range) in m.named_groups() {
                assert!(range.is_none_or(|r| r.end <= text.len()));
            }
            // An empty match moves on by one character, as JavaScript's `g` flag does.
            start = match text[m.end()..].chars().next() {
                _ if m.end() > m.start() => m.end(),
                Some(c) => m.end() + c.len_utf8(),
                None => break,
            };
        }
    });
}

/// The calls of the `pictures` target. Each is evaluated on its own, so that one that fails does
/// not hide the others.
const CALLS: &[&str] = &[
    "$formatNumber(1234.5678, $picture)",
    "$formatNumber(-0.000123, $picture)",
    "$formatNumber(1e300, $picture)",
    "$formatNumber(0, $picture, {'decimal-separator': $text, 'percent': $text})",
    "$formatInteger(1234, $picture)",
    "$formatInteger(-7, $picture)",
    "$formatInteger(0, $picture)",
    "$parseInteger($text, $picture)",
    "$fromMillis(1234567890123, $picture)",
    "$fromMillis(-62198755200000, $picture, $text)",
    "$toMillis($text, $picture)",
];

/// The pictures of `$formatNumber`, `$formatInteger`, `$parseInteger`, `$fromMillis` and
/// `$toMillis` never make them panic.
///
/// The input is a picture, optionally followed by a NUL byte and a second string: the text that
/// `$parseInteger` and `$toMillis` parse, and the time zone of `$fromMillis`.
pub fn pictures(data: &[u8]) {
    record(data);
    let (picture, text) = match data.iter().position(|&b| b == 0) {
        Some(nul) => (&data[..nul], &data[nul + 1..]),
        None => (data, &[][..]),
    };
    let (Ok(picture), Ok(text)) = (std::str::from_utf8(picture), std::str::from_utf8(text)) else {
        return;
    };
    on_small_stack(|| {
        let bindings = Bindings::new()
            .var("picture", json!(picture))
            .var("text", json!(text));
        let options = options();
        for call in CALLS {
            let expr = Expression::compile(call).expect("valid expression");
            let _ = expr.evaluate_with(None, &bindings, &options);
        }
    });
}

/// The language service never panics on any text, at any offset, and its ranges lie within the
/// text.
///
/// The first two bytes of the input give the offset (little-endian, modulo the length of the
/// text plus two, so that it can fall inside a character or past the end). The rest is the text.
pub fn language_service(data: &[u8]) {
    record(data);
    let Some((offset, text)) = data.split_first_chunk::<2>() else {
        return;
    };
    let Ok(text) = std::str::from_utf8(text) else {
        return;
    };
    let offset = usize::from(u16::from_le_bytes(*offset)) % (text.len() + 2);
    on_small_stack(|| {
        let schema = Schema::infer(&[default_input()]);
        let items = schema.shape_at("items").cloned();
        let service = LanguageService::new(Some(schema))
            .with_variable("items", items)
            .with_variable("x", None)
            .with_function(
                FunctionDecl::new("echo", "Returns its arguments.")
                    .signature("<x-x?:o>")
                    .params(["first", "second"]),
            );
        for d in service.diagnostics(text) {
            assert!(
                d.span.start <= d.span.end && d.span.end <= text.len(),
                "{d:?}"
            );
        }
        let completions = service.complete(text, offset);
        let range = completions.range;
        assert!(
            range.start <= range.end && range.end <= text.len(),
            "{range:?}"
        );
        if let Some(hover) = service.hover(text, offset) {
            let range = hover.range;
            assert!(
                range.start <= range.end && range.end <= text.len(),
                "{range:?}"
            );
        }
        let _ = service.signature_help(text, offset);

        // A schema that refers to itself and to a document it was not given (roadmap 0.33 #8
        // and #9): the typing follows it as deep as the text goes, and never further.
        let recursive = LanguageService::new(Some(
            Schema::from_json_schema(&json!({
                "$ref": "#/$defs/node",
                "$defs": {"node": {
                    "type": "object",
                    "required": ["name"],
                    "properties": {
                        "name": {"type": "string"},
                        "kind": {"enum": ["file", "folder"]},
                        "items": {"type": "array", "items": {"$ref": "#/$defs/node"}},
                        "parent": {"$ref": "#"},
                        "owner": {"$ref": "people.json#/$defs/person"}
                    }
                }}
            }))
            .expect("a schema"),
        ))
        .with_function(FunctionDecl::new("echo", "Returns its arguments.").signature("<x-x?:o>"));
        for d in recursive.diagnostics(text) {
            assert!(
                d.span.start <= d.span.end && d.span.end <= text.len(),
                "{d:?}"
            );
        }
        let _ = recursive.complete(text, offset);
        let _ = recursive.hover(text, offset);

        // The typing of roadmap 0.34: a result expected to be a boolean, the text as the body of
        // `lines[]`, and the keywords it reads, `anyOf` and `oneOf` as one branch or another, open
        // objects, tuples and conditional fields; and of 0.35, the positions of tuples, read
        // after an index (`rows[0][1]`, `rows[$i][1]`) and through `anyOf`.
        let typed = LanguageService::new(Some(
            Schema::from_json_schema(&json!({
                "type": "object",
                "required": ["lines", "status"],
                "properties": {
                    "status": {"anyOf": [{"type": "string"}, {"const": 1}, {}]},
                    "lines": {"type": "array", "minItems": 1, "items": {
                        "oneOf": [
                            {"type": "object", "required": ["qty"],
                             "properties": {"qty": {"type": "integer"}, "tags": {"type": "array"}}},
                            {"type": "object", "properties": {"note": {"type": ["string", "null"]}}}
                        ]
                    }},
                    "attrs": {"type": "object",
                              "patternProperties": {"^x-": {"type": "string"}, "(": {}},
                              "additionalProperties": {"type": "number"}},
                    "pair": {"type": "array", "prefixItems": [{"type": "string"}],
                             "items": {"type": "number"}},
                    "rows": {"type": "array", "items": {"type": "array",
                             "prefixItems": [{"type": "string"}, {"type": "number"}],
                             "items": false}},
                    "point": {"anyOf": [
                        {"type": "array", "prefixItems": [{"type": "number"}, {"type": "string"}]},
                        {"type": "array", "prefixItems": [{"type": "integer"}]}
                    ]},
                    "kind": {"type": "string"}
                },
                "if": {"properties": {"kind": {"const": "a"}}},
                "then": {"properties": {"extra": {"type": "boolean"}}},
                "else": {"properties": {"other": {"type": "string"}}}
            }))
            .expect("a schema"),
        ));
        for service in [
            typed.clone().with_result_type([JsonType::Boolean]),
            typed
                .with_context_path("lines[]")
                .with_result_type([JsonType::Number]),
        ] {
            for d in service.diagnostics(text) {
                assert!(
                    d.span.start <= d.span.end && d.span.end <= text.len(),
                    "{d:?}"
                );
            }
            let result = service.result_type(text);
            for span in [result.undefined, result.sequence].into_iter().flatten() {
                assert!(span.start <= span.end && span.end <= text.len(), "{span:?}");
            }
            let _ = service.complete(text, offset);
            let _ = service.hover(text, offset);
        }

        // What the expression uses of each schema reads back from its JSON form, and compared
        // with itself has no change (roadmap 0.33 #5 and #6).
        let Ok(expression) = Expression::compile(text) else {
            return;
        };
        let answers = [&service, &recursive].map(|service| {
            let uses = service.uses_of(&expression);
            let json = serde_json::to_value(&uses).expect("JSON");
            // A path that holds a backquote does not read back: such an answer is refused.
            if let Ok(read) = Uses::from_json(&json) {
                assert_eq!(read, uses, "{json}");
            }
            assert!(uses.changes(&uses).is_empty(), "{json}");
            uses
        });
        let changes = answers[0].changes(&answers[1]);
        let _ = serde_json::to_value(&changes).expect("JSON");
    });
}

/// Splits an input at its first two NUL bytes, into up to three parts.
fn parts(data: &[u8]) -> [&[u8]; 3] {
    let mut parts = data.splitn(3, |&b| b == 0);
    [(); 3].map(|_| parts.next().unwrap_or_default())
}

/// Asserts that a span lies within `text`, on character boundaries.
fn within(span: Span, text: &str) {
    assert!(
        span.start <= span.end
            && span.end <= text.len()
            && text.is_char_boundary(span.start)
            && text.is_char_boundary(span.end),
        "{span:?} in {text:?}"
    );
}

/// Loading any JSON Schema never panics, and what it loads is written back as a JSON Schema
/// that reads again as itself; the fields it lists are the ones the service knows (roadmap
/// 0.37 #9 and #19).
///
/// The input is a JSON document, read as a JSON Schema (that also refers to itself under
/// another URI) and as a sample, from which a schema is inferred.
pub fn json_schema(data: &[u8]) {
    record(data);
    let Ok(doc) = serde_json::from_slice::<serde_json::Value>(data) else {
        return;
    };
    on_small_stack(|| {
        let inferred = Schema::infer(std::slice::from_ref(&doc));
        let read =
            Schema::from_json_schema(&inferred.to_json_schema()).expect("an inferred schema");
        assert_eq!(read.root, inferred.root.without_state());
        check_schema(inferred);
        let loaded = SchemaLoader::new()
            .document("https://example.com/other.json", &doc)
            .load_at("https://example.com/schema.json", &doc);
        if let Ok((schema, report)) = loaded {
            let _ = serde_json::to_value(&report).expect("JSON");
            check_schema(schema);
        }
    });
}

/// What [`json_schema`] checks of each schema it gets.
fn check_schema(schema: Schema) {
    // Written back, the schema reads again; once read, it is written back as itself (but for
    // the definitions that nothing refers to, which are not written).
    let written = schema.to_json_schema();
    let again = Schema::from_json_schema(&written).unwrap_or_else(|e| panic!("{e}: {written}"));
    let twice = Schema::from_json_schema(&again.to_json_schema()).expect("written twice");
    assert_eq!(twice, again, "{written}");

    let service = LanguageService::new(Some(schema));
    let fields = service.fields();
    let json = serde_json::to_value(&fields).expect("JSON");
    let read: Vec<ListedField> = serde_json::from_value(json.clone()).expect("read back");
    assert_eq!(read, fields, "{json}");

    // Each listed field's path is an expression the service knows, and completes after it.
    let texts = fields.iter().take(8).map(|field| field.path.clone());
    for text in texts.chain(["".into(), "$.".into(), "**.".into()]) {
        let diagnostics = service.diagnostics(&text);
        for d in &diagnostics {
            within(d.span, &text);
        }
        // A name that holds a backquote cannot be written in JSONata: its path may read as
        // another one, or not at all.
        if !text.ends_with('.') && !key_holds_backquote(&written) {
            assert!(
                diagnostics.iter().all(|d| d.code != "W0201"),
                "{text}: {diagnostics:?} in {written}"
            );
        }
        let dotted = format!("{text}.");
        let completions = service.complete(&dotted, dotted.len());
        within(completions.range, &dotted);
        if let Some(hover) = service.hover(&text, text.len()) {
            within(hover.range, &text);
        }
        if let Ok(expression) = Expression::compile(&text) {
            let uses = service.uses_of(&expression);
            let json = serde_json::to_value(&uses).expect("JSON");
            if let Ok(read) = Uses::from_json(&json) {
                assert_eq!(read, uses, "{json}");
            }
        }
    }
}

/// Whether a key of an object in `value`, at any depth, holds a backquote.
fn key_holds_backquote(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(members) => members
            .iter()
            .any(|(key, member)| key.contains('`') || key_holds_backquote(member)),
        serde_json::Value::Array(items) => items.iter().any(key_holds_backquote),
        _ => false,
    }
}

/// An outcome to compare: the value, or the error's code; `None` for a timeout, which depends
/// on the machine.
fn outcome(result: Result<Option<serde_json::Value>, jlens_core::Error>) -> Option<String> {
    match result {
        Ok(value) => Some(format!("{value:?}")),
        Err(e) if e.code() == "D1012" => None,
        Err(e) => Some(e.code().to_owned()),
    }
}

/// A document parsed cut by what an expression reads serves the expression as the whole one
/// does, and never panics (roadmap 0.37 #19).
///
/// The input is an expression, optionally followed by a NUL byte and the JSON text of the input
/// document. Without one, the text is a fixed document's.
pub fn parse_cut(data: &[u8]) {
    record(data);
    let [source, text, _] = parts(data);
    let (Ok(source), Ok(text)) = (std::str::from_utf8(source), std::str::from_utf8(text)) else {
        return;
    };
    let fixed;
    let text = if data.contains(&0) {
        text
    } else {
        fixed = default_input().to_string();
        &fixed
    };
    on_small_stack(|| {
        // Cut by nothing, the document is the one `parse` gives, and fails where it fails.
        let whole = Document::parse(text);
        let all = Document::parse_cut(text, &Read::Whole);
        assert_eq!(whole.is_ok(), all.is_ok(), "{text}");
        let Ok(whole) = whole else {
            let Ok(expr) = Expression::compile(source) else {
                return;
            };
            if let Some(read) = expr.reads().input() {
                let _ = Document::parse_cut(text, read);
            }
            return;
        };
        let value = whole.to_json();
        assert_eq!(all.expect("parsed").to_json(), value);
        let Ok(expr) = Expression::compile(source) else {
            return;
        };
        let reads = expr.reads();
        let Some(read) = reads.input() else {
            return;
        };
        let cut = Document::parse_cut(text, read).expect("parsed whole, so parsed cut");
        assert_eq!(cut.to_json(), Document::new(&read.cut(&value)).to_json());
        let options = options();
        let bindings = bindings();
        let a = outcome(expr.evaluate_document(&whole, &bindings, &options));
        let b = outcome(expr.evaluate_document(&cut, &bindings, &options));
        if let (Some(a), Some(b)) = (a, b) {
            assert_eq!(
                a,
                b,
                "{source} reads {}",
                serde_json::to_string(read).expect("JSON")
            );
        }
    });
}

/// A name written as a path of the rename table writes it, `None` for one it cannot hold.
fn written_name(name: &str) -> Option<String> {
    let plain = name
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    match name {
        _ if plain => Some(name.to_owned()),
        _ if name.is_empty() || name.contains('`') => None,
        _ => Some(format!("`{name}`")),
    }
}

/// The rename table of every field `read` reads, each given a new name, as text pairs and by
/// path; `false` past 64 fields, where a table cut short could rename `a` to the `a_v2` it
/// leaves out.
fn rename_all(
    read: &Read,
    path: &mut Vec<String>,
    written: (&str, &str),
    table: &mut Vec<(String, String)>,
    renamed: &mut HashMap<Vec<String>, String>,
) -> bool {
    let Read::Fields(fields) = read else {
        return true;
    };
    for (name, inner) in fields {
        let new = format!("{name}_v2");
        let (Some(old_name), Some(new_name)) = (written_name(name), written_name(&new)) else {
            continue;
        };
        if table.len() >= 64 {
            return false;
        }
        let join = |parent: &str, name: &str| match parent {
            "" => name.to_owned(),
            _ => format!("{parent}.{name}"),
        };
        let (old, new_path) = (join(written.0, &old_name), join(written.1, &new_name));
        table.push((old.clone(), new_path.clone()));
        path.push(name.clone());
        renamed.insert(path.clone(), new);
        let whole = rename_all(inner, path, (&old, &new_path), table, renamed);
        path.pop();
        if !whole {
            return false;
        }
    }
    true
}

/// A JSON value with each key at a renamed path given its new name; `None` when a new name is
/// already a key of its object.
fn renamed_keys(
    value: &serde_json::Value,
    path: &mut Vec<String>,
    table: &HashMap<Vec<String>, String>,
) -> Option<serde_json::Value> {
    Some(match value {
        serde_json::Value::Array(items) => items
            .iter()
            .map(|item| renamed_keys(item, path, table))
            .collect::<Option<_>>()?,
        serde_json::Value::Object(members) => {
            let mut out = serde_json::Map::new();
            for (key, member) in members {
                path.push(key.clone());
                let name = table.get(path.as_slice()).unwrap_or(key).clone();
                let member = renamed_keys(member, path, table);
                path.pop();
                if out.insert(name, member?).is_some() {
                    return None;
                }
            }
            serde_json::Value::Object(out)
        }
        other => other.clone(),
    })
}

fn holds_object(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(_) => true,
        serde_json::Value::Array(items) => items.iter().any(holds_object),
        _ => false,
    }
}

/// What a rename gives holds: its edits, in order, make its text from the expression's, every
/// span lies within the expression, and the text parses.
fn check_renamed(expression: &Expression, renamed: &Renamed) {
    let source = expression.source();
    let mut text = String::new();
    let mut at = 0;
    for edit in &renamed.edits {
        within(edit.span, source);
        assert!(edit.span.start >= at, "{source}: {:?}", renamed.edits);
        text.push_str(&source[at..edit.span.start]);
        text.push_str(&edit.text);
        at = edit.span.end;
    }
    text.push_str(&source[at..]);
    assert_eq!(text, renamed.text, "{source}");
    for unsure in &renamed.unsure {
        within(unsure.span, source);
    }
    let _ = serde_json::to_value(renamed).expect("JSON");
    if let Err(e) = Expression::compile(&renamed.text) {
        panic!("{source} gave {}: {e}", renamed.text);
    }
}

/// Renaming fields in an expression never panics, gives back the text with an empty table, and
/// gives a text that parses; renaming every field it reads, when the rename is sure of every
/// place, gives an expression that gives on the input with its keys renamed what the
/// expression gives (roadmap 0.37 #13 and #19).
///
/// The input is an expression, then optionally a NUL byte and the JSON input document (a fixed
/// one when there is none or it is not JSON), and a NUL byte and a table of renames, a line
/// for each old path followed by a line for its new one.
pub fn rename(data: &[u8]) {
    record(data);
    let [source, input, table] = parts(data);
    let (Ok(source), Ok(table)) = (std::str::from_utf8(source), std::str::from_utf8(table)) else {
        return;
    };
    let input = serde_json::from_slice(input).unwrap_or_else(|_| default_input());
    let lines: Vec<&str> = table.lines().collect();
    on_small_stack(|| {
        let Ok(expression) = Expression::compile(source) else {
            return;
        };
        let service = LanguageService::new(None);
        let none: [(&str, &str); 0] = [];
        let same = service.rename(&expression, none).expect("an empty table");
        assert_eq!(same.text, source);
        assert!(same.edits.is_empty() && same.unsure.is_empty());

        let given = lines
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[old, new]| (*old, *new));
        if let Ok(renamed) = service.rename(&expression, given) {
            check_renamed(&expression, &renamed);
        }

        let Some(read) = expression.reads().input().cloned() else {
            return;
        };
        let (mut table, mut renames) = (Vec::new(), HashMap::new());
        if !rename_all(&read, &mut Vec::new(), ("", ""), &mut table, &mut renames) {
            return;
        }
        let renamed = service
            .rename(&expression, table.iter().map(|(a, b)| (a, b)))
            .unwrap_or_else(|e| panic!("{e}: {table:?}"));
        check_renamed(&expression, &renamed);
        if !renamed.unsure.is_empty() {
            return;
        }
        let Some(moved) = renamed_keys(&input, &mut Vec::new(), &renames) else {
            return;
        };
        let rewritten = Expression::compile(&renamed.text).expect("parsed");
        let options = options();
        let bindings = Bindings::new();
        let before = expression.evaluate_with(Some(&input), &bindings, &options);
        let after = rewritten.evaluate_with(Some(&moved), &bindings, &options);
        if before
            .as_ref()
            .is_ok_and(|v| v.as_ref().is_some_and(holds_object))
        {
            return;
        }
        // A field called as a function, when a built-in has its name, fails with a code that
        // asks whether the built-in was meant, before its arguments are evaluated; once renamed
        // it is no built-in's name, so its arguments are evaluated first and may fail on their
        // own: it fails either way, with a code that may differ.
        if let Err(e) = &before
            && matches!(e.code(), "T1005" | "T1007")
        {
            assert!(after.is_err(), "{source} gave {}: {after:?}", renamed.text);
            return;
        }
        if let (Some(a), Some(b)) = (outcome(before), outcome(after)) {
            assert_eq!(a, b, "{source} gave {}", renamed.text);
        }
    });
}
