//! The language service never panics on any text, at any offset, and its ranges lie within the
//! text.
//!
//! The first two bytes of the fuzz input give the offset (little-endian, modulo the length of the
//! text plus two, so that it can fall inside a character or past the end). The rest is the text.

#![no_main]

use jlens_schema::{FunctionDecl, LanguageService, Schema};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    jlens_fuzz::record(data);
    let Some((offset, text)) = data.split_first_chunk::<2>() else {
        return;
    };
    let Ok(text) = std::str::from_utf8(text) else {
        return;
    };
    let offset = usize::from(u16::from_le_bytes(*offset)) % (text.len() + 2);
    jlens_fuzz::on_small_stack(|| {
        let schema = Schema::infer(&[jlens_fuzz::default_input()]);
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
});
