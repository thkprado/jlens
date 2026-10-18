//! WebAssembly bindings for jlens: an ES module for the browser.
//!
//! Build the package with `cargo xtask wasm` (or `wasm-pack build --target web`). The
//! JavaScript API mirrors the Rust crates:
//!
//! * [`Expression`] compiles an expression once; `evaluate`, `evaluateJson` and
//!   `evaluateWithAudit` run it against any number of documents. JavaScript functions in the
//!   bindings, or registered with `registerFunction`, are called as host functions.
//! * `Expression.reads` tells which fields of the input and the bindings an expression reads,
//!   and [`union_reads`] unites what several expressions read (ADR 0016).
//! * [`parse_recover`] parses with error recovery, for tooling.
//! * [`LanguageService`] offers completion, diagnostics, hover and signature help over a
//!   JSON Schema, and [`infer_schema`] infers one from sample documents.
//! * [`functions`] lists the built-in functions.
//!
//! Every offset that crosses the boundary (error positions, spans, completion ranges, cursor
//! offsets) is in UTF-16 code units, the unit of JavaScript strings. Errors are thrown as
//! `Error` objects named `JlensError` that carry the JSONata error `code`.
//!
//! The TypeScript declarations of the values exchanged are in `src/types.d.ts`.

mod convert;
mod host;
mod output;
mod service;
mod source;

use std::cell::OnceCell;
use std::rc::Rc;

pub use service::{LanguageService, functions, infer_schema};

use jlens_core::audit::{self, AuditOptions, CaptureLimits};
use jlens_core::reads::Read;
use jlens_core::{Bindings, Options};
use serde_json::Value;
use wasm_bindgen::prelude::*;

use convert::{Offsets, Registered, error_json, from_js, js_error, ser_to_js, to_js, type_error};
use host::Scope;
use output::{JsBuilder, NoInput};

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = include_str!("types.d.ts");

/// The jlens version.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// A compiled JSONata expression.
///
/// Compile once, then evaluate against any number of documents. Call `free()` when the
/// expression is no longer needed (or rely on the garbage collector, where `FinalizationRegistry`
/// is available).
#[wasm_bindgen(skip_typescript)]
pub struct Expression {
    inner: jlens_core::Expression,
    offsets: Rc<Offsets>,
    registered: Vec<Registered>,
    /// What the expression reads of its input, which cuts the input of `evaluateJson`; worked
    /// out on the first call.
    input_read: OnceCell<Read>,
}

#[wasm_bindgen]
impl Expression {
    /// Compiles a JSONata expression. Throws a `JlensError` on syntax errors.
    #[wasm_bindgen(constructor, skip_typescript)]
    pub fn new(source: &str) -> Result<Expression, JsValue> {
        convert::install_hooks();
        let offsets = Offsets::new(source);
        let inner = jlens_core::Expression::compile(source).map_err(|e| js_error(&e, &offsets))?;
        Ok(Expression {
            inner,
            offsets: Rc::new(offsets),
            registered: Vec::new(),
            input_read: OnceCell::new(),
        })
    }

    /// Makes `implementation` available to the expression as `$name`, in every evaluation. With
    /// a `signature` (such as `<n-n?:n>`), arguments are checked before each call, as for
    /// built-in functions. Throws a `JlensError` when the signature is invalid.
    #[wasm_bindgen(js_name = registerFunction, skip_typescript)]
    pub fn register_function(
        &mut self,
        name: &str,
        implementation: js_sys::Function,
        signature: Option<String>,
    ) -> Result<(), JsValue> {
        if let Some(signature) = &signature {
            jlens_core::signature::Signature::parse(signature)
                .map_err(|e| js_error(&e, &Offsets::new(signature)))?;
        }
        let name = name.trim_start_matches('$').to_owned();
        self.registered.retain(|r| r.name != name);
        self.registered.push(Registered {
            name,
            function: implementation,
            signature,
        });
        Ok(())
    }

    /// The source text of the expression.
    #[wasm_bindgen(getter, skip_typescript)]
    pub fn source(&self) -> String {
        self.inner.source().to_owned()
    }

    /// Evaluates the expression against `input`, a JSON-compatible value (`undefined` means no
    /// input document). `bindings` provides variables and functions: `{limit: 10}` defines
    /// `$limit`, and `{rate: (region) => ...}` defines `$rate(region)`.
    /// Returns `undefined` when the result is undefined; throws a `JlensError` when the
    /// evaluation fails.
    ///
    /// The input is read during the evaluation, one level at a time, as the expression reaches
    /// it (ADR 0010).
    #[wasm_bindgen(skip_typescript)]
    pub fn evaluate(
        &self,
        input: Option<JsValue>,
        bindings: Option<JsValue>,
        options: Option<JsValue>,
    ) -> Result<JsValue, JsValue> {
        let scope = Scope::enter(&self.offsets);
        let bindings = self.bindings(&scope, bindings)?;
        let options = convert::options(options.as_ref())?;
        let mut builder = JsBuilder::default();
        let write = |out: jlens_core::output::Output<'_, '_>| out.write(&mut builder);
        let written = match input.filter(|input| !input.is_undefined()) {
            Some(input) => {
                let source = source::JsSource::input(input);
                self.inner
                    .evaluate_source_to(&source, &bindings, &options, write)
                    .map_err(|e| {
                        source
                            .exception(&e)
                            .unwrap_or_else(|| self.error(&scope, &e))
                    })?
            }
            None => self
                .inner
                .evaluate_source_to(&NoInput, &bindings, &options, write)
                .map_err(|e| self.error(&scope, &e))?,
        };
        Ok(builder.finish(written))
    }

    /// Like `evaluate`, with the input document and the result as JSON text. This avoids
    /// converting large documents to and from JavaScript objects. The fields of the input that
    /// the expression does not read (`reads()`, without bindings) are skipped while parsing.
    #[wasm_bindgen(js_name = evaluateJson, skip_typescript)]
    pub fn evaluate_json(
        &self,
        input: Option<String>,
        bindings: Option<JsValue>,
        options: Option<JsValue>,
    ) -> Result<Option<String>, JsValue> {
        let document = match &input {
            Some(text) => Some(
                jlens_core::Document::parse_cut(text, self.input_read())
                    .map_err(|e| type_error(&format!("input is not valid JSON: {e}")))?,
            ),
            None => None,
        };
        let scope = Scope::enter(&self.offsets);
        let bindings = self.bindings(&scope, bindings)?;
        let options = convert::options(options.as_ref())?;
        let result = self.run(&scope, document.as_ref(), &bindings, &options)?;
        Ok(result.map(|v| jlens_core::format_json(&v, false)))
    }

    /// Evaluates the expression against a `Document` converted beforehand, which avoids
    /// converting the input on every call.
    #[wasm_bindgen(js_name = evaluateDocument, skip_typescript)]
    pub fn evaluate_document(
        &self,
        document: &Document,
        bindings: Option<JsValue>,
        options: Option<JsValue>,
    ) -> Result<JsValue, JsValue> {
        let scope = Scope::enter(&self.offsets);
        let bindings = self.bindings(&scope, bindings)?;
        let options = convert::options(options.as_ref())?;
        let mut builder = JsBuilder::default();
        let written = self
            .inner
            .evaluate_document_to(&document.inner, &bindings, &options, |out| {
                out.write(&mut builder)
            })
            .map_err(|e| self.error(&scope, &e))?;
        Ok(builder.finish(written))
    }

    /// Evaluates the expression and records how the result was reached. Never throws for
    /// evaluation errors: they are reported in the returned `error`.
    #[wasm_bindgen(js_name = evaluateWithAudit, skip_typescript)]
    pub fn evaluate_with_audit(
        &self,
        input: Option<JsValue>,
        bindings: Option<JsValue>,
        options: Option<JsValue>,
    ) -> Result<JsValue, JsValue> {
        let input = match &input {
            Some(input) => from_js(input, "input")?,
            None => None,
        };
        let scope = Scope::enter(&self.offsets);
        let bindings = self.bindings(&scope, bindings)?;
        let eval_options = convert::options(options.as_ref())?;
        let audit_options = audit_options(options.as_ref())?;
        let (result, tree) =
            self.inner
                .evaluate_audited(input.as_ref(), &bindings, &eval_options, &audit_options);
        let explanation = audit::explain(&tree);
        let mut out = serde_json::Map::new();
        match result {
            Ok(Some(value)) => {
                out.insert("result".into(), value);
            }
            Ok(None) => {}
            Err(err) => {
                out.insert("error".into(), error_json(&err, &self.offsets));
            }
        }
        let mut tree = serde_json::to_value(&tree).expect("serializable");
        self.offsets.convert_audit_spans(&mut tree);
        out.insert("audit".into(), tree);
        out.insert("explanationText".into(), explanation.to_string().into());
        out.insert(
            "explanation".into(),
            serde_json::to_value(&explanation).expect("serializable"),
        );
        Ok(to_js(&Value::Object(out)))
    }

    /// The fields of the input and of the bindings the expression reads (ADR 0016). The
    /// functions in `bindings`, and those registered on the expression, read only their
    /// arguments, since a JavaScript function cannot read the context of its call; a call to a
    /// function the analysis does not know reads its context whole.
    #[wasm_bindgen(skip_typescript)]
    pub fn reads(&self, bindings: Option<JsValue>) -> Result<JsValue, JsValue> {
        let scope = Scope::enter(&self.offsets);
        let bindings = self.bindings(&scope, bindings)?;
        Ok(ser_to_js(&self.inner.reads_with(&bindings)))
    }

    /// What the expression reads of its input, without bindings: a superset of what it reads
    /// with any bindings. An expression that does not read its input needs none of its fields.
    fn input_read(&self) -> &Read {
        self.input_read.get_or_init(|| {
            self.inner
                .reads()
                .input()
                .cloned()
                .unwrap_or_else(|| Read::Fields(Default::default()))
        })
    }

    /// The processed syntax tree, as plain objects (spans in UTF-16 code units).
    #[wasm_bindgen(skip_typescript)]
    pub fn ast(&self) -> JsValue {
        let mut tree = serde_json::to_value(&self.inner.ast().root).expect("serializable");
        self.offsets.convert_spans(&mut tree);
        to_js(&tree)
    }
}

impl Expression {
    /// Evaluates, throwing a `JlensError`, or the exception a JavaScript function threw.
    fn run(
        &self,
        scope: &Scope,
        document: Option<&jlens_core::Document>,
        bindings: &Bindings,
        options: &Options,
    ) -> Result<Option<Value>, JsValue> {
        match document {
            Some(document) => self.inner.evaluate_document(document, bindings, options),
            None => self.inner.evaluate_with(None, bindings, options),
        }
        .map_err(|e| self.error(scope, &e))
    }

    /// The exception for an evaluation error: a `JlensError`, or the exception a JavaScript
    /// function threw.
    fn error(&self, scope: &Scope, err: &jlens_core::Error) -> JsValue {
        match scope.exception(err) {
            Some(exception) => locate(exception, err),
            None => js_error(err, &self.offsets),
        }
    }

    /// Reads the `bindings` argument of the evaluation methods. The functions of the bindings,
    /// and those registered on the expression, belong to `scope`.
    fn bindings(&self, scope: &Scope, bindings: Option<JsValue>) -> Result<Bindings, JsValue> {
        convert::bindings(
            &bindings.unwrap_or(JsValue::UNDEFINED),
            scope,
            &self.registered,
        )
    }
}

/// Adds the call's `position` and `token` to an exception thrown by a JavaScript function, when
/// it is an object that has none, as for other errors.
fn locate(exception: JsValue, err: &jlens_core::Error) -> JsValue {
    if exception.is_object() {
        let fields = [
            ("position", err.position().map(|p| JsValue::from(p as u32))),
            ("token", err.token().map(|t| JsValue::from_str(&t))),
        ];
        for (key, value) in fields {
            let key = JsValue::from_str(key);
            if let Some(value) = value
                && !js_sys::Reflect::has(&exception, &key).unwrap_or(true)
            {
                let _ = js_sys::Reflect::set(&exception, &key, &value);
            }
        }
    }
    exception
}

/// A JSON document converted once into jlens's internal representation, for evaluating
/// several expressions (or one several times) against it.
#[wasm_bindgen(skip_typescript)]
pub struct Document {
    inner: jlens_core::Document,
}

#[wasm_bindgen]
impl Document {
    /// Converts a JSON-compatible value.
    #[wasm_bindgen(constructor, skip_typescript)]
    pub fn new(input: JsValue) -> Result<Document, JsValue> {
        convert::install_hooks();
        match convert::document(&input, "the document")? {
            Some(inner) => Ok(Document { inner }),
            None => Err(type_error("the document cannot be converted to JSON")),
        }
    }

    /// Parses JSON text. With `read` (the `input` of a `reads()`, or of a `unionReads()`), the
    /// fields it leaves out are skipped: they are checked as JSON syntax only.
    #[wasm_bindgen(js_name = fromJson, skip_typescript)]
    pub fn from_json(text: &str, read: Option<JsValue>) -> Result<Document, JsValue> {
        convert::install_hooks();
        let read = match read
            .map(|read| from_js(&read, "read"))
            .transpose()?
            .flatten()
        {
            None | Some(Value::Null) => None,
            Some(read) => Some(
                serde_json::from_value::<Read>(read)
                    .map_err(|e| type_error(&format!("invalid read: {e}")))?,
            ),
        };
        let inner = match &read {
            Some(read) => jlens_core::Document::parse_cut(text, read),
            None => jlens_core::Document::parse(text),
        }
        .map_err(|e| type_error(&format!("the document is not valid JSON: {e}")))?;
        Ok(Document { inner })
    }
}

/// Reads `level` and `limits` from the options of `evaluateWithAudit` (an object, checked by
/// `convert::options`), each through JSON.
fn audit_options(options: Option<&JsValue>) -> Result<AuditOptions, JsValue> {
    let mut out = AuditOptions::default();
    let Some(options) = options else {
        return Ok(out);
    };
    let get = |key: &str| -> Result<Option<Value>, JsValue> {
        let value = js_sys::Reflect::get(options, &JsValue::from_str(key))?;
        Ok(from_js(&value, &format!("options.{key}"))?.filter(|v| !v.is_null()))
    };
    if let Some(level) = get("level")? {
        out.level = serde_json::from_value(level)
            .map_err(|_| type_error("options.level must be \"off\", \"decisions\" or \"full\""))?;
    }
    if let Some(limits) = get("limits")? {
        let Value::Object(given) = limits else {
            return Err(type_error("options.limits must be an object"));
        };
        let mut merged = serde_json::to_value(CaptureLimits::default()).expect("serializable");
        if let Value::Object(base) = &mut merged {
            for (key, value) in given {
                if !base.contains_key(&key) {
                    return Err(type_error(&format!("unknown capture limit `{key}`")));
                }
                base.insert(key, value);
            }
        }
        out.limits = serde_json::from_value(merged)
            .map_err(|e| type_error(&format!("invalid capture limits: {e}")))?;
    }
    Ok(out)
}

/// What any of several expressions reads: the union of their `reads()`. Throws a `TypeError`
/// when an item is not in the form `reads()` returns.
#[wasm_bindgen(js_name = unionReads, skip_typescript)]
pub fn union_reads(reads: JsValue) -> Result<JsValue, JsValue> {
    convert::install_hooks();
    let Some(Value::Array(items)) = from_js(&reads, "reads")? else {
        return Err(type_error("reads must be an array"));
    };
    let mut union = jlens_core::reads::Reads::default();
    for item in items {
        let item: jlens_core::reads::Reads =
            serde_json::from_value(item).map_err(|e| type_error(&format!("invalid reads: {e}")))?;
        union = union.union(&item);
    }
    Ok(ser_to_js(&union))
}

/// Parses an expression, recovering from syntax errors: returns the (partial) syntax tree and
/// every problem found. This never throws.
#[wasm_bindgen(js_name = parseRecover, unchecked_return_type = "ParseResult")]
pub fn parse_recover(source: &str) -> JsValue {
    convert::install_hooks();
    let offsets = Offsets::new(source);
    let ast = jlens_core::parse_recover(source);
    let mut out = serde_json::json!({
        "ast": ast.root,
        "diagnostics": ast.diagnostics,
    });
    offsets.convert_spans(&mut out);
    to_js(&out)
}
