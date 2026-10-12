//! The bodies of the fuzz targets. Each `fuzz_targets/*.rs` calls one with libFuzzer's input,
//! and the `replay` program calls them with saved inputs, without libFuzzer.

use jlens_core::audit::AuditOptions;
use jlens_core::{Bindings, Document, Expression};
use jlens_schema::{FunctionDecl, LanguageService, Schema};
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
        let _ = expr.evaluate_document(&Document::new(&input), &bindings, &options);
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
    });
}
