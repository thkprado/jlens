//! The C API of jlens (ADR 0021): the exports the Java package calls through Panama, kept free
//! of Java specifics so that other languages that call C could use them too.
//!
//! No Rust code dereferences a pointer it is given. Memory crosses the boundary in buffers
//! owned by Rust and known by handle ([`buffers`]): the host asks for a buffer of `n` bytes,
//! writes into it through the address Rust exposes, and passes the handle.
//!
//! # Handles
//!
//! A handle is a `u64`, never 0, that names a live buffer or expression. It carries the
//! generation of its slot, so a handle that was freed stays unknown after its slot is reused:
//! using it, or freeing it twice, fails with `U2002` instead of reaching another object.
//! The tables of handles are global, since a virtual thread can move to another carrier thread
//! between two calls, and they are locked only to find an entry ([`handles`]).
//!
//! # Replies
//!
//! The exports that can fail return the handle of a new buffer, the reply, which the caller
//! reads and frees. Its first byte tells what follows:
//!
//! | Byte | Constant | What follows |
//! |---|---|---|
//! | 0 | [`REPLY_UNDEFINED`] | nothing: the result is undefined |
//! | 1 | [`REPLY_JSON`] | the result, as UTF-8 JSON text |
//! | 2 | [`REPLY_ERROR`] | the error envelope, as UTF-8 JSON text |
//! | 3 | [`REPLY_HANDLE`] | a handle, as 8 bytes little-endian |
//! | 4 | [`REPLY_VALUES`] | the result, as values ([`values`]) |
//!
//! The envelope is an object with the error's `code` and `message`, and, when the error has
//! them, `position`, `start`, `end` (the range of the expression's source), `token` and
//! `value`. Its offsets count UTF-16 code units, as `position` does in jsonata-js. The C API's
//! own failures use `U` codes: `U2001` for input that is not valid JSON (or not UTF-8),
//! `U2002` for an unknown or freed handle, `U2003` for an expression that is not UTF-8,
//! `U2004` for a panic, `U2005` for options or reads that are not in their form (its `token`
//! says which: `options`, `read` or `reads`), and `U1001` when a table of handles is full. An
//! unknown handle is reported before anything wrong with what the other handles hold. A reply
//! handle of 0 means that no buffer could be made for the reply.
//!
//! # Objects
//!
//! Besides buffers, the host holds handles of expressions, options and documents, each freed
//! by its own export. Options are made once from JSON text, with the names of the WebAssembly
//! package's ([`api::options_new`]), and an evaluation takes their handle, or 0 for the
//! defaults. A document is JSON text parsed once, kept behind an `Arc` and converted once per
//! thread that evaluates against it ([`documents`]); it can be pruned to the fields that
//! [`api::reads`] or [`api::union_reads`] give.
//!
//! # Panics
//!
//! A panic that unwinds out of an `extern "C"` function aborts the process, the JVM included.
//! Every export catches it ([`guarded`]): the exports that reply return a `U2004` error, the
//! others return 0. A panic is a bug in jlens.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

// Evaluation allocates for most of its work, and Windows' system allocator made the spike's
// large cases up to 2.7 times slower than on Linux (ADR 0021). `local_dynamic_tls`, since the
// library is loaded at run time, as the Python package's is (ADR 0014, amendment).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod api;
pub mod buffers;
pub mod documents;
// The crate's one exception to `unsafe_code = "deny"`, for `#[unsafe(no_mangle)]` (ADR 0021).
#[allow(unsafe_code)]
pub mod exports;
pub mod handles;
mod options;
pub mod values;

/// The first byte of a reply whose result is undefined.
pub const REPLY_UNDEFINED: u8 = 0;
/// The first byte of a reply holding the result as JSON text.
pub const REPLY_JSON: u8 = 1;
/// The first byte of a reply holding an error envelope.
pub const REPLY_ERROR: u8 = 2;
/// The first byte of a reply holding a handle.
pub const REPLY_HANDLE: u8 = 3;
/// The first byte of a reply holding the result as values ([`values`]).
pub const REPLY_VALUES: u8 = 4;

/// Runs `f`, and `fallback` with the panic's payload if `f` panics, so that no panic unwinds
/// into the caller of an export. First, this thread drops its conversions of the documents
/// freed since its last call ([`documents`]).
pub fn guarded<R>(fallback: impl FnOnce(Box<dyn Any + Send>) -> R, f: impl FnOnce() -> R) -> R {
    // Unwind safety: what a panic can leave half-done is the evaluation's own state, which is
    // dropped; the tables take their locks back from a poisoned mutex, and are never left in
    // the middle of an update.
    catch_unwind(AssertUnwindSafe(|| {
        documents::sweep_if_freed();
        f()
    }))
    .unwrap_or_else(fallback)
}

/// The fallback of the exports that do not reply: 0.
pub fn zero<R: Default>(_payload: Box<dyn Any + Send>) -> R {
    R::default()
}
