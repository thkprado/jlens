//! The `pictures` target: see `jlens_fuzz::targets::pictures`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    jlens_fuzz::targets::pictures(data);
});
