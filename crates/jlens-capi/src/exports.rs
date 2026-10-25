//! The C ABI. The only module where `unsafe_code` is allowed, for `#[unsafe(no_mangle)]`: each
//! export is written by `export!`, whose body is one guarded call to a safe function, so no
//! `unsafe` block fits here. Calling these from Rust is safe; they are public so that the tests
//! go through them.

macro_rules! export {
    ($($(#[$meta:meta])* $name:ident($($arg:ident: $ty:ty),*) -> $ret:ty = $target:path, $fallback:path;)*) => {
        $(
            $(#[$meta])*
            #[unsafe(no_mangle)]
            pub extern "C" fn $name($($arg: $ty),*) -> $ret {
                crate::guarded($fallback, || $target($($arg),*))
            }
        )*
    };
}

export! {
    /// Creates a buffer of `len` zero bytes: its handle, or 0 when it cannot be allocated.
    jlens_buffer_new(len: usize) -> u64 = crate::buffers::create, crate::zero;
    /// The address of a buffer's bytes, valid until the buffer is freed (0 for an unknown
    /// handle).
    jlens_buffer_address(buffer: u64) -> usize = crate::buffers::address, crate::zero;
    /// The length of a buffer (0 for an unknown handle).
    jlens_buffer_len(buffer: u64) -> usize = crate::buffers::len, crate::zero;
    /// Frees a buffer: 1 when it was live, 0 for an unknown or freed handle.
    jlens_buffer_free(buffer: u64) -> u32 = crate::buffers::free, crate::zero;
    /// Compiles the UTF-8 expression in buffer `source`: a reply with the expression's handle
    /// or the error.
    jlens_compile(source: u64) -> u64 = crate::api::compile, crate::api::panicked;
    /// Frees an expression: 1 when it was live, 0 for an unknown or freed handle.
    jlens_expression_free(expression: u64) -> u32 = crate::api::expression_free, crate::zero;
    /// Evaluates an expression against the JSON text in buffer `input` (0 for no input): a
    /// reply with the result or the error.
    jlens_evaluate(expression: u64, input: u64) -> u64 = crate::api::evaluate, crate::api::panicked;
}
