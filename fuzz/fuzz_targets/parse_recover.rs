//! The `parse_recover` target: see `jlens_fuzz::targets::parse_recover`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    jlens_fuzz::targets::parse_recover(data);
});
