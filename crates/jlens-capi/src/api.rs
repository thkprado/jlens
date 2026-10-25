//! The safe functions behind the exports: expressions, documents, options and evaluations, and
//! the replies.

use std::any::Any;
use std::sync::{Arc, LazyLock};

use jlens_core::input::{Sink, Source};
use jlens_core::output::Output;
use jlens_core::reads::{Read, Reads};
use jlens_core::{Bindings, Document, Error, Expression, Options};
use serde_json::{Map, Value};

use crate::documents::{self, DOCUMENTS};
use crate::handles::Table;
use crate::options::{self, OPTIONS};
use crate::values::Encoder;
use crate::{REPLY_ERROR, REPLY_HANDLE, REPLY_JSON, REPLY_UNDEFINED, REPLY_VALUES, buffers};

/// A compiled expression, with its source (for the offsets of its errors) and the fields it
/// reads of its input (for the pruned parse of ADR 0016).
struct Compiled {
    expression: Expression,
    source: String,
    reads: Reads,
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
    let reads = expression.reads();
    // An expression that does not read its input needs none of its fields.
    let read = reads
        .input()
        .cloned()
        .unwrap_or_else(|| Read::Fields(Default::default()));
    let compiled = Compiled {
        expression,
        source: text,
        reads,
        read,
    };
    handle_reply(EXPRESSIONS.insert(compiled))
}

/// Frees an expression: 1 when it was live, 0 for an unknown or freed handle. An evaluation
/// running on another thread keeps it until it returns.
pub fn expression_free(expression: u64) -> u32 {
    u32::from(EXPRESSIONS.remove(expression).is_some())
}

/// Replies with what expression `expression` reads (ADR 0016), as `Expression::reads` gives
/// it: an object whose `input` is the `read` of `document_new`.
pub fn reads(expression: u64) -> u64 {
    reply(
        EXPRESSIONS
            .get(expression)
            .ok_or_else(|| unknown("expression", expression))
            .map(|compiled| json_reply(&serde_json::to_value(&compiled.reads).unwrap_or_default())),
    )
}

/// Replies with the union of the reads in buffer `reads`, a JSON array of what `reads` replies
/// with: what any of several expressions reads.
pub fn union_reads(reads: u64) -> u64 {
    reply(union_text(reads))
}

fn union_text(reads: u64) -> Result<Vec<u8>, Vec<u8>> {
    let all: Vec<Reads> = buffers::with(reads, |bytes| serde_json::from_slice(bytes))
        .ok_or_else(|| unknown("buffer", reads))?
        .map_err(|e| invalid("reads", e.to_string()))?;
    let union = all
        .iter()
        .fold(Reads::default(), |union, reads| union.union(reads));
    Ok(json_reply(
        &serde_json::to_value(&union).unwrap_or_default(),
    ))
}

/// Makes options from the JSON text in buffer `options`, which stays the caller's, and replies
/// with their handle or a `U2005` error. The text is an object with `timeout` (milliseconds),
/// `maxDepth`, `maxSteps`, `maxStringLength`, `maxArrayLength`, `maxResultSize`, `now`
/// (milliseconds since the epoch) and `seed`, each optional.
pub fn options_new(options: u64) -> u64 {
    reply(
        buffers::with(options, |bytes| options::parse(bytes))
            .ok_or_else(|| unknown("buffer", options))
            .and_then(|parsed| parsed.map_err(|message| invalid("options", message)))
            .and_then(|parsed| handle_reply(OPTIONS.insert(parsed))),
    )
}

/// Frees options: 1 when they were live, 0 for an unknown or freed handle.
pub fn options_free(options: u64) -> u32 {
    u32::from(OPTIONS.remove(options).is_some())
}

/// Parses the JSON text in buffer `text` into a document, keeping the fields of the JSON
/// `read` in buffer `read` (the `input` of `reads` or `union_reads`; 0 keeps them all), and
/// replies with its handle or the error. Both buffers stay the caller's. This thread keeps
/// its conversion; each other thread converts the text the first time it evaluates against it.
pub fn document_new(text: u64, read: u64) -> u64 {
    reply(document_text(text, read))
}

fn document_text(text: u64, read: u64) -> Result<Vec<u8>, Vec<u8>> {
    // The handles are checked before what they hold, so that an unknown one is always `U2002`.
    let text = buffers::with(text, |bytes| String::from_utf8(bytes.clone()))
        .ok_or_else(|| unknown("buffer", text))?;
    let read = match read {
        0 => None,
        read => Some(
            buffers::with(read, |bytes| serde_json::from_slice::<Read>(bytes))
                .ok_or_else(|| unknown("buffer", read))?
                .map_err(|e| invalid("read", e.to_string()))?,
        ),
    };
    let text = text.map_err(|e| not_json(e.utf8_error().to_string()))?;
    let text = Arc::new(documents::Text::new(text, read));
    // Parsed now, to fail on text that is not JSON.
    documents::converted(&text).map_err(|e| not_json(e.to_string()))?;
    let Some(handle) = DOCUMENTS.insert_arc(text) else {
        documents::sweep_if_freed();
        return Err(full());
    };
    handle_reply(Some(handle))
}

/// Frees a document: 1 when it was live, 0 for an unknown or freed handle. An evaluation
/// running on another thread keeps it until it returns.
pub fn document_free(document: u64) -> u32 {
    let freed = DOCUMENTS.remove(document);
    let live = freed.is_some();
    drop(freed);
    documents::sweep_if_freed();
    u32::from(live)
}

/// How an evaluation gives its result.
enum Format {
    /// As JSON text.
    Json,
    /// As values ([`crate::values`]).
    Values,
}

impl Format {
    /// The format named by an export's `result` argument: 0 for JSON text, 1 for values.
    fn named(result: u32) -> Result<Format, Vec<u8>> {
        match result {
            0 => Ok(Format::Json),
            1 => Ok(Format::Values),
            _ => Err(invalid("result", format!("no result format {result}"))),
        }
    }
}

/// Evaluates expression `expression` against the JSON text in buffer `input` (0 for no
/// input), which stays the caller's, with options `options` (0 for the defaults), and replies
/// with the result as JSON text (`result` 0) or as values (`result` 1), or with the error. The
/// fields of the input that the expression does not read are skipped while parsing.
pub fn evaluate(expression: u64, input: u64, options: u64, result: u32) -> u64 {
    reply(
        Format::named(result).and_then(|format| evaluate_text(expression, input, options, format)),
    )
}

fn evaluate_text(
    expression: u64,
    input: u64,
    options: u64,
    format: Format,
) -> Result<Vec<u8>, Vec<u8>> {
    let compiled = EXPRESSIONS
        .get(expression)
        .ok_or_else(|| unknown("expression", expression))?;
    let options = find_options(options)?;
    let document = match input {
        0 => None,
        input => Some(
            buffers::with(input, |bytes| parse(bytes, &compiled.read))
                .ok_or_else(|| unknown("buffer", input))??,
        ),
    };
    run(
        &compiled,
        document.as_ref(),
        options.as_deref().unwrap_or(&DEFAULTS),
        format,
    )
}

/// Evaluates expression `expression` against document `document` with options `options` (0
/// for the defaults), and replies with the result as JSON text (`result` 0) or as values
/// (`result` 1), or with the error.
pub fn evaluate_document(expression: u64, document: u64, options: u64, result: u32) -> u64 {
    reply(
        Format::named(result)
            .and_then(|format| evaluate_on_document(expression, document, options, format)),
    )
}

fn evaluate_on_document(
    expression: u64,
    document: u64,
    options: u64,
    format: Format,
) -> Result<Vec<u8>, Vec<u8>> {
    let compiled = EXPRESSIONS
        .get(expression)
        .ok_or_else(|| unknown("expression", expression))?;
    let options = find_options(options)?;
    let text = DOCUMENTS
        .get(document)
        .ok_or_else(|| unknown("document", document))?;
    let document = documents::converted(&text).map_err(|e| not_json(e.to_string()))?;
    run(
        &compiled,
        Some(&document),
        options.as_deref().unwrap_or(&DEFAULTS),
        format,
    )
}

/// The options of handle `options`, or `None` for 0, the defaults.
fn find_options(options: u64) -> Result<Option<Arc<Options>>, Vec<u8>> {
    match options {
        0 => Ok(None),
        options => OPTIONS
            .get(options)
            .map(Some)
            .ok_or_else(|| unknown("options", options)),
    }
}

/// The default options, shared by the evaluations given none.
static DEFAULTS: LazyLock<Options> = LazyLock::new(Options::default);

fn run(
    compiled: &Compiled,
    document: Option<&Document>,
    options: &Options,
    format: Format,
) -> Result<Vec<u8>, Vec<u8>> {
    let bindings = Bindings::new();
    let expression = &compiled.expression;
    let result = match format {
        Format::Json => match document {
            Some(document) => expression.evaluate_document(document, &bindings, options),
            None => expression.evaluate_with(None, &bindings, options),
        }
        .map(|value| value.map(|value| json_reply(&value))),
        Format::Values => {
            let write = |output: Output<'_, '_>| {
                let mut encoder = Encoder::new(vec![REPLY_VALUES]);
                Ok(output.write(&mut encoder)?.map(|()| encoder.out))
            };
            match document {
                Some(document) => {
                    expression.evaluate_document_to(document, &bindings, options, write)
                }
                None => expression.evaluate_source_to(&NoInput, &bindings, options, write),
            }
        }
    };
    Ok(result
        .map_err(|e| failure(&e, &compiled.source))?
        .unwrap_or_else(|| vec![REPLY_UNDEFINED]))
}

/// No input: an evaluation of [`evaluate`] given none.
struct NoInput;

impl Source<'_> for NoInput {
    fn read(&self, _sink: &mut Sink<'_, '_>) -> Result<(), String> {
        Ok(())
    }
}

/// Parses JSON text for the fields `read` names; text that is not UTF-8 is not JSON either
/// (RFC 8259).
fn parse(bytes: &[u8], read: &Read) -> Result<Document, Vec<u8>> {
    let text = std::str::from_utf8(bytes).map_err(|e| not_json(e.to_string()))?;
    Document::parse_cut(text, read).map_err(|e| not_json(e.to_string()))
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

/// The reply holding `handle`, or the `U1001` error when its table was full.
fn handle_reply(handle: Option<u64>) -> Result<Vec<u8>, Vec<u8>> {
    let handle = handle.ok_or_else(full)?;
    let mut reply = vec![REPLY_HANDLE];
    reply.extend_from_slice(&handle.to_le_bytes());
    Ok(reply)
}

/// The reply holding `value` as compact JSON text.
fn json_reply(value: &Value) -> Vec<u8> {
    let text = jlens_core::format_json(value, false);
    let mut reply = Vec::with_capacity(1 + text.len());
    reply.push(REPLY_JSON);
    reply.extend_from_slice(text.as_bytes());
    reply
}

/// The `U2001` reply for input that is not JSON.
fn not_json(message: String) -> Vec<u8> {
    failure(&Error::new("U2001").with("message", message), "")
}

/// The `U2005` reply for an argument (`options`, `read`, `reads`) that is not in its form.
fn invalid(kind: &str, message: String) -> Vec<u8> {
    failure(
        &Error::new("U2005")
            .with("token", kind)
            .with("message", message),
        "",
    )
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
