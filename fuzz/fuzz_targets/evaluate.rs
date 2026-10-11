//! Compiling and evaluating never panic, under limits.
//!
//! The fuzz input is an expression, optionally followed by a NUL byte and the JSON input
//! document. Without a document, or when it is not JSON, the evaluation reads a fixed one.

#![no_main]

use jlens_core::audit::AuditOptions;
use jlens_core::{Document, Expression};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    jlens_fuzz::record(data);
    let (source, input) = match data.iter().position(|&b| b == 0) {
        Some(nul) => (&data[..nul], serde_json::from_slice(&data[nul + 1..]).ok()),
        None => (data, None),
    };
    let Ok(source) = std::str::from_utf8(source) else {
        return;
    };
    let input = input.unwrap_or_else(jlens_fuzz::default_input);
    jlens_fuzz::on_small_stack(|| {
        let Ok(expr) = Expression::compile(source) else {
            return;
        };
        let options = jlens_fuzz::options();
        let bindings = jlens_fuzz::bindings();
        let _ = expr.evaluate_with(Some(&input), &bindings, &options);
        let _ = expr.evaluate_with(None, &bindings, &options);
        let _ = expr.evaluate_document(&Document::new(&input), &bindings, &options);
        let _ = expr.evaluate_audited(Some(&input), &bindings, &options, &AuditOptions::full());
    });
});
