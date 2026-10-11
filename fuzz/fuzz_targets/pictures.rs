//! The pictures of `$formatNumber`, `$formatInteger`, `$parseInteger`, `$fromMillis` and
//! `$toMillis` never make them panic.
//!
//! The fuzz input is a picture, optionally followed by a NUL byte and a second string: the text
//! that `$parseInteger` and `$toMillis` parse, and the time zone of `$fromMillis`.

#![no_main]

use jlens_core::{Bindings, Expression};
use libfuzzer_sys::fuzz_target;
use serde_json::json;

/// Each call is evaluated on its own, so that one that fails does not hide the others.
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

fuzz_target!(|data: &[u8]| {
    jlens_fuzz::record(data);
    let (picture, text) = match data.iter().position(|&b| b == 0) {
        Some(nul) => (&data[..nul], &data[nul + 1..]),
        None => (data, &[][..]),
    };
    let (Ok(picture), Ok(text)) = (std::str::from_utf8(picture), std::str::from_utf8(text)) else {
        return;
    };
    jlens_fuzz::on_small_stack(|| {
        let bindings = Bindings::new()
            .var("picture", json!(picture))
            .var("text", json!(text));
        let options = jlens_fuzz::options();
        for call in CALLS {
            let expr = Expression::compile(call).expect("valid expression");
            let _ = expr.evaluate_with(None, &bindings, &options);
        }
    });
});
