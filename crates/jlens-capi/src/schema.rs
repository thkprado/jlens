//! Schemas and language services (ADR 0023): a schema read from JSON Schema text, a service
//! made of a schema and the declarations of the variables and host functions its expressions
//! are evaluated with, and the diagnostics of an expression's text.
//!
//! Both are known by handle and never change once made. One serves every thread: a call holds
//! its table's lock only to find the object, and types the expression with no lock held. A
//! service copies the schemas it is made from, so freeing one does not affect it; the copy
//! costs about a sixth of the schema's load (roadmap 0.33 #3).

use jlens_core::{Error, Offsets};
use jlens_schema::{FunctionDecl, LanguageService, Schema};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::api::{failure, handle_reply, invalid, json_reply, not_json, reply, unknown};
use crate::buffers;
use crate::handles::Table;

/// The live schemas. `Schema` is `Send` and `Sync`: one serves every thread.
static SCHEMAS: Table<Schema> = Table::new();

/// The live services. `LanguageService` is `Send` and `Sync`: one serves every thread.
static SERVICES: Table<LanguageService> = Table::new();

/// Reads the JSON Schema text in buffer `text`, which stays the caller's, and replies with the
/// schema's handle or the error: `U2001` for text that is not JSON, `U2005` with the token
/// `schema` for a JSON Schema that jlens refuses.
pub fn schema_new(text: u64) -> u64 {
    reply(schema_text(text))
}

fn schema_text(text: u64) -> Result<Vec<u8>, Vec<u8>> {
    let document = buffers::with(text, |bytes| serde_json::from_slice::<Value>(bytes))
        .ok_or_else(|| unknown("buffer", text))?
        .map_err(|e| not_json(e.to_string()))?;
    let schema = Schema::from_json_schema(&document).map_err(|e| invalid("schema", e))?;
    handle_reply(SCHEMAS.insert(schema))
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
    let mut service = LanguageService::new(schema.map(|schema| Schema::clone(&schema)));
    let Some(declarations) = declarations else {
        return handle_reply(SERVICES.insert(service));
    };
    for (name, value) in declarations.variables.unwrap_or_default() {
        let name = name.trim_start_matches('$');
        let shape = match &value {
            Value::Null => None,
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
                Some(schema.root.clone())
            }
        };
        service = service.with_variable(name, shape);
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
