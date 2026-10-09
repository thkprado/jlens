//! WebAssembly bindings for jlens, built with `wasm-pack build --target web`.
//!
//! The JavaScript API mirrors the crate: compile once with `new Expression(source)` and call
//! `evaluate(input, bindings)` many times. Errors are thrown as JS errors carrying the JSONata
//! error `code`.

use jlens_core::Bindings;
use serde::Serialize;
use serde_json::Value;
use wasm_bindgen::prelude::*;

/// Returns the jlens version.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// A compiled JSONata expression.
#[wasm_bindgen]
pub struct Expression {
    inner: jlens_core::Expression,
}

#[wasm_bindgen]
impl Expression {
    /// Compiles a JSONata expression. Throws on syntax errors.
    #[wasm_bindgen(constructor)]
    pub fn new(source: &str) -> Result<Expression, JsError> {
        let inner = jlens_core::Expression::compile(source).map_err(to_js_error)?;
        Ok(Expression { inner })
    }

    /// Evaluates the expression. `input` and `bindings` are plain JS values; `undefined` input
    /// means no input document. Returns `undefined` when the result is undefined.
    pub fn evaluate(&self, input: JsValue, bindings: JsValue) -> Result<JsValue, JsError> {
        let input: Option<Value> = if input.is_undefined() {
            None
        } else {
            Some(serde_wasm_bindgen::from_value(input)?)
        };
        let bindings: Bindings = if bindings.is_undefined() || bindings.is_null() {
            Bindings::new()
        } else {
            let map: serde_json::Map<String, Value> = serde_wasm_bindgen::from_value(bindings)?;
            map.into_iter().collect()
        };
        match self
            .inner
            .evaluate(input.as_ref(), &bindings)
            .map_err(to_js_error)?
        {
            Some(value) => Ok(value.serialize(&serde_wasm_bindgen::Serializer::json_compatible())?),
            None => Ok(JsValue::UNDEFINED),
        }
    }
}

fn to_js_error(err: jlens_core::Error) -> JsError {
    JsError::new(&err.to_string())
}
