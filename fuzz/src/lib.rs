//! What the fuzz targets share: a small stack, the limits of an evaluation and its bindings, and
//! the bodies of the targets (`targets`).

pub mod targets;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use jlens_core::{Bindings, Error, HostFunction, Options};
use serde_json::json;

/// The stack of the thread a target runs on: 1 MB, the size of the main thread's stack on
/// Windows. Linux gives the main thread 8 MB, which would hide recursion that does not go
/// through `jlens_core::stack::grow`.
const STACK: usize = 1024 * 1024;

/// Writes the input about to run to the file that `JLENS_FUZZ_LAST_INPUT` names, if set. A stack
/// overflow on the target's thread kills the process before libFuzzer can write the input that
/// caused it, so the Fuzz workflow keeps this file instead.
pub fn record(data: &[u8]) {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    let path = PATH.get_or_init(|| std::env::var_os("JLENS_FUZZ_LAST_INPUT").map(PathBuf::from));
    if let Some(path) = path {
        let _ = std::fs::write(path, data);
    }
}

/// Runs `f` on a thread with a 1 MB stack. A panic on that thread is a crash of the target.
pub fn on_small_stack<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    std::thread::scope(|scope| {
        let thread = std::thread::Builder::new()
            .stack_size(STACK)
            .spawn_scoped(scope, f)
            .expect("spawn the target's thread");
        thread
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

/// Limits that keep every evaluation short and small, and a fixed clock and seed, so that a
/// crash reproduces.
pub fn options() -> Options {
    Options::new()
        .timeout(Duration::from_millis(100))
        .max_depth(300)
        .max_string_length(1 << 20)
        .max_array_length(100_000)
        .max_result_size(1 << 20)
        .now(0)
        .seed(1)
}

/// A variable and host functions: one that returns its arguments, one with a signature, one
/// that fails.
pub fn bindings() -> Bindings {
    Bindings::new()
        .var("x", json!([1, "two", null]))
        .function(
            "echo",
            HostFunction::new(2, |args, call| {
                Ok(Some(json!({"args": args, "input": call.input()})))
            }),
        )
        .function(
            "checked",
            HostFunction::with_signature("<n-a<s>?f?:a>", |args, _| Ok(Some(json!(args))))
                .expect("valid signature"),
        )
        .function(
            "fail",
            HostFunction::new(0, |_, _| Err(Error::new("X0001").with("message", "fails"))),
        )
}

/// The input of an evaluation whose fuzz input carries none.
pub fn default_input() -> serde_json::Value {
    json!({
        "a": {"b": [1, 2, {"b": "x"}]},
        "items": [{"price": 2, "a": "p"}, {"price": 3.5, "a": null}, {"price": "4"}],
        "s": "hello 𝄞"
    })
}
