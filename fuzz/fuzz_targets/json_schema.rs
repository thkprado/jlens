//! The `json_schema` target: see `jlens_fuzz::targets::json_schema`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    jlens_fuzz::targets::json_schema(data);
});
