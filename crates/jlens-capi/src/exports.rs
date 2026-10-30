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
    /// What an expression reads: a reply with its reads as JSON text, or the error.
    jlens_reads(expression: u64) -> u64 = crate::api::reads, crate::api::panicked;
    /// The union of the reads in buffer `reads`, a JSON array: a reply with the union as JSON
    /// text, or the error.
    jlens_union_reads(reads: u64) -> u64 = crate::api::union_reads, crate::api::panicked;
    /// Makes options from the JSON text in buffer `options`: a reply with their handle or the
    /// error.
    jlens_options_new(options: u64) -> u64 = crate::api::options_new, crate::api::panicked;
    /// Frees options: 1 when they were live, 0 for an unknown or freed handle.
    jlens_options_free(options: u64) -> u32 = crate::api::options_free, crate::zero;
    /// Parses the JSON text in buffer `text` into a document, keeping the fields of the read in
    /// buffer `read` (0 for all): a reply with the document's handle or the error.
    jlens_document_new(text: u64, read: u64) -> u64 = crate::api::document_new, crate::api::panicked;
    /// Frees a document: 1 when it was live, 0 for an unknown or freed handle.
    jlens_document_free(document: u64) -> u32 = crate::api::document_free, crate::zero;
    /// Makes bindings of the host functions in buffer `functions` (JSON text, 0 for none), run
    /// by `function` given `context`, and of the variables in buffer `variables` (values, 0 for
    /// none), as the `host` module describes: a reply with their handle or the error.
    jlens_bindings_new(functions: u64, variables: u64, function: Option<crate::host::Function>, context: u64) -> u64 = crate::host::bindings_new, crate::api::panicked;
    /// Frees bindings: 1 when they were live, 0 for an unknown or freed handle.
    jlens_bindings_free(bindings: u64) -> u32 = crate::host::bindings_free, crate::zero;
    /// Calls argument `index`, a function, of the host function's call `frame`, with the
    /// arguments in buffer `arguments` (values, 0 for none): a reply with the result as values,
    /// or the error.
    jlens_call(frame: u64, index: u32, arguments: u64) -> u64 = crate::host::call, crate::api::panicked;
    /// Evaluates an expression against the JSON text in buffer `input` (0 for no input), with
    /// bindings `bindings` (0 for none) and options `options` (0 for the defaults): a reply with
    /// the result, as JSON text (`result` 0) or values (`result` 1), or the error, in buffer
    /// `scratch` when it fits (0 for none; see the crate's Replies).
    jlens_evaluate(expression: u64, input: u64, bindings: u64, options: u64, result: u32, scratch: u64) -> u64 = crate::api::evaluate_into, crate::api::panicked;
    /// Evaluates an expression against a document, with bindings `bindings` (0 for none) and
    /// options `options` (0 for the defaults): a reply with the result, as JSON text (`result`
    /// 0) or values (`result` 1), or the error, in buffer `scratch` when it fits.
    jlens_evaluate_document(expression: u64, document: u64, bindings: u64, options: u64, result: u32, scratch: u64) -> u64 = crate::api::evaluate_document_into, crate::api::panicked;
    /// Evaluates an expression against the host's own values: buffer `input` holds the input
    /// as a level, and `reader`, given `context`, writes the levels of its arrays and objects
    /// (the `source` module). With bindings `bindings` (0 for none) and options `options` (0
    /// for the defaults): a reply with the result, as JSON text (`result` 0) or values
    /// (`result` 1), or the error, in buffer `scratch` when it fits.
    jlens_evaluate_source(expression: u64, input: u64, reader: Option<crate::source::Reader>, context: u64, bindings: u64, options: u64, result: u32, scratch: u64) -> u64 = crate::api::evaluate_source_into, crate::api::panicked;
    /// Evaluates an expression against the values in buffer `input` (0 for no input), with
    /// bindings `bindings` (0 for none), options `options` (0 for the defaults) and the audit
    /// options in buffer `audit`, JSON text (0 for the defaults), recording how the result was
    /// reached: a reply with the result, its error, the audit tree and its explanation as values
    /// (the `audit` module), or the error of an argument.
    jlens_evaluate_audited(expression: u64, input: u64, bindings: u64, options: u64, audit: u64) -> u64 = crate::audit::evaluate_audited, crate::api::panicked;
}
