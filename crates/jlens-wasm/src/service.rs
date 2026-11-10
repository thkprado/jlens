//! Schema inference and the language service.

use std::collections::BTreeMap;

use jlens_schema::{FunctionDecl, Override, Schema};
use serde_json::Value;
use wasm_bindgen::prelude::*;

use crate::convert::{convert_spans, from_js, ser_to_js, to_js, type_error};
use jlens_core::Offsets;

/// Infers a JSON Schema (2020-12, with `x-jlens-*` annotations) from sample documents.
/// `overrides` adds descriptions, labels, allowed values and hidden fields by path, such as
/// `"items[].type"`.
/// Paths missing from the samples are declared; a malformed path or override throws a `TypeError`.
#[wasm_bindgen(js_name = inferSchema, skip_typescript)]
pub fn infer_schema(samples: JsValue, overrides: Option<JsValue>) -> Result<JsValue, JsValue> {
    crate::convert::install_hooks();
    let (mut schema, overrides) = read_samples(&samples, overrides.as_ref())?;
    if let Some(overrides) = overrides {
        let failed = schema.apply_overrides(&overrides);
        if !failed.is_empty() {
            return Err(type_error(&format!(
                "malformed override paths: {}",
                failed.join(", ")
            )));
        }
    }
    Ok(to_js(&schema.to_json_schema()))
}

/// The paths of `overrides` that the samples lack, which `inferSchema` declares: each as
/// `{path, suggestion?}`, the suggestion being an existing path it most likely meant. Throws as
/// `inferSchema` does for invalid samples or overrides; malformed paths are left out.
#[wasm_bindgen(js_name = missingOverridePaths, skip_typescript)]
pub fn missing_override_paths(samples: JsValue, overrides: JsValue) -> Result<JsValue, JsValue> {
    crate::convert::install_hooks();
    let (schema, overrides) = read_samples(&samples, Some(&overrides))?;
    let missing = overrides.map_or_else(Vec::new, |o| schema.missing_override_paths(&o));
    Ok(ser_to_js(&missing))
}

/// The schema inferred from `samples`, and the overrides, not applied yet.
fn read_samples(
    samples: &JsValue,
    overrides: Option<&JsValue>,
) -> Result<(Schema, Option<BTreeMap<String, Override>>), JsValue> {
    let samples = match from_js(samples, "samples")? {
        Some(Value::Array(samples)) => samples,
        _ => return Err(type_error("samples must be an array of documents")),
    };
    let overrides = match overrides {
        Some(overrides) => from_js(overrides, "overrides")?,
        None => None,
    };
    let overrides = overrides
        .map(|o| {
            serde_json::from_value(o).map_err(|e| type_error(&format!("invalid overrides: {e}")))
        })
        .transpose()?;
    Ok((Schema::infer(&samples), overrides))
}

/// `value` as a JSONata literal, to insert in place of a value request's `range`: a string in
/// `quote` (`"` by default, or `'`) when it can be, other values as JSON. Throws a `TypeError`
/// for another quote, or a value JSON cannot hold.
#[wasm_bindgen(js_name = valueLiteral, skip_typescript)]
pub fn value_literal(value: JsValue, quote: Option<String>) -> Result<String, JsValue> {
    crate::convert::install_hooks();
    let quote = match quote.as_deref() {
        None | Some("\"") => '"',
        Some("'") => '\'',
        Some(_) => return Err(type_error("quote must be \" or '")),
    };
    let value = from_js(&value, "value")?.ok_or_else(|| type_error("value has no JSON form"))?;
    Ok(jlens_schema::value_literal(&value, quote))
}

/// The built-in functions: name, signature, parameters, category and summary.
#[wasm_bindgen(unchecked_return_type = "FunctionInfo[]")]
pub fn functions() -> JsValue {
    ser_to_js(&jlens_core::function_catalog())
}

fn import_schema(doc: Value, what: &str) -> Result<Schema, JsValue> {
    Schema::from_json_schema(&doc).map_err(|e| type_error(&format!("{what}: {e}")))
}

/// Completion, diagnostics, hover and signature help for expressions over documents described
/// by a JSON Schema. Offsets are in UTF-16 code units.
#[wasm_bindgen(skip_typescript)]
pub struct LanguageService {
    inner: jlens_schema::LanguageService,
}

#[wasm_bindgen]
impl LanguageService {
    /// Creates a service. `schema` describes the input documents (without it, functions and
    /// variables are still completed); `variables` declares the bindings the expressions are
    /// evaluated with, optionally with a schema for their values; `functions` declares the host
    /// functions they are evaluated with.
    #[wasm_bindgen(constructor, skip_typescript)]
    pub fn new(
        schema: Option<JsValue>,
        variables: Option<JsValue>,
        functions: Option<JsValue>,
    ) -> Result<LanguageService, JsValue> {
        crate::convert::install_hooks();
        let schema = match &schema {
            Some(schema) if !schema.is_null() => match from_js(schema, "schema")? {
                Some(doc) => Some(import_schema(doc, "schema")?),
                None => None,
            },
            _ => None,
        };
        let mut inner = jlens_schema::LanguageService::new(schema);
        if let Some(variables) = &variables {
            match from_js(variables, "variables")? {
                None | Some(Value::Null) => {}
                Some(Value::Object(map)) => {
                    for (name, doc) in map {
                        let name = name.trim_start_matches('$').to_owned();
                        inner = match doc {
                            Value::Null => inner.with_variable(name, None),
                            doc => {
                                let schema = import_schema(doc, &format!("variable ${name}"))?;
                                inner.with_variable_schema(name, schema)
                            }
                        };
                    }
                }
                Some(_) => return Err(type_error("variables must be an object")),
            }
        }
        if let Some(functions) = &functions {
            match from_js(functions, "functions")? {
                None | Some(Value::Null) => {}
                Some(list @ Value::Array(_)) => {
                    let list: Vec<FunctionDecl> = serde_json::from_value(list)
                        .map_err(|e| type_error(&format!("invalid function declaration: {e}")))?;
                    for mut decl in list {
                        decl.name = decl.name.trim_start_matches('$').to_owned();
                        inner = inner.with_function(decl);
                    }
                }
                Some(_) => return Err(type_error("functions must be an array")),
            }
        }
        Ok(LanguageService { inner })
    }

    /// Suggestions for the word at `offset`, and the range they replace.
    #[wasm_bindgen(skip_typescript)]
    pub fn complete(&self, text: &str, offset: usize) -> JsValue {
        let offsets = Offsets::new(text);
        let mut out = serde_json::to_value(self.inner.complete(text, offsets.to_byte(offset)))
            .expect("serializable");
        convert_spans(&offsets, &mut out);
        to_js(&out)
    }

    /// Syntax errors, and references to fields the schema does not have.
    #[wasm_bindgen(skip_typescript)]
    pub fn diagnostics(&self, text: &str) -> JsValue {
        let offsets = Offsets::new(text);
        let mut out = serde_json::to_value(self.inner.diagnostics(text)).expect("serializable");
        convert_spans(&offsets, &mut out);
        to_js(&out)
    }

    /// The type and description of what is under `offset`, if anything.
    #[wasm_bindgen(skip_typescript)]
    pub fn hover(&self, text: &str, offset: usize) -> JsValue {
        let offsets = Offsets::new(text);
        match self.inner.hover(text, offsets.to_byte(offset)) {
            Some(hover) => {
                let mut out = serde_json::to_value(hover).expect("serializable");
                convert_spans(&offsets, &mut out);
                to_js(&out)
            }
            None => JsValue::UNDEFINED,
        }
    }

    /// The parameters of the function call around `offset`, if any.
    #[wasm_bindgen(js_name = signatureHelp, skip_typescript)]
    pub fn signature_help(&self, text: &str, offset: usize) -> JsValue {
        let offsets = Offsets::new(text);
        match self.inner.signature_help(text, offsets.to_byte(offset)) {
            Some(help) => ser_to_js(&help),
            None => JsValue::UNDEFINED,
        }
    }
}

#[wasm_bindgen]
impl LanguageService {
    /// What an expression uses of the service's schemas, from its `reads()`: for each field
    /// it reads of its input and of the variables declared, what the schema says there.
    /// Throws a `TypeError` when `reads` is not in the form `reads()` returns.
    #[wasm_bindgen(skip_typescript)]
    pub fn uses(&self, reads: JsValue) -> Result<JsValue, JsValue> {
        let reads = match from_js(&reads, "reads")? {
            // A derived `Deserialize` would take an array for its fields in order.
            Some(reads @ Value::Object(_)) => {
                serde_json::from_value::<jlens_core::reads::Reads>(reads)
                    .map_err(|e| type_error(&format!("invalid reads: {e}")))?
            }
            _ => return Err(type_error("reads must be an object")),
        };
        Ok(ser_to_js(&self.inner.uses(&reads)))
    }
}

/// What changed between two answers of `LanguageService.uses`, that of the schema an
/// expression was written against and that of a new version: `{}` when nothing the expression
/// uses changed. Throws a `TypeError` when one is not in the form `uses()` returns.
#[wasm_bindgen(js_name = usesChanges, skip_typescript)]
pub fn uses_changes(old: JsValue, new: JsValue) -> Result<JsValue, JsValue> {
    crate::convert::install_hooks();
    let read = |value: &JsValue, what: &str| {
        let json = from_js(value, what)?.unwrap_or_default();
        jlens_schema::Uses::from_json(&json)
            .map_err(|e| type_error(&format!("invalid {what}: {e}")))
    };
    Ok(ser_to_js(&read(&old, "old")?.changes(&read(&new, "new")?)))
}
