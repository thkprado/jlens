//! Schema inference and the language service.

use std::collections::BTreeMap;

use jlens_schema::{FunctionDecl, JsonType, LoadReport, Override, Schema, SchemaLoader};
use serde::Deserialize;
use serde_json::Value;
use wasm_bindgen::prelude::*;

use crate::convert::{convert_bounds, convert_spans, from_js, ser_to_js, to_js, type_error};
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

/// Where the `$ref`s of a schema lead, as the application gives it: nothing is fetched.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sources {
    /// The URI of the schema itself, which its `$ref`s are resolved against where no `$id`
    /// says otherwise.
    #[serde(default)]
    uri: Option<String>,
    /// The other documents, by URI.
    #[serde(default)]
    documents: Option<serde_json::Map<String, Value>>,
}

impl Sources {
    /// The sources in `sources`, as JSON: none in `null`, or in none at all.
    fn read(sources: Option<Value>) -> Result<Sources, JsValue> {
        match sources {
            None | Some(Value::Null) => Ok(Sources::default()),
            // A derived `Deserialize` would take an array for its fields in order.
            Some(sources @ Value::Object(_)) => serde_json::from_value(sources)
                .map_err(|e| type_error(&format!("invalid sources: {e}"))),
            Some(_) => Err(type_error("sources must be an object")),
        }
    }

    /// Reads `doc` with these documents; `located` says whether it is the schema that `uri`
    /// names, which a variable's schema is not.
    fn load(
        &self,
        doc: &Value,
        located: bool,
        what: &str,
    ) -> Result<(Schema, LoadReport), JsValue> {
        let loader = self
            .documents
            .iter()
            .flatten()
            .fold(SchemaLoader::new(), |loader, (uri, document)| {
                loader.document(uri, document)
            });
        let uri = self.uri.as_deref().filter(|_| located).unwrap_or("");
        loader
            .load_at(uri, doc)
            .map_err(|e| type_error(&format!("{what}: {e}")))
    }
}

/// The members of the object a `LanguageService` is made from.
const OPTIONS: [&str; 6] = [
    "schema",
    "variables",
    "functions",
    "sources",
    "resultType",
    "contextPath",
];

/// The form of a `LanguageService`'s argument, for the messages of the calls that do not have it.
const OPTIONS_FORM: &str = "{schema, variables, functions, sources, resultType, contextPath}";

/// The options of a `LanguageService`, as JSON: an object with members of `OPTIONS` only, empty
/// for no options or `null`. A JSON Schema given in their place, as the constructor took it
/// before 0.35, is an object too: its keywords are refused here, as any other name is.
fn read_options(options: &JsValue) -> Result<serde_json::Map<String, Value>, JsValue> {
    let options = match from_js(options, "options")? {
        None | Some(Value::Null) => return Ok(serde_json::Map::new()),
        Some(Value::Object(options)) => options,
        Some(_) => {
            return Err(type_error(&format!(
                "LanguageService takes one object, {OPTIONS_FORM}"
            )));
        }
    };
    let unknown = options
        .keys()
        .find(|name| !OPTIONS.contains(&name.as_str()));
    match unknown {
        Some(name) => Err(type_error(&format!(
            "LanguageService has no option `{name}`: it takes one object, {OPTIONS_FORM}, \
             with the schema of the input as its `schema`"
        ))),
        None => Ok(options),
    }
}

/// What a load of `schema` does not read (the `$ref`s it does not follow), with the documents
/// that `sources` gives: `{}` when it reads everything. A `LanguageService` made of the same
/// schema and sources leaves the places of those `$ref`s unknown. Throws a `TypeError` for a
/// document that is not a schema, or sources not in their form.
#[wasm_bindgen(js_name = schemaReport, skip_typescript)]
pub fn schema_report(schema: JsValue, sources: Option<JsValue>) -> Result<JsValue, JsValue> {
    crate::convert::install_hooks();
    let sources = match &sources {
        Some(sources) => Sources::read(from_js(sources, "sources")?)?,
        None => Sources::default(),
    };
    let doc = from_js(&schema, "schema")?.ok_or_else(|| type_error("schema has no JSON form"))?;
    let (_, report) = sources.load(&doc, true, "schema")?;
    Ok(ser_to_js(&report))
}

/// Completion, diagnostics, hover and signature help for expressions over documents described
/// by a JSON Schema. Offsets are in UTF-16 code units.
#[wasm_bindgen(skip_typescript)]
pub struct LanguageService {
    inner: jlens_schema::LanguageService,
}

#[wasm_bindgen]
impl LanguageService {
    /// Creates a service from one object of options, each optional. `schema` describes the
    /// input documents (without it, functions and variables are still completed); `variables`
    /// declares the bindings the expressions are evaluated with, optionally with a schema for
    /// their values; `functions` declares the host functions they are evaluated with; `sources`
    /// gives the URI of `schema` and the other documents that the `$ref`s of the schemas lead
    /// to; `resultType` gives the types the result of the expressions is expected to have, and
    /// `contextPath` the path whose items they are evaluated against.
    ///
    /// Throws a `TypeError` for a member the options do not have, and for a second argument:
    /// before 0.35 these were positional arguments, the first of them the schema.
    #[wasm_bindgen(constructor, variadic, skip_typescript)]
    pub fn new(options: JsValue, rest: Vec<JsValue>) -> Result<LanguageService, JsValue> {
        crate::convert::install_hooks();
        if !rest.is_empty() {
            return Err(type_error(&format!(
                "LanguageService takes one object, {OPTIONS_FORM}, in place of several arguments"
            )));
        }
        let mut options = read_options(&options)?;
        let sources = Sources::read(options.remove("sources"))?;
        let schema = match options.remove("schema") {
            None | Some(Value::Null) => None,
            Some(doc) => Some(sources.load(&doc, true, "schema")?.0),
        };
        let mut inner = jlens_schema::LanguageService::new(schema);
        match options.remove("variables") {
            None | Some(Value::Null) => {}
            Some(Value::Object(map)) => {
                for (name, doc) in map {
                    let name = name.trim_start_matches('$').to_owned();
                    inner = match doc {
                        Value::Null => inner.with_variable(name, None),
                        doc => {
                            let what = format!("variable ${name}");
                            let schema = sources.load(&doc, false, &what)?.0;
                            inner.with_variable_schema(name, schema)
                        }
                    };
                }
            }
            Some(_) => return Err(type_error("variables must be an object")),
        }
        match options.remove("functions") {
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
        match options.remove("resultType") {
            None | Some(Value::Null) => {}
            Some(types) => {
                let types = JsonType::one_or_more(&types)
                    .map_err(|e| type_error(&format!("resultType {e}")))?;
                inner = inner.with_result_type(types);
            }
        }
        match options.remove("contextPath") {
            None | Some(Value::Null) => {}
            Some(Value::String(path)) => inner = inner.with_context_path(&path),
            Some(_) => return Err(type_error("contextPath must be a string")),
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

    /// What the typing knows of the result of `text`: `{types, undefined, sequence}`, with
    /// `check` when the service expects a result type. `undefined` and `sequence` are the
    /// spans of the operands that make the result so.
    #[wasm_bindgen(js_name = resultType, skip_typescript)]
    pub fn result_type(&self, text: &str) -> JsValue {
        let offsets = Offsets::new(text);
        let result = self.inner.result_type(text);
        let mut out = serde_json::to_value(&result).expect("serializable");
        if let Value::Object(object) = &mut out {
            for key in ["undefined", "sequence"] {
                if let Some(Value::Object(span)) = object.get_mut(key) {
                    convert_bounds(&offsets, span);
                }
            }
            if let Some(expected) = self.inner.expected_result_type() {
                let check = serde_json::to_value(result.check(expected)).expect("serializable");
                object.insert("check".into(), check);
            }
        }
        to_js(&out)
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

    /// The fields of the schema of the service's input, each with what completion and hover
    /// tell of it, in the order of their paths; none without a schema.
    #[wasm_bindgen(skip_typescript)]
    pub fn fields(&self) -> JsValue {
        ser_to_js(&self.inner.fields())
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
