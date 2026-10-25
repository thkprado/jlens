//! The safe functions behind the exports: expressions and their evaluations, and the replies.

use std::any::Any;

use jlens_core::reads::Read;
use jlens_core::{Bindings, Document, Error, Expression, Options};
use serde_json::{Map, Value};

use crate::handles::Table;
use crate::{REPLY_ERROR, REPLY_HANDLE, REPLY_JSON, REPLY_UNDEFINED, buffers};

/// A compiled expression, with its source (for the offsets of its errors) and the fields it
/// reads of its input (for the pruned parse of ADR 0016).
struct Compiled {
    expression: Expression,
    source: String,
    read: Read,
}

/// The live expressions. `Expression` is `Send` and `Sync`: one serves every thread.
static EXPRESSIONS: Table<Compiled> = Table::new();

/// Compiles the expression in buffer `source`, which stays the caller's, and replies with the
/// expression's handle or the compile error.
pub fn compile(source: u64) -> u64 {
    reply(compile_text(source))
}

fn compile_text(source: u64) -> Result<Vec<u8>, Vec<u8>> {
    let text = buffers::with(source, |bytes| {
        std::str::from_utf8(bytes).map(str::to_owned)
    })
    .ok_or_else(|| unknown("buffer", source))?
    .map_err(|e| failure(&Error::new("U2003").with("message", e.to_string()), ""))?;
    let expression = Expression::compile(&text).map_err(|e| failure(&e, &text))?;
    // An expression that does not read its input needs none of its fields.
    let read = expression
        .reads()
        .input()
        .cloned()
        .unwrap_or_else(|| Read::Fields(Default::default()));
    let compiled = Compiled {
        expression,
        source: text,
        read,
    };
    let handle = EXPRESSIONS.insert(compiled).ok_or_else(full)?;
    let mut reply = vec![REPLY_HANDLE];
    reply.extend_from_slice(&handle.to_le_bytes());
    Ok(reply)
}

/// Frees an expression: 1 when it was live, 0 for an unknown or freed handle. An evaluation
/// running on another thread keeps it until it returns.
pub fn expression_free(expression: u64) -> u32 {
    u32::from(EXPRESSIONS.remove(expression).is_some())
}

/// Evaluates expression `expression` against the JSON text in buffer `input` (0 for no
/// input), which stays the caller's, and replies with the result or the error. The fields of
/// the input that the expression does not read are skipped while parsing.
pub fn evaluate(expression: u64, input: u64) -> u64 {
    reply(evaluate_text(expression, input))
}

fn evaluate_text(expression: u64, input: u64) -> Result<Vec<u8>, Vec<u8>> {
    let compiled = EXPRESSIONS
        .get(expression)
        .ok_or_else(|| unknown("expression", expression))?;
    let document = match input {
        0 => None,
        input => Some(
            buffers::with(input, |bytes| parse(bytes, &compiled.read))
                .ok_or_else(|| unknown("buffer", input))??,
        ),
    };
    let (bindings, options) = (Bindings::new(), Options::default());
    let result = match &document {
        Some(document) => compiled
            .expression
            .evaluate_document(document, &bindings, &options),
        None => compiled.expression.evaluate_with(None, &bindings, &options),
    };
    match result.map_err(|e| failure(&e, &compiled.source))? {
        Some(value) => {
            let text = jlens_core::format_json(&value, false);
            let mut reply = Vec::with_capacity(1 + text.len());
            reply.push(REPLY_JSON);
            reply.extend_from_slice(text.as_bytes());
            Ok(reply)
        }
        None => Ok(vec![REPLY_UNDEFINED]),
    }
}

/// Parses JSON text for the fields `read` names; text that is not UTF-8 is not JSON either
/// (RFC 8259).
fn parse(bytes: &[u8], read: &Read) -> Result<Document, Vec<u8>> {
    let invalid = |message: String| failure(&Error::new("U2001").with("message", message), "");
    let text = std::str::from_utf8(bytes).map_err(|e| invalid(e.to_string()))?;
    Document::parse_cut(text, read).map_err(|e| invalid(e.to_string()))
}

/// The reply of a panic caught at an export: a `U2004` error with the panic's message.
pub fn panicked(payload: Box<dyn Any + Send>) -> u64 {
    let message = match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => (*message).to_owned(),
            Err(_) => "a panic".to_owned(),
        },
    };
    buffers::insert(failure(&Error::new("U2004").with("message", message), ""))
}

fn reply(reply: Result<Vec<u8>, Vec<u8>>) -> u64 {
    buffers::insert(reply.unwrap_or_else(|error| error))
}

/// The `U2002` reply for a handle that names no live object of its `kind`.
fn unknown(kind: &str, handle: u64) -> Vec<u8> {
    failure(
        &Error::new("U2002")
            .with("token", kind)
            .with("value", handle),
        "",
    )
}

/// The `U1001` reply for a table of handles that is full.
fn full() -> Vec<u8> {
    failure(
        &Error::new("U1001").with("message", "Too many live objects in the C API"),
        "",
    )
}

/// The reply of an error, its offsets converted to UTF-16 against `source`, the expression's.
fn failure(error: &Error, source: &str) -> Vec<u8> {
    let mut envelope = Map::new();
    envelope.insert("code".into(), error.code().into());
    envelope.insert("message".into(), error.message().into());
    if let Some(position) = error.position() {
        envelope.insert("position".into(), position.into());
    }
    if let Some(span) = error.span() {
        envelope.insert("start".into(), utf16(source, span.start).into());
        envelope.insert("end".into(), utf16(source, span.end).into());
    }
    if let Some(token) = error.token() {
        envelope.insert("token".into(), token.into());
    }
    if let Some(value) = error.value() {
        envelope.insert("value".into(), value.clone());
    }
    let mut reply = vec![REPLY_ERROR];
    reply.extend_from_slice(jlens_core::format_json(&Value::Object(envelope), false).as_bytes());
    reply
}

/// The offset in UTF-16 code units of byte offset `byte` of `text`.
fn utf16(text: &str, byte: usize) -> usize {
    text.char_indices()
        .take_while(|&(at, _)| at < byte)
        .map(|(_, c)| c.len_utf16())
        .sum()
}
