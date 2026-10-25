//! The evaluation options, made once from JSON text and known by handle, so that an evaluation
//! passes a `u64` instead of its options.

use std::time::Duration;

use jlens_core::Options;
use serde::Deserialize;

use crate::handles::Table;

/// The live options. `Options` is `Send` and `Sync`: one serves every thread.
pub(crate) static OPTIONS: Table<Options> = Table::new();

/// The options as JSON text gives them, with the names of the WebAssembly package. A field
/// that is absent or `null` keeps its default; an unknown one is an error, so that a misspelt
/// limit does not pass unnoticed.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Given {
    /// Milliseconds, not negative.
    timeout: Option<f64>,
    max_depth: Option<usize>,
    max_steps: Option<u64>,
    max_string_length: Option<usize>,
    max_array_length: Option<usize>,
    max_result_size: Option<usize>,
    /// Milliseconds since the epoch.
    now: Option<i64>,
    seed: Option<u64>,
}

/// Reads options from JSON text: an object with `timeout` (milliseconds), `maxDepth`,
/// `maxSteps`, `maxStringLength`, `maxArrayLength`, `maxResultSize`, `now` (milliseconds since
/// the epoch) and `seed`. The error is a message.
pub(crate) fn parse(bytes: &[u8]) -> Result<Options, String> {
    let given: Given = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let timeout = given
        .timeout
        .map(|ms| {
            Duration::try_from_secs_f64(ms / 1000.0)
                .map_err(|_| "timeout must be a non-negative number of milliseconds".to_owned())
        })
        .transpose()?;
    let mut options = Options::default();
    options.timeout = timeout;
    options.max_depth = given.max_depth;
    options.max_steps = given.max_steps;
    options.max_string_length = given.max_string_length;
    options.max_array_length = given.max_array_length;
    options.max_result_size = given.max_result_size;
    options.now = given.now;
    options.seed = given.seed;
    Ok(options)
}
