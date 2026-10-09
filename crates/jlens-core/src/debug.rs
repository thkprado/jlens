//! Inspection helpers for the parsing pipeline, used by `jlens ast` and in tests.
//!
//! Modelled on the `Dump` tool from the jsonata-java walkthrough: they show the token stream,
//! the raw parse tree before post-processing, and the processed tree.

use serde::Serialize;
use serde_json::Value as JsonValue;

use crate::ast::Span;
use crate::error::{Error, Result};
use crate::lexer::{Lexer, TokKind};

/// A token as shown by [`tokens`].
#[derive(Debug, Clone, Serialize)]
pub struct TokenInfo {
    /// `name`, `variable`, `string`, `number`, `value`, `regex` or `operator`.
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// The token's value.
    pub value: JsonValue,
    /// Byte range in the source.
    pub span: Span,
}

/// Tokenizes `source`.
///
/// The parser tells the lexer whether `/` may start a regular expression. This function
/// approximates that context: `/` is a division after an operand or a closing bracket, and a
/// regex anywhere else.
pub fn tokens(source: &str) -> Result<Vec<TokenInfo>> {
    let mut lexer = Lexer::new(source);
    let mut out = Vec::new();
    let mut infix = false;
    while let Some(tok) = lexer.next(infix)? {
        let (kind, value, operand) = match tok.kind {
            TokKind::Name(n) => ("name", n.into(), true),
            TokKind::Variable(v) => ("variable", v.into(), true),
            TokKind::Str(s) => ("string", s.into(), true),
            TokKind::Number(n) => ("number", n.into(), true),
            TokKind::Value(lit) => ("value", serde_json::to_value(lit).unwrap_or_default(), true),
            TokKind::Regex { pattern, flags } => {
                ("regex", format!("/{pattern}/{flags}").into(), true)
            }
            TokKind::Op(op) => ("operator", op.into(), matches!(op, ")" | "]" | "}")),
        };
        infix = operand;
        out.push(TokenInfo {
            kind,
            value,
            span: tok.span,
        });
    }
    Ok(out)
}

/// The raw parse tree (before post-processing) as JSON, together with any recovered errors.
pub fn raw_tree(source: &str, recover: bool) -> Result<(JsonValue, Vec<Error>)> {
    let (raw, errors, _warnings) = crate::parser::parse(source, recover);
    let raw = raw?;
    Ok((serde_json::to_value(&raw).unwrap_or_default(), errors))
}
