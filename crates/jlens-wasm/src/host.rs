//! JavaScript functions as host functions (see ADR 0006).
//!
//! A `HostFunction` must be `Send + Sync`, and a JavaScript function is neither. So the functions
//! of an evaluation are kept in a registry local to the thread, in a frame that lives as long as
//! the evaluation's [`Scope`], and the host functions refer to them by index. Frames form a
//! stack, because a JavaScript function can itself evaluate an expression.
//!
//! An exception thrown by a JavaScript function is kept in the frame too, and the error that
//! stops the evaluation refers to it (detail `hostException`), so that the evaluation can
//! rethrow the original exception.

use std::cell::RefCell;

use jlens_core::{Error, HostFunction};
use serde_json::Value;
use wasm_bindgen::prelude::*;

use crate::convert::{from_js, to_js};

/// The detail of an error that refers to an exception kept in the frame.
const EXCEPTION: &str = "hostException";

#[derive(Default)]
struct Frame {
    functions: Vec<js_sys::Function>,
    exceptions: Vec<JsValue>,
}

thread_local! {
    static FRAMES: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
}

/// The JavaScript functions of one evaluation. Dropping it releases them.
pub struct Scope {
    frame: usize,
}

impl Scope {
    pub fn enter() -> Scope {
        FRAMES.with(|frames| {
            let mut frames = frames.borrow_mut();
            frames.push(Frame::default());
            Scope {
                frame: frames.len() - 1,
            }
        })
    }

    /// A host function that calls `function`, bound as `$name`. Without a signature, its arity
    /// is the function's `length`.
    pub fn function(
        &self,
        name: &str,
        function: &js_sys::Function,
        signature: Option<&str>,
    ) -> Result<HostFunction, Error> {
        let arity = function.length() as usize;
        let index = FRAMES.with(|frames| {
            let functions = &mut frames.borrow_mut()[self.frame].functions;
            functions.push(function.clone());
            functions.len() - 1
        });
        let (frame, name) = (self.frame, name.to_owned());
        let imp = move |args: &[Option<Value>], _: &jlens_core::HostCall<'_>| {
            call(frame, index, &name, args)
        };
        match signature {
            Some(signature) => HostFunction::with_signature(signature, imp),
            None => Ok(HostFunction::new(arity, imp)),
        }
    }

    /// The JavaScript exception that caused `err`, if a function of this scope threw it.
    pub fn exception(&self, err: &Error) -> Option<JsValue> {
        let index = err.detail(EXCEPTION)?.as_u64()? as usize;
        FRAMES.with(|frames| frames.borrow()[self.frame].exceptions.get(index).cloned())
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        FRAMES.with(|frames| frames.borrow_mut().truncate(self.frame));
    }
}

fn call(
    frame: usize,
    index: usize,
    name: &str,
    args: &[Option<Value>],
) -> jlens_core::Result<Option<Value>> {
    // The registry is not borrowed during the call: the function may evaluate expressions too.
    let function = FRAMES.with(|frames| frames.borrow()[frame].functions[index].clone());
    let js_args: js_sys::Array = args
        .iter()
        .map(|a| a.as_ref().map_or(JsValue::UNDEFINED, to_js))
        .collect();
    let result = function
        .apply(&JsValue::UNDEFINED, &js_args)
        .map_err(|e| exception(frame, e))?;
    if is_thenable(&result) {
        return Err(Error::new("U1002").with("token", name));
    }
    // A result that cannot be converted (a cycle, a BigInt) fails like a thrown exception.
    from_js(&result, &format!("the result of ${name}")).map_err(|e| exception(frame, e))
}

fn is_thenable(value: &JsValue) -> bool {
    value.is_object()
        && js_sys::Reflect::get(value, &JsValue::from_str("then"))
            .is_ok_and(|then| then.is_function())
}

/// Keeps the exception in the frame and returns an error that refers to it. Its code and message
/// are those of the exception (its `code`, or its `name`, and its `message`), for the audit
/// results that report errors instead of throwing them.
fn exception(frame: usize, value: JsValue) -> Error {
    let string = |key: &str| {
        value
            .is_object()
            .then(|| js_sys::Reflect::get(&value, &JsValue::from_str(key)).ok())
            .flatten()
            .and_then(|v| v.as_string())
    };
    let code = string("code")
        .or_else(|| string("name"))
        .unwrap_or_else(|| "Error".into());
    let message = string("message")
        .or_else(|| value.as_string())
        .or_else(|| js_sys::JSON::stringify(&value).ok()?.as_string())
        .unwrap_or_default();
    let index = FRAMES.with(|frames| {
        let exceptions = &mut frames.borrow_mut()[frame].exceptions;
        exceptions.push(value);
        exceptions.len() - 1
    });
    Error::new(code)
        .with("message", message)
        .with(EXCEPTION, index)
}
