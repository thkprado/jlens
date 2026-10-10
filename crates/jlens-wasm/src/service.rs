//! Schema inference and the language service.

use std::collections::BTreeMap;

use jlens_schema::{FunctionDecl, Override, Schema};
use serde_json::Value;
use wasm_bindgen::prelude::*;

use crate::convert::{Offsets, from_js, ser_to_js, to_js, type_error};

/// Infers a JSON Schema (2020-12, with `x-jlens-*` annotations) from sample documents.
/// `overrides` adds descriptions, labels, allowed values and hidden fields by path, such as
/// `"items[].type"`.
/// Paths missing from the samples are declared; a malformed path or override throws a `TypeError`.
#[wasm_bindgen(js_name = inferSchema, skip_typescript)]
pub fn infer_schema(samples: JsValue, overrides: Option<JsValue>) -> Result<JsValue, JsValue> {
    crate::convert::install_hooks();
    let samples = match from_js(&samples, "samples")? {
        Some(Value::Array(samples)) => samples,
        _ => return Err(type_error("samples must be an array of documents")),
    };
    let mut schema = Schema::infer(&samples);
    let overrides = match &overrides {
        Some(overrides) => from_js(overrides, "overrides")?,
        None => None,
    };
    if let Some(overrides) = overrides {
        let overrides: BTreeMap<String, Override> = serde_json::from_value(overrides)
            .map_err(|e| type_error(&format!("invalid overrides: {e}")))?;
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
                        let shape = match doc {
                            Value::Null => None,
                            doc => Some(import_schema(doc, &format!("variable ${name}"))?.root),
                        };
                        inner = inner.with_variable(name, shape);
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
        offsets.convert_spans(&mut out);
        to_js(&out)
    }

    /// Syntax errors, and references to fields the schema does not have.
    #[wasm_bindgen(skip_typescript)]
    pub fn diagnostics(&self, text: &str) -> JsValue {
        let offsets = Offsets::new(text);
        let mut out = serde_json::to_value(self.inner.diagnostics(text)).expect("serializable");
        offsets.convert_spans(&mut out);
        to_js(&out)
    }

    /// The type and description of what is under `offset`, if anything.
    #[wasm_bindgen(skip_typescript)]
    pub fn hover(&self, text: &str, offset: usize) -> JsValue {
        let offsets = Offsets::new(text);
        match self.inner.hover(text, offsets.to_byte(offset)) {
            Some(hover) => {
                let mut out = serde_json::to_value(hover).expect("serializable");
                offsets.convert_spans(&mut out);
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
