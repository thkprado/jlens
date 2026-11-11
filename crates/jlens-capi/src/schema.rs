//! Schemas and language services (ADR 0023): a schema read from JSON Schema text, a service
//! made of a schema and the declarations of the variables and host functions its expressions
//! are evaluated with, the diagnostics of an expression's text, what a compiled expression uses
//! of the service's schemas, and what changed between two such answers.
//!
//! Both are known by handle and never change once made. One serves every thread: a call holds
//! its table's lock only to find the object, and types the expression with no lock held. A
//! service copies the schemas it is made from, so freeing one does not affect it; the copy
//! costs about a sixth of the schema's load (roadmap 0.33 #3).

use jlens_core::{Error, Offsets};
use jlens_schema::{FunctionDecl, LanguageService, LoadReport, Schema, SchemaLoader, Uses};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::api::{
    EXPRESSIONS, failure, handle_reply, invalid, json_reply, not_json, reply, unknown,
};
use crate::buffers;
use crate::handles::Table;

/// A schema with what its load did not read.
struct Loaded {
    schema: Schema,
    report: LoadReport,
}

/// The live schemas. `Schema` and its report are `Send` and `Sync`: one serves every thread.
static SCHEMAS: Table<Loaded> = Table::new();

/// The live services. `LanguageService` is `Send` and `Sync`: one serves every thread.
static SERVICES: Table<LanguageService> = Table::new();

/// Where the `$ref`s of a schema lead, as JSON text gives it. A member that is absent or
/// `null` gives nothing; an unknown one is an error.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sources {
    /// The URI of the schema itself, which its `$ref`s are resolved against where no `$id`
    /// says otherwise.
    #[serde(default)]
    uri: Option<String>,
    /// From the URI of each other document to the handle of the buffer holding its JSON text.
    #[serde(default)]
    documents: Option<Map<String, Value>>,
}

/// Reads the JSON Schema text in buffer `text`, with the other documents its `$ref`s lead to
/// as buffer `sources` gives them (JSON text, 0 for none), and replies with the schema's
/// handle or the error. Every buffer stays the caller's.
///
/// The sources are an object with `uri`, the URI of the schema itself, and `documents`, an
/// object from the URI of each other document to the handle of the buffer holding its JSON
/// text, a number. Nothing is fetched: a `$ref` to a document that is not given leaves its
/// place unknown, and `schema_report` names it (ADR 0025).
///
/// Text that is not JSON fails with `U2001`, a document's included; a JSON Schema that jlens
/// refuses with `U2005` and the token `schema`; sources not in this form with `U2005` and the
/// token `sources`; a document's handle that names no live buffer with `U2002`.
pub fn schema_new(text: u64, sources: u64) -> u64 {
    reply(schema_text(text, sources))
}

fn schema_text(text: u64, sources: u64) -> Result<Vec<u8>, Vec<u8>> {
    let json = |buffer: u64| {
        buffers::with(buffer, |bytes| serde_json::from_slice::<Value>(bytes))
            .ok_or_else(|| unknown("buffer", buffer))
    };
    let document = json(text)?.map_err(|e| not_json(e.to_string()))?;
    let sources = match sources {
        0 => None,
        sources => Some(json(sources)?.map_err(|e| not_json(e.to_string()))?),
    };
    let sources = match sources {
        // A derived `Deserialize` would take an array for its fields in order.
        Some(sources @ Value::Object(_)) => {
            serde_json::from_value::<Sources>(sources).map_err(|e| e.to_string())
        }
        Some(_) => Err("the sources must be an object".to_owned()),
        None => Ok(Sources {
            uri: None,
            documents: None,
        }),
    }
    .map_err(|message| invalid("sources", message))?;
    let mut documents = Vec::new();
    for (uri, buffer) in sources.documents.unwrap_or_default() {
        let buffer = buffer.as_u64().ok_or_else(|| {
            invalid(
                "sources",
                format!("document {uri} must be a buffer's handle"),
            )
        })?;
        let document = json(buffer)?.map_err(|e| not_json(format!("document {uri}: {e}")))?;
        documents.push((uri, document));
    }
    let loader = documents
        .iter()
        .fold(SchemaLoader::new(), |loader, (uri, document)| {
            loader.document(uri, document)
        });
    let (schema, report) = loader
        .load_at(sources.uri.as_deref().unwrap_or(""), &document)
        .map_err(|e| invalid("schema", e))?;
    handle_reply(SCHEMAS.insert(Loaded { schema, report }))
}

/// Replies with what the load of schema `schema` did not read (ADR 0025), as JSON text: an
/// object whose `unresolved` lists the `$ref`s that were not followed, in the order the load
/// met them, each with `place` (where it is written: its document's URI and a JSON Pointer),
/// `reference` (its text), `uri` (what it resolves to) and `document`, the URI of the document
/// that was not given, absent when the document is there without that place. `{}` when
/// everything was read.
pub fn schema_report(schema: u64) -> u64 {
    reply(report_text(schema))
}

fn report_text(schema: u64) -> Result<Vec<u8>, Vec<u8>> {
    let loaded = SCHEMAS
        .get(schema)
        .ok_or_else(|| unknown("schema", schema))?;
    Ok(json_reply(
        &serde_json::to_value(&loaded.report).unwrap_or_default(),
    ))
}

/// Frees a schema: 1 when it was live, 0 for an unknown or freed handle. The services made
/// from it keep what they need of it.
pub fn schema_free(schema: u64) -> u32 {
    u32::from(SCHEMAS.remove(schema).is_some())
}

/// The declarations of a service as JSON text gives them. A member that is absent or `null`
/// declares nothing; an unknown one is an error, so that a misspelt one does not pass
/// unnoticed.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Declarations {
    /// From each variable's name to the handle of the schema of its value, or `null`.
    #[serde(default)]
    variables: Option<Map<String, Value>>,
    #[serde(default)]
    functions: Option<Vec<FunctionDecl>>,
}

/// Makes a language service for expressions over the documents that schema `schema` describes
/// (0 for no schema), with the declarations in buffer `declarations` (JSON text, 0 for none),
/// which stays the caller's, and replies with the service's handle or the error.
///
/// The declarations are an object with `variables`, an object from each variable's name to the
/// handle of the schema of its value, a number, or `null` for a variable declared with no
/// schema; and `functions`, an array of the host functions, each an object with `name` and,
/// optionally, `signature`, `params` and `summary`. Names are taken with or without `$`. Text
/// that is not JSON fails with `U2001`, declarations not in this form with `U2005` and the
/// token `service`, and a variable's handle that names no live schema with `U2002`.
pub fn service_new(schema: u64, declarations: u64) -> u64 {
    reply(service_text(schema, declarations))
}

fn service_text(schema: u64, declarations: u64) -> Result<Vec<u8>, Vec<u8>> {
    // The handles are checked before what they hold, so that an unknown one is always `U2002`.
    let schema = match schema {
        0 => None,
        schema => Some(
            SCHEMAS
                .get(schema)
                .ok_or_else(|| unknown("schema", schema))?,
        ),
    };
    let declarations = match declarations {
        0 => None,
        declarations => Some(
            buffers::with(declarations, |bytes| serde_json::from_slice::<Value>(bytes))
                .ok_or_else(|| unknown("buffer", declarations))?
                .map_err(|e| not_json(e.to_string()))?,
        ),
    };
    let declarations = declarations
        .map(|declarations| match declarations {
            // A derived `Deserialize` would take an array for its fields in order.
            Value::Object(_) => {
                serde_json::from_value::<Declarations>(declarations).map_err(|e| e.to_string())
            }
            _ => Err("the declarations must be an object".to_owned()),
        })
        .transpose()
        .map_err(|message| invalid("service", message))?;
    let mut service = LanguageService::new(schema.map(|loaded| loaded.schema.clone()));
    let Some(declarations) = declarations else {
        return handle_reply(SERVICES.insert(service));
    };
    for (name, value) in declarations.variables.unwrap_or_default() {
        let name = name.trim_start_matches('$');
        service = match &value {
            Value::Null => service.with_variable(name, None),
            value => {
                let handle = value.as_u64().ok_or_else(|| {
                    invalid(
                        "service",
                        format!("variable ${name} must be a schema's handle or null"),
                    )
                })?;
                let schema = SCHEMAS
                    .get(handle)
                    .ok_or_else(|| unknown("schema", handle))?;
                service.with_variable_schema(name, schema.schema.clone())
            }
        };
    }
    for mut function in declarations.functions.unwrap_or_default() {
        function.name = function.name.trim_start_matches('$').to_owned();
        service = service.with_function(function);
    }
    handle_reply(SERVICES.insert(service))
}

/// Frees a service: 1 when it was live, 0 for an unknown or freed handle. A call running on
/// another thread keeps it until it returns.
pub fn service_free(service: u64) -> u32 {
    u32::from(SERVICES.remove(service).is_some())
}

/// Replies with the diagnostics of the UTF-8 expression in buffer `text`, which stays the
/// caller's, as service `service` gives them: a JSON array of objects with `severity`
/// (`error` or `warning`), `code`, `message` and `span`, whose `start` and `end` are offsets in
/// UTF-16 code units. A text with a syntax error is not a failure: its errors are diagnostics.
/// Text that is not UTF-8 fails with `U2003`.
pub fn service_diagnostics(service: u64, text: u64) -> u64 {
    reply(diagnostics_text(service, text))
}

fn diagnostics_text(service: u64, text: u64) -> Result<Vec<u8>, Vec<u8>> {
    let service = SERVICES
        .get(service)
        .ok_or_else(|| unknown("service", service))?;
    // The text is copied, so that its buffer is not locked while the expression is typed.
    let text = buffers::with(text, |bytes| std::str::from_utf8(bytes).map(str::to_owned))
        .ok_or_else(|| unknown("buffer", text))?
        .map_err(|e| failure(&Error::new("U2003").with("message", e.to_string()), ""))?;
    let mut diagnostics = serde_json::to_value(service.diagnostics(&text)).unwrap_or_default();
    let offsets = Offsets::new(&text);
    if !offsets.is_ascii()
        && let Value::Array(diagnostics) = &mut diagnostics
    {
        for diagnostic in diagnostics {
            let Some(Value::Object(span)) = diagnostic.get_mut("span") else {
                continue;
            };
            for bound in ["start", "end"] {
                if let Some(byte) = span.get(bound).and_then(Value::as_u64) {
                    span.insert(bound.into(), offsets.to_utf16(byte as usize).into());
                }
            }
        }
    }
    Ok(json_reply(&diagnostics))
}

/// Replies with what expression `expression` uses of the schemas of service `service`
/// (roadmap 0.33 #5), as `LanguageService::uses` gives it for the expression's reads: the JSON
/// object `{"input": {path: Use}, "bindings": {name: {path: Use}}, "everyBinding": true}`, the
/// same text for two equal answers. Paths are not offsets: nothing is converted.
pub fn service_uses(service: u64, expression: u64) -> u64 {
    reply(uses_text(service, expression))
}

fn uses_text(service: u64, expression: u64) -> Result<Vec<u8>, Vec<u8>> {
    let service = SERVICES
        .get(service)
        .ok_or_else(|| unknown("service", service))?;
    let compiled = EXPRESSIONS
        .get(expression)
        .ok_or_else(|| unknown("expression", expression))?;
    let uses = service.uses(&compiled.reads);
    Ok(json_reply(&serde_json::to_value(uses).unwrap_or_default()))
}

/// Replies with what changed between two answers of `service_uses`, the JSON texts in buffers
/// `old` and `new`, which stay the caller's (roadmap 0.33 #6), as `Uses::changes` gives it:
/// `{}` when nothing the expression uses changed. It takes no schema and no service, so that
/// an answer kept beside an expression is compared when its schema is gone. Text that is not
/// JSON fails with `U2001`, and JSON that is not an answer with `U2005` and the token `uses`.
pub fn uses_changes(old: u64, new: u64) -> u64 {
    reply(changes_text(old, new))
}

fn changes_text(old: u64, new: u64) -> Result<Vec<u8>, Vec<u8>> {
    let read = |buffer: u64| {
        let value = buffers::with(buffer, |bytes| serde_json::from_slice::<Value>(bytes))
            .ok_or_else(|| unknown("buffer", buffer))?
            .map_err(|e| not_json(e.to_string()))?;
        Uses::from_json(&value).map_err(|message| invalid("uses", message))
    };
    let changes = read(old)?.changes(&read(new)?);
    Ok(json_reply(
        &serde_json::to_value(changes).unwrap_or_default(),
    ))
}
