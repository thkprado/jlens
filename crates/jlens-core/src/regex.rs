//! Regular expressions with JavaScript semantics, backed by `regress` (see ADR 0002).

use crate::error::{Error, Result};

/// A compiled regular expression literal.
#[allow(dead_code)] // used by the regex functions (phase 3)
pub(crate) struct Regex {
    pub re: regress::Regex,
    pub pattern: String,
    pub flags: String,
}

impl Regex {
    /// Compiles `/pattern/flags`. Flags are `i` and `m`; matching is always global, as in
    /// jsonata-js.
    pub fn new(pattern: &str, flags: &str) -> Result<Regex> {
        let re = regress::Regex::with_flags(pattern, flags).map_err(|e| {
            // jsonata-js surfaces the JavaScript SyntaxError; jlens reports it as a regex
            // syntax error.
            Error::new("S0302")
                .with("value", pattern)
                .with("message", e.to_string())
        })?;
        Ok(Regex {
            re,
            pattern: pattern.to_owned(),
            flags: flags.to_owned(),
        })
    }
}
