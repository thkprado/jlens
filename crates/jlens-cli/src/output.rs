//! Writing results to stdout.

use std::io::Write;

use anyhow::Result;
use serde_json::Value;

/// Writes one line to stdout. Errors (including a closed pipe, as with `| head`) are returned.
pub fn line(text: &str) -> Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{text}")?;
    Ok(())
}

/// Whether an error comes from writing to a closed pipe; `jlens` then exits quietly.
pub fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|e| {
        e.downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::BrokenPipe)
    })
}

/// How results are printed.
#[derive(Clone, Copy)]
pub struct Format {
    /// One line per result instead of indented JSON.
    pub compact: bool,
    /// Strings without quotes.
    pub raw: bool,
}

impl Format {
    /// Renders a result.
    pub fn render(self, value: &Value) -> String {
        match value {
            Value::String(s) if self.raw => s.clone(),
            other => jlens_core::format_json(other, !self.compact),
        }
    }
}

/// JSONata's effective boolean value (`$boolean`) of a JSON result; undefined is false.
pub fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Array(items)) => match items.as_slice() {
            [single] => truthy(Some(single)),
            many => many.iter().any(|v| truthy(Some(v))),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn truthiness_follows_jsonata() {
        assert!(!truthy(None));
        assert!(!truthy(Some(&json!([]))));
        assert!(!truthy(Some(&json!([0]))));
        assert!(truthy(Some(&json!([0, "a"]))));
        assert!(!truthy(Some(&json!({}))));
        assert!(truthy(Some(&json!("x"))));
    }
}
