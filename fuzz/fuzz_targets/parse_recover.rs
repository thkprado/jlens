//! Recovering parsing never panics, and its diagnostics lie within the source.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|source: &str| {
    jlens_fuzz::on_small_stack(|| {
        let ast = jlens_core::parse_recover(source);
        for d in &ast.diagnostics {
            assert!(
                d.span.start <= d.span.end && d.span.end <= source.len(),
                "{d:?}"
            );
        }
    });
});
