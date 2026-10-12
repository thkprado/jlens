//! The `language_service` target: see `jlens_fuzz::targets::language_service`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    jlens_fuzz::targets::language_service(data);
});
