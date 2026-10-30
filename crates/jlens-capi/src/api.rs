//! The safe functions behind the exports: expressions, documents, options and evaluations, and
//! the replies.

use std::any::Any;
use std::cell::RefCell;
use std::sync::{Arc, LazyLock};

use jlens_core::input::{Sink, Source};
use jlens_core::output::Output;
use jlens_core::reads::{Read, Reads};
use jlens_core::{Bindings, Document, Error, Expression, Options};
use serde_json::{Map, Value};

use crate::documents::{self, DOCUMENTS};
use crate::handles::{Cache, Found, Table};
use crate::host::{self, BINDINGS, Bound};
use crate::options::{self, OPTIONS};
use crate::source::{self, Host, Reader};
use crate::values::Encoder;
use crate::{REPLY_ERROR, REPLY_HANDLE, REPLY_JSON, REPLY_UNDEFINED, REPLY_VALUES, buffers};

/// A compiled expression, with its source (for the offsets of its errors) and the fields it
/// reads of its input (for the pruned parse of ADR 0016).
pub(crate) struct Compiled {
    pub(crate) expression: Expression,
    pub(crate) source: String,
    reads: Reads,
    read: Read,
}

/// The live expressions. `Expression` is `Send` and `Sync`: one serves every thread.
pub(crate) static EXPRESSIONS: Table<Compiled> = Table::new();

thread_local! {
    /// The objects this thread's evaluations found, found again without the tables' locks.
    static FOUND_EXPRESSIONS: RefCell<Cache<Compiled>> = const { RefCell::new(Cache::new()) };
    static FOUND_BINDINGS: RefCell<Cache<Bound>> = const { RefCell::new(Cache::new()) };
    static FOUND_OPTIONS: RefCell<Cache<Options>> = const { RefCell::new(Cache::new()) };
}

/// The expression of handle `expression`, for an evaluation.
fn find_expression(expression: u64) -> Result<Found<Compiled>, Vec<u8>> {
    EXPRESSIONS
        .cached(&FOUND_EXPRESSIONS, expression)
        .ok_or_else(|| unknown("expression", expression))
}

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
/// input), which stays the caller's, with bindings `bindings` (0 for none) and options
/// `options` (0 for the defaults), and replies with the result as JSON text (`result` 0) or as
/// values (`result` 1), or with the error. The fields of the input that the expression does not
/// read are skipped while parsing.
pub fn evaluate(expression: u64, input: u64, bindings: u64, options: u64, result: u32) -> u64 {
    evaluate_into(expression, input, bindings, options, result, 0)
}

/// Evaluates as [`evaluate`] does, and replies into buffer `scratch` when the reply fits
/// (see [Replies](crate#replies)).
pub fn evaluate_into(
    expression: u64,
    input: u64,
    bindings: u64,
    options: u64,
    result: u32,
    scratch: u64,
) -> u64 {
    reply_into(
        Format::named(result)
            .and_then(|format| evaluate_text(expression, input, bindings, options, format)),
        scratch,
    )
}

fn evaluate_text(
    expression: u64,
    input: u64,
    bindings: u64,
    options: u64,
    format: Format,
) -> Result<Vec<u8>, Vec<u8>> {
    let compiled = find_expression(expression)?;
    let bindings = find_bindings(bindings)?;
    let options = find_options(options)?;
    let document = match input {
        0 => None,
        input => Some(
            buffers::with(input, |bytes| parse(bytes, &compiled.read))
                .ok_or_else(|| unknown("buffer", input))??,
        ),
    };
    let input = match &document {
        Some(document) => Input::Document(document),
        None => Input::None,
    };
    run(
        &compiled,
        input,
        bindings.as_deref(),
        options.as_deref().unwrap_or(&DEFAULTS),
        format,
    )
}

/// Evaluates expression `expression` against document `document` with bindings `bindings` (0
/// for none) and options `options` (0 for the defaults), and replies with the result as JSON
/// text (`result` 0) or as values (`result` 1), or with the error.
pub fn evaluate_document(
    expression: u64,
    document: u64,
    bindings: u64,
    options: u64,
    result: u32,
) -> u64 {
    evaluate_document_into(expression, document, bindings, options, result, 0)
}

/// Evaluates as [`evaluate_document`] does, and replies into buffer `scratch` when the reply
/// fits (see [Replies](crate#replies)).
pub fn evaluate_document_into(
    expression: u64,
    document: u64,
    bindings: u64,
    options: u64,
    result: u32,
    scratch: u64,
) -> u64 {
    reply_into(
        Format::named(result).and_then(|format| {
            evaluate_on_document(expression, document, bindings, options, format)
        }),
        scratch,
    )
}

fn evaluate_on_document(
    expression: u64,
    document: u64,
    bindings: u64,
    options: u64,
    format: Format,
) -> Result<Vec<u8>, Vec<u8>> {
    let compiled = find_expression(expression)?;
    let bindings = find_bindings(bindings)?;
    let options = find_options(options)?;
    let text = documents::find(document).ok_or_else(|| unknown("document", document))?;
    let document = documents::converted(text.arc()).map_err(|e| not_json(e.to_string()))?;
    run(
        &compiled,
        Input::Document(&document),
        bindings.as_deref(),
        options.as_deref().unwrap_or(&DEFAULTS),
        format,
    )
}

/// Evaluates expression `expression` against the host's own values, read through `reader`
/// ([`source`]), with bindings `bindings` (0 for none) and options `options` (0 for the
/// defaults), and replies with the result as JSON text (`result` 0) or as values (`result` 1),
/// or with the error. Buffer `input` holds
/// the input as a level of one value, and stays the caller's; `reader` is given `context` at
/// each call. In values, an array or object of the input that the result holds unchanged is
/// written as its number. A `reader` of 0 fails with `U2005`.
pub fn evaluate_source(
    expression: u64,
    input: u64,
    reader: Option<Reader>,
    context: u64,
    bindings: u64,
    options: u64,
    result: u32,
) -> u64 {
    evaluate_source_into(
        expression, input, reader, context, bindings, options, result, 0,
    )
}

/// Evaluates as [`evaluate_source`] does, and replies into buffer `scratch` when the reply fits
/// (see [Replies](crate#replies)).
#[allow(clippy::too_many_arguments)]
pub fn evaluate_source_into(
    expression: u64,
    input: u64,
    reader: Option<Reader>,
    context: u64,
    bindings: u64,
    options: u64,
    result: u32,
    scratch: u64,
) -> u64 {
    reply_into(
        Format::named(result).and_then(|format| {
            let compiled = find_expression(expression)?;
            let bindings = find_bindings(bindings)?;
            let options = find_options(options)?;
            buffers::with(input, |_| ()).ok_or_else(|| unknown("buffer", input))?;
            let reader = reader.ok_or_else(|| invalid("reader", "no reader".into()))?;
            let host = Host::new(reader, context);
            run(
                &compiled,
                Input::Source(source::Input { host: &host, input }),
                bindings.as_deref(),
                options.as_deref().unwrap_or(&DEFAULTS),
                format,
            )
        }),
        scratch,
    )
}

/// The bindings of handle `bindings`, or `None` for 0, none.
pub(crate) fn find_bindings(bindings: u64) -> Result<Option<Found<Bound>>, Vec<u8>> {
    match bindings {
        0 => Ok(None),
        bindings => BINDINGS
            .cached(&FOUND_BINDINGS, bindings)
            .map(Some)
            .ok_or_else(|| unknown("bindings", bindings)),
    }
}

/// The options of handle `options`, or `None` for 0, the defaults.
pub(crate) fn find_options(options: u64) -> Result<Option<Found<Options>>, Vec<u8>> {
    match options {
        0 => Ok(None),
        options => OPTIONS
            .cached(&FOUND_OPTIONS, options)
            .map(Some)
            .ok_or_else(|| unknown("options", options)),
    }
}

/// The default options, shared by the evaluations given none.
pub(crate) static DEFAULTS: LazyLock<Options> = LazyLock::new(Options::default);

/// The input of an evaluation.
enum Input<'i> {
    None,
    Document(&'i Document),
    /// The host's values, read through its reader.
    Source(source::Input<'i>),
}

fn run(
    compiled: &Compiled,
    input: Input<'_>,
    bindings: Option<&Bound>,
    options: &Options,
    format: Format,
) -> Result<Vec<u8>, Vec<u8>> {
    match bindings {
        // The errors of the functions host functions call need the source for their offsets.
        Some(bound) if bound.functions => host::evaluating(&compiled.source, || {
            evaluate_with(compiled, input, &bound.bindings, options, format)
        }),
        Some(bound) => evaluate_with(compiled, input, &bound.bindings, options, format),
        None => evaluate_with(compiled, input, &NO_BINDINGS, options, format),
    }
}

/// The bindings of the evaluations given none.
pub(crate) static NO_BINDINGS: LazyLock<Bindings> = LazyLock::new(Bindings::new);

fn evaluate_with(
    compiled: &Compiled,
    input: Input<'_>,
    bindings: &Bindings,
    options: &Options,
    format: Format,
) -> Result<Vec<u8>, Vec<u8>> {
    let expression = &compiled.expression;
    let result = match format {
        Format::Json => match &input {
            Input::Document(document) => expression.evaluate_document(document, bindings, options),
            Input::None => expression.evaluate_with(None, bindings, options),
            Input::Source(source) => expression.evaluate_source(source, bindings, options),
        }
        .map(|value| value.map(|value| json_reply(&value))),
        Format::Values => {
            let mut encoder = match &input {
                Input::Source(source) => {
                    Encoder::with_input(vec![REPLY_VALUES], source.host.clone())
                }
                _ => Encoder::new(vec![REPLY_VALUES]),
            };
            let write = |output: Output<'_, '_>| {
                Ok(output
                    .write(&mut encoder)?
                    .map(|()| std::mem::take(&mut encoder.out)))
            };
            match &input {
                Input::Document(document) => {
                    expression.evaluate_document_to(document, bindings, options, write)
                }
                Input::None => expression.evaluate_source_to(&NoInput, bindings, options, write),
                Input::Source(source) => {
                    expression.evaluate_source_to(source, bindings, options, write)
                }
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

pub(crate) fn reply(reply: Result<Vec<u8>, Vec<u8>>) -> u64 {
    buffers::insert(reply.unwrap_or_else(|error| error))
}

/// Writes `reply` at the start of buffer `scratch` when it fits, and returns its length in the
/// high half, the low half 0, which no handle has. Otherwise, or when `scratch` is 0 or unknown,
/// stores it in a buffer of its own and returns that buffer's handle, as [`reply`] does.
pub(crate) fn reply_into(reply: Result<Vec<u8>, Vec<u8>>, scratch: u64) -> u64 {
    let reply = reply.unwrap_or_else(|error| error);
    if scratch != 0
        && let Ok(len) = u32::try_from(reply.len())
        && buffers::with_cached(scratch, |bytes| match bytes.get_mut(..reply.len()) {
            Some(start) => {
                start.copy_from_slice(&reply);
                true
            }
            None => false,
        }) == Some(true)
    {
        return u64::from(len) << 32;
    }
    buffers::insert(reply)
}

/// The reply holding `handle`, or the `U1001` error when its table was full.
pub(crate) fn handle_reply(handle: Option<u64>) -> Result<Vec<u8>, Vec<u8>> {
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

/// The `U2005` reply for an argument (`options`, `read`, `reads`, `result`, `reader`,
/// `functions`, `variables`, `function`) that is not in its form.
pub(crate) fn invalid(kind: &str, message: String) -> Vec<u8> {
    failure(
        &Error::new("U2005")
            .with("token", kind)
            .with("message", message),
        "",
    )
}

/// The `U2002` reply for a handle that names no live object of its `kind`.
pub(crate) fn unknown(kind: &str, handle: u64) -> Vec<u8> {
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
pub(crate) fn failure(error: &Error, source: &str) -> Vec<u8> {
    let mut reply = vec![REPLY_ERROR];
    reply.extend_from_slice(
        jlens_core::format_json(&Value::Object(envelope(error, source)), false).as_bytes(),
    );
    reply
}

/// The envelope of an error, its offsets converted to UTF-16 against `source`. A host
/// function's exception is named by its `hostException` ([`host`]).
pub(crate) fn envelope(error: &Error, source: &str) -> Map<String, Value> {
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
    if let Some(exception) = error.detail("hostException") {
        envelope.insert("hostException".into(), exception.clone());
    }
    envelope
}

/// The offset in UTF-16 code units of byte offset `byte` of `text`.
fn utf16(text: &str, byte: usize) -> usize {
    text.char_indices()
        .take_while(|&(at, _)| at < byte)
        .map(|(_, c)| c.len_utf16())
        .sum()
}
