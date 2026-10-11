//! Strict parsing never panics.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|source: &str| {
    jlens_fuzz::record(source.as_bytes());
    jlens_fuzz::on_small_stack(|| {
        let _ = jlens_core::parse(source);
    });
});
