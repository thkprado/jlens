//! Python functions as host functions (ADR 0006).
//!
//! A Python callable in the bindings, or registered with `register_function`, becomes a
//! `HostFunction` that holds it and attaches to the interpreter to call it: `evaluate_json` and
//! `evaluate_document` evaluate detached from it.
//!
//! An exception raised by a Python function is kept in the evaluation's [`Host`], and the error
//! that stops the evaluation refers to it (detail `hostException`), so that the evaluation can
//! raise the original exception again.
//!
//! A JSONata function among the arguments reaches the Python function as a `jlens.Function`
//! that calls it (ADR 0011). The handle jlens-core gives borrows the call, so a Python object
//! cannot hold it: each call publishes it in a frame of a scoped thread-local, and the
//! `Function` refers to its frame by number. Once the call has returned, or from another thread,
//! the frame is gone, and calling the `Function` raises `U1005`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use jlens_core::{Error, HostCall, HostFunction};
use pyo3::prelude::*;
use pyo3::types::{PyString, PyTuple};
use scoped_tls_hkt::scoped_thread_local;
use serde_json::Value as Json;

use crate::convert::{self, to_py, utf16_to_chars};

/// The detail of an error that refers to an exception kept in the [`Host`].
const EXCEPTION: &str = "hostException";

/// The Python functions' side of one evaluation: the exceptions they raised.
pub struct Host {
    /// The expression's source, for the offsets of the errors that `Function`s raise.
    source: Arc<str>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    exceptions: Vec<PyErr>,
    /// The exceptions raised by `Function`s, and the errors they stand for, so that an
    /// exception let through keeps its error.
    failures: Vec<(Py<PyAny>, Error)>,
}

/// A call of a Python function in progress, published for the `Function`s it receives.
#[derive(Clone, Copy)]
struct Frame<'a> {
    id: u64,
    call: &'a HostCall<'a>,
    host: &'a Host,
    /// The call this one runs in, if any: a `Function` of an outer call may still be called.
    parent: Option<&'a Frame<'a>>,
}

scoped_thread_local!(static FRAME: for<'a> &'a Frame<'a>);

/// Numbers the frames, so that a `Function` finds its own.
static FRAMES: AtomicU64 = AtomicU64::new(0);

impl Host {
    /// The host of an evaluation of the expression `source`.
    pub fn new(source: Arc<str>) -> Arc<Host> {
        Arc::new(Host {
            source,
            state: Mutex::default(),
        })
    }

    fn state(&self) -> MutexGuard<'_, State> {
        // No Python code runs under the lock, so a panic cannot leave the state half updated.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The exception that caused `err`, if a Python function of this evaluation raised it, with
    /// the `position` and `token` of the call when it has none.
    pub fn exception(&self, py: Python<'_>, err: &Error) -> Option<PyErr> {
        let index = err.detail(EXCEPTION)?.as_u64()? as usize;
        let exception = self.state().exceptions.get(index)?.clone_ref(py);
        let value = exception.value(py);
        let fields = [
            (
                "position",
                err.position()
                    .map(|p| utf16_to_chars(&self.source, p).into_pyobject(py))
                    .transpose()
                    .ok()
                    .flatten()
                    .map(Bound::into_any),
            ),
            (
                "token",
                err.token().map(|t| PyString::new(py, &t).into_any()),
            ),
        ];
        for (name, field) in fields {
            if let Some(field) = field
                && !value.hasattr(name).unwrap_or(true)
            {
                let _ = value.setattr(name, field);
            }
        }
        Some(exception)
    }

    /// Keeps `err`, raised by a Python function, and returns an error that refers to it. Its code
    /// and message are those of the exception (its `code`, or the name of its type, and its
    /// text), for the audits that report errors instead of raising them. An exception that a
    /// `Function` raised keeps its error.
    fn keep(&self, py: Python<'_>, err: PyErr) -> Error {
        let value = err.value(py);
        let failure = self
            .state()
            .failures
            .iter()
            .rev()
            .find(|(raised, _)| raised.bind(py).is(value))
            .map(|(_, failure)| failure.clone());
        if let Some(failure) = &failure
            && failure.detail(EXCEPTION).is_some()
        {
            return failure.clone();
        }
        // Reading the exception runs Python code, which may call a `Function`: not under the lock.
        let error = failure.unwrap_or_else(|| {
            let code = value
                .getattr("code")
                .ok()
                .and_then(|code| code.extract::<String>().ok())
                .or_else(|| value.get_type().name().ok().map(|name| name.to_string()))
                .unwrap_or_else(|| "Exception".into());
            let message = value.str().map(|s| s.to_string()).unwrap_or_default();
            Error::new(code).with("message", message)
        });
        let mut state = self.state();
        state.exceptions.push(err);
        error.with(EXCEPTION, state.exceptions.len() - 1)
    }

    /// The exception a `Function` raises for `err`: the exception a Python function raised, or a
    /// `JlensError`.
    fn raise(&self, py: Python<'_>, err: Error) -> PyErr {
        let exception = match err.detail(EXCEPTION).and_then(Json::as_u64) {
            Some(index) => self.state().exceptions[index as usize].clone_ref(py),
            None => convert::error(py, &err, &self.source),
        };
        let raised = exception.value(py).clone().into_any().unbind();
        self.state().failures.push((raised, err));
        exception
    }
}

/// A host function that calls `function`, bound as `$name`, with `arity` parameters unless it
/// has a signature. The signature must be valid. It is marked as not reading the context of its
/// call, which a Python function does not receive, for `Expression.reads`.
pub fn function(
    host: &Arc<Host>,
    name: &str,
    function: Py<PyAny>,
    signature: Option<&str>,
    arity: usize,
) -> Result<HostFunction, Error> {
    let (host, name) = (host.clone(), name.to_owned());
    let imp = move |args: &[Option<Json>], call: &HostCall<'_>| {
        Python::attach(|py| self::call(py, &host, &name, function.bind(py), args, call))
    };
    let function = match signature {
        Some(signature) => HostFunction::with_signature(signature, imp)?,
        None => HostFunction::new(arity, imp),
    };
    Ok(function.without_input())
}

/// The number of parameters of `function` that a call must give, as a JavaScript function's
/// `length` counts them: the positional ones before the first with a default value. 0 when
/// Python cannot tell.
pub fn arity(function: &Bound<'_, PyAny>) -> usize {
    let count = || -> PyResult<usize> {
        let inspect = function.py().import("inspect")?;
        let empty = inspect.getattr("Parameter")?.getattr("empty")?;
        let parameters = inspect
            .call_method1("signature", (function,))?
            .getattr("parameters")?
            .call_method0("values")?;
        let mut count = 0;
        for parameter in parameters.try_iter()? {
            let parameter = parameter?;
            // POSITIONAL_ONLY is 0 and POSITIONAL_OR_KEYWORD 1.
            let positional = parameter.getattr("kind")?.extract::<u8>()? <= 1;
            if !positional || !parameter.getattr("default")?.is(&empty) {
                break;
            }
            count += 1;
        }
        Ok(count)
    };
    count().unwrap_or(0)
}

fn call(
    py: Python<'_>,
    host: &Host,
    name: &str,
    function: &Bound<'_, PyAny>,
    args: &[Option<Json>],
    call: &HostCall<'_>,
) -> jlens_core::Result<Option<Json>> {
    let id = FRAMES.fetch_add(1, Ordering::Relaxed);
    let args = args
        .iter()
        .enumerate()
        .map(|(index, arg)| match call.function(index) {
            Some(callable) => Bound::new(
                py,
                Function {
                    frame: id,
                    index,
                    arity: callable.arity(),
                    name: name.to_owned(),
                },
            )
            .map(Bound::into_any),
            None => match arg {
                Some(value) => to_py(py, value),
                None => Ok(py.None().into_bound(py)),
            },
        })
        .collect::<PyResult<Vec<_>>>()
        .map_err(|e| host.keep(py, e))?;
    let args = PyTuple::new(py, args).map_err(|e| host.keep(py, e))?;
    let run = || function.call1(args);
    let result = if FRAME.is_set() {
        FRAME.with(|parent| {
            let frame = Frame {
                id,
                call,
                host,
                parent: Some(parent),
            };
            FRAME.set(&frame, run)
        })
    } else {
        let frame = Frame {
            id,
            call,
            host,
            parent: None,
        };
        FRAME.set(&frame, run)
    };
    let result = result.map_err(|e| host.keep(py, e))?;
    if result.hasattr("__await__").unwrap_or(false) {
        // A coroutine that will never run: closed, so that Python does not warn about it.
        if result.hasattr("close").unwrap_or(false) {
            let _ = result.call_method0("close");
        }
        return Err(Error::new("U1002").with("token", name));
    }
    if result.is_none() {
        return Ok(None);
    }
    // A result that cannot be converted (a cycle, a set) fails like a raised exception.
    convert::to_json(&result, &format!("the result of ${name}"))
        .map(Some)
        .map_err(|e| host.keep(py, e))
}

/// A JSONata function passed to a Python function, such as the lambda in `$retry(function()
/// {...}, 3)`. Calling it applies the function with the context value `null`; it is valid until
/// the Python function returns.
#[pyclass(frozen, module = "jlens")]
pub struct Function {
    frame: u64,
    index: usize,
    arity: usize,
    /// The host function it was passed to.
    name: String,
}

#[pymethods]
impl Function {
    /// The number of parameters the function declares.
    #[getter]
    fn arity(&self) -> usize {
        self.arity
    }

    /// Applies the function. Arguments and the result are converted as for a host function:
    /// `None` stands for undefined, and a callable among the arguments becomes undefined.
    /// Raises a `JlensError`, or the exception a Python function raised, when the function
    /// fails, and `JlensError` `U1005` once the Python function it was passed to has returned.
    #[pyo3(signature = (*args))]
    fn __call__<'py>(
        &self,
        py: Python<'py>,
        args: &Bound<'py, PyTuple>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let args = args
            .iter()
            .enumerate()
            .map(|(i, arg)| {
                if arg.is_none() || arg.is_callable() {
                    Ok(None)
                } else {
                    convert::to_json(&arg, &format!("argument {} of the function", i + 1)).map(Some)
                }
            })
            .collect::<PyResult<Vec<_>>>()?;
        let result = FRAME
            .is_set()
            .then(|| {
                FRAME.with(|mut frame| {
                    while frame.id != self.frame {
                        frame = frame.parent?;
                    }
                    let callable = frame.call.function(self.index)?;
                    Some(callable.call(&args).map_err(|e| frame.host.raise(py, e)))
                })
            })
            .flatten();
        let expired = || {
            let err = Error::new("U1005").with("token", self.name.as_str());
            convert::error(py, &err, "")
        };
        match result.ok_or_else(expired)?? {
            Some(value) => to_py(py, &value),
            None => Ok(py.None().into_bound(py)),
        }
    }

    fn __repr__(&self) -> String {
        format!("<jlens.Function of arity {}>", self.arity)
    }
}
