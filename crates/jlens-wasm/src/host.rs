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
//!
//! A JSONata function among the arguments reaches the JavaScript function as a JavaScript
//! function that calls it (ADR 0011). It borrows the call, through a closure that wasm-bindgen
//! invalidates when the call returns; after that, it throws `U1005`.

use std::cell::RefCell;
use std::rc::Rc;

use jlens_core::{Callable, Error, HostCall, HostFunction};
use serde_json::Value;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;

use crate::convert::{Offsets, from_js, js_error, to_js};

/// The detail of an error that refers to an exception kept in the frame.
const EXCEPTION: &str = "hostException";

#[wasm_bindgen(inline_js = r#"
// Wraps `inner`, which takes the arguments as an array, in a function with an `arity` property,
// as jsonata-js does. Returns the function, and a function that ends it: from then on, it throws
// the error given.
export function jlensCallable(inner, arity) {
  let expired;
  const callable = (...args) => {
    if (expired) throw expired;
    return inner(args);
  };
  callable.arity = arity;
  return [callable, (error) => { expired = error; }];
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = jlensCallable)]
    fn callable(inner: &JsValue, arity: u32) -> js_sys::Array;
}

/// The closure behind the JavaScript function given for a JSONata function.
type Body<'a> = dyn Fn(js_sys::Array) -> Result<JsValue, JsValue> + 'a;

struct Frame {
    functions: Vec<js_sys::Function>,
    exceptions: Vec<JsValue>,
    /// The exceptions thrown by the functions given to JavaScript functions, and the errors
    /// they stand for, so that an exception let through keeps its error.
    failures: Vec<(JsValue, Error)>,
    /// The offsets of the expression, for the errors those functions throw.
    offsets: Rc<Offsets>,
}

thread_local! {
    static FRAMES: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
}

/// The JavaScript functions of one evaluation. Dropping it releases them.
pub struct Scope {
    frame: usize,
}

impl Scope {
    /// A scope for an evaluation of the expression whose offsets are `offsets`.
    pub fn enter(offsets: &Rc<Offsets>) -> Scope {
        FRAMES.with(|frames| {
            let mut frames = frames.borrow_mut();
            frames.push(Frame {
                functions: Vec::new(),
                exceptions: Vec::new(),
                failures: Vec::new(),
                offsets: offsets.clone(),
            });
            Scope {
                frame: frames.len() - 1,
            }
        })
    }

    /// A host function that calls `function`, bound as `$name`. Without a signature, its arity
    /// is the function's `length`. It is marked as not reading the context of its call, which a
    /// JavaScript function cannot read (`this` is `undefined`), for `Expression.reads`.
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
        let imp = move |args: &[Option<Value>], host: &HostCall<'_>| {
            call(frame, index, &name, args, host)
        };
        let function = match signature {
            Some(signature) => HostFunction::with_signature(signature, imp)?,
            None => HostFunction::new(arity, imp),
        };
        Ok(function.without_input())
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
    host: &HostCall<'_>,
) -> jlens_core::Result<Option<Value>> {
    // The registry is not borrowed during the call: the function may evaluate expressions too.
    let function = FRAMES.with(|frames| frames.borrow()[frame].functions[index].clone());
    let js_args: js_sys::Array = args
        .iter()
        .map(|a| a.as_ref().map_or(JsValue::UNDEFINED, to_js))
        .collect();
    // The JSONata functions among the arguments, as JavaScript functions.
    let callables: Vec<(u32, Callable<'_>)> = (0..args.len())
        .filter_map(|i| Some((i as u32, host.function(i)?)))
        .collect();
    let bodies: Vec<_> = callables
        .iter()
        .map(|(_, callable)| move |args: js_sys::Array| callback(frame, callable, &args))
        .collect();
    let closures: Vec<_> = bodies
        .iter()
        .map(|body| Closure::<Body<'_>>::borrow_aborting(body))
        .collect();
    let mut ends = Vec::with_capacity(closures.len());
    for ((i, callable), closure) in callables.iter().zip(&closures) {
        let pair = self::callable(closure.as_js_value(), callable.arity() as u32);
        js_args.set(*i, pair.get(0));
        ends.push(js_sys::Function::from(pair.get(1)));
    }
    let result = function.apply(&JsValue::UNDEFINED, &js_args);
    if !ends.is_empty() {
        let expired = FRAMES.with(|frames| {
            js_error(
                &Error::new("U1005").with("token", name),
                &frames.borrow()[frame].offsets,
            )
        });
        for end in &ends {
            let _ = end.call1(&JsValue::UNDEFINED, &expired);
        }
    }
    drop(closures);
    let result = result.map_err(|e| exception(frame, e))?;
    if is_thenable(&result) {
        return Err(Error::new("U1002").with("token", name));
    }
    // A result that cannot be converted (a cycle, a BigInt) fails like a thrown exception.
    from_js(&result, &format!("the result of ${name}")).map_err(|e| exception(frame, e))
}

/// Calls a JSONata function given to a JavaScript function. Its arguments are converted to JSON
/// (a function becomes undefined), and its result back. Its errors are thrown as a `JlensError`,
/// or as the exception a JavaScript function threw.
fn callback(
    frame: usize,
    callable: &Callable<'_>,
    args: &js_sys::Array,
) -> Result<JsValue, JsValue> {
    let args = args
        .iter()
        .enumerate()
        .map(|(i, arg)| from_js(&arg, &format!("argument {} of the function", i + 1)))
        .collect::<Result<Vec<_>, _>>()?;
    match callable.call(&args) {
        Ok(result) => Ok(result.map_or(JsValue::UNDEFINED, |v| to_js(&v))),
        Err(err) => Err(FRAMES.with(|frames| {
            let frame = &mut frames.borrow_mut()[frame];
            let thrown = match err.detail(EXCEPTION).and_then(|i| i.as_u64()) {
                Some(i) => frame.exceptions[i as usize].clone(),
                None => js_error(&err, &frame.offsets),
            };
            frame.failures.push((thrown.clone(), err));
            thrown
        })),
    }
}

fn is_thenable(value: &JsValue) -> bool {
    value.is_object()
        && js_sys::Reflect::get(value, &JsValue::from_str("then"))
            .is_ok_and(|then| then.is_function())
}

/// Keeps the exception in the frame and returns an error that refers to it. Its code and message
/// are those of the exception (its `code`, or its `name`, and its `message`), for the audit
/// results that report errors instead of throwing them. An exception that a function given to
/// a JavaScript function threw keeps its error.
fn exception(frame: usize, value: JsValue) -> Error {
    let failure = FRAMES.with(|frames| {
        let failures = &frames.borrow()[frame].failures;
        failures
            .iter()
            .rev()
            .find(|(thrown, _)| *thrown == value)
            .map(|(_, err)| err.clone())
    });
    if let Some(err) = &failure
        && err.detail(EXCEPTION).is_some()
    {
        return err.clone();
    }
    let string = |key: &str| {
        value
            .is_object()
            .then(|| js_sys::Reflect::get(&value, &JsValue::from_str(key)).ok())
            .flatten()
            .and_then(|v| v.as_string())
    };
    let error = failure.unwrap_or_else(|| {
        let code = string("code")
            .or_else(|| string("name"))
            .unwrap_or_else(|| "Error".into());
        let message = string("message")
            .or_else(|| value.as_string())
            .or_else(|| js_sys::JSON::stringify(&value).ok()?.as_string())
            .unwrap_or_default();
        Error::new(code).with("message", message)
    });
    let index = FRAMES.with(|frames| {
        let exceptions = &mut frames.borrow_mut()[frame].exceptions;
        exceptions.push(value);
        exceptions.len() - 1
    });
    error.with(EXCEPTION, index)
}
