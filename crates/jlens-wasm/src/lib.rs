//! WebAssembly bindings for jlens: an ES module for the browser.
//!
//! Build the package with `cargo xtask wasm` (or `wasm-pack build --target web`). The
//! JavaScript API mirrors the Rust crates:
//!
//! * [`Expression`] compiles an expression once; `evaluate`, `evaluateJson` and
//!   `evaluateWithAudit` run it against any number of documents.
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
mod service;

pub use service::{LanguageService, functions, infer_schema};

use jlens_core::audit::{self, AuditOptions, CaptureLimits};
use serde_json::Value;
use wasm_bindgen::prelude::*;

use convert::{Offsets, error_json, from_js, js_error, parse_json, to_js, type_error};

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
    offsets: Offsets,
}

#[wasm_bindgen]
impl Expression {
    /// Compiles a JSONata expression. Throws a `JlensError` on syntax errors.
    #[wasm_bindgen(constructor, skip_typescript)]
    pub fn new(source: &str) -> Result<Expression, JsValue> {
        convert::install_hooks();
        let offsets = Offsets::new(source);
        let inner = jlens_core::Expression::compile(source).map_err(|e| js_error(&e, &offsets))?;
        Ok(Expression { inner, offsets })
    }

    /// The source text of the expression.
    #[wasm_bindgen(getter, skip_typescript)]
    pub fn source(&self) -> String {
        self.inner.source().to_owned()
    }

    /// Evaluates the expression against `input`, a JSON-compatible value (`undefined` means no
    /// input document). `bindings` provides variables: `{limit: 10}` defines `$limit`.
    /// Returns `undefined` when the result is undefined; throws a `JlensError` when the
    /// evaluation fails.
    #[wasm_bindgen(skip_typescript)]
    pub fn evaluate(
        &self,
        input: Option<JsValue>,
        bindings: Option<JsValue>,
        options: Option<JsValue>,
    ) -> Result<JsValue, JsValue> {
        let input = match &input {
            Some(input) => from_js(input, "input")?,
            None => None,
        };
        Ok(self
            .run(input.as_ref(), bindings, options)?
            .map_or(JsValue::UNDEFINED, |v| to_js(&v)))
    }

    /// Like `evaluate`, with the input document and the result as JSON text. This avoids
    /// converting large documents to and from JavaScript objects.
    #[wasm_bindgen(js_name = evaluateJson, skip_typescript)]
    pub fn evaluate_json(
        &self,
        input: Option<String>,
        bindings: Option<JsValue>,
        options: Option<JsValue>,
    ) -> Result<Option<String>, JsValue> {
        let input = match &input {
            Some(text) => Some(parse_json(text, "input")?),
            None => None,
        };
        Ok(self
            .run(input.as_ref(), bindings, options)?
            .map(|v| jlens_core::format_json(&v, false)))
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
        let bindings = convert::bindings(&bindings.unwrap_or(JsValue::UNDEFINED))?;
        let options = match &options {
            Some(options) => from_js(options, "options")?,
            None => None,
        };
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
        self.offsets.convert_spans(&mut tree);
        out.insert("audit".into(), tree);
        out.insert("explanationText".into(), explanation.to_string().into());
        out.insert(
            "explanation".into(),
            serde_json::to_value(&explanation).expect("serializable"),
        );
        Ok(to_js(&Value::Object(out)))
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
    fn run(
        &self,
        input: Option<&Value>,
        bindings: Option<JsValue>,
        options: Option<JsValue>,
    ) -> Result<Option<Value>, JsValue> {
        let bindings = convert::bindings(&bindings.unwrap_or(JsValue::UNDEFINED))?;
        let options = match &options {
            Some(options) => from_js(options, "options")?,
            None => None,
        };
        let options = convert::options(options.as_ref())?;
        self.inner
            .evaluate_with(input, &bindings, &options)
            .map_err(|e| js_error(&e, &self.offsets))
    }
}

/// Reads `level` and `limits` from the options of `evaluateWithAudit`.
fn audit_options(options: Option<&Value>) -> Result<AuditOptions, JsValue> {
    let mut out = AuditOptions::default();
    let Some(Value::Object(map)) = options else {
        return Ok(out);
    };
    if let Some(level) = map.get("level").filter(|v| !v.is_null()) {
        out.level = serde_json::from_value(level.clone())
            .map_err(|_| type_error("options.level must be \"off\", \"decisions\" or \"full\""))?;
    }
    if let Some(limits) = map.get("limits").filter(|v| !v.is_null()) {
        let Value::Object(given) = limits else {
            return Err(type_error("options.limits must be an object"));
        };
        let mut merged = serde_json::to_value(CaptureLimits::default()).expect("serializable");
        if let Value::Object(base) = &mut merged {
            for (key, value) in given {
                if !base.contains_key(key) {
                    return Err(type_error(&format!("unknown capture limit `{key}`")));
                }
                base.insert(key.clone(), value.clone());
            }
        }
        out.limits = serde_json::from_value(merged)
            .map_err(|e| type_error(&format!("invalid capture limits: {e}")))?;
    }
    Ok(out)
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
