//! Writing results to stdout.

use std::io::Write;

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

use crate::report;

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

/// The `--json` envelope of a successful evaluation (ADR 0008). `line` is the input line, with
/// `--ndjson`.
pub fn ok_envelope(value: Option<&Value>, line: Option<usize>) -> String {
    let line = line.map_or_else(String::new, |n| format!(r#","line":{n}"#));
    match value {
        // `format_json` prints numbers as jsonata-js does, which `serde_json` would not.
        Some(value) => format!(
            r#"{{"ok":true{line},"result":{}}}"#,
            jlens_core::format_json(value, false)
        ),
        None => format!(r#"{{"ok":true{line},"undefined":true}}"#),
    }
}

/// The `--json` envelope of a failed evaluation (ADR 0008). `source` is the expression, in
/// which the error's line and column are computed; `line` is the input line, with `--ndjson`.
pub fn error_envelope(err: &jlens_core::Error, source: &str, line: Option<usize>) -> String {
    #[derive(Serialize)]
    struct Envelope<'a> {
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
        error: ErrorInfo<'a>,
    }
    #[derive(Serialize)]
    struct ErrorInfo<'a> {
        code: &'a str,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        token: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<&'a Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        position: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        column: Option<usize>,
    }
    let location = err.span().map(|span| report::location(source, span.start));
    let envelope = Envelope {
        ok: false,
        line,
        error: ErrorInfo {
            code: err.code(),
            message: err.message(),
            token: err.token(),
            value: err.value(),
            position: err.position(),
            line: location.map(|(line, _)| line),
            column: location.map(|(_, column)| column),
        },
    };
    serde_json::to_string(&envelope).expect("the envelope serializes")
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

    #[test]
    fn envelopes() {
        assert_eq!(
            ok_envelope(Some(&json!([1e21, null])), None),
            r#"{"ok":true,"result":[1e+21,null]}"#
        );
        assert_eq!(ok_envelope(None, None), r#"{"ok":true,"undefined":true}"#);
        assert_eq!(
            ok_envelope(Some(&json!(1)), Some(3)),
            r#"{"ok":true,"line":3,"result":1}"#
        );
        assert_eq!(
            ok_envelope(None, Some(4)),
            r#"{"ok":true,"line":4,"undefined":true}"#
        );
        let source = "(\n  \"é\" + 1\n)";
        let err = jlens_core::Expression::compile(source)
            .unwrap()
            .evaluate(None, &jlens_core::Bindings::new())
            .unwrap_err();
        let envelope: Value = serde_json::from_str(&error_envelope(&err, source, None)).unwrap();
        assert_eq!(envelope["error"]["code"], "T2001");
        assert_eq!(envelope["error"]["line"], 2);
        assert_eq!(envelope["error"]["column"], 3);
        let input = jlens_core::Error::new("U2001").with("message", "oops");
        assert_eq!(
            error_envelope(&input, "", None),
            r#"{"ok":false,"error":{"code":"U2001","message":"The input is not valid JSON: oops"}}"#
        );
        assert_eq!(
            error_envelope(&input, "", Some(5)),
            r#"{"ok":false,"line":5,"error":{"code":"U2001","message":"The input is not valid JSON: oops"}}"#
        );
    }
}
