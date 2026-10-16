//! Python bindings for jlens: the native module `jlens._jlens` of the `jlens` package (ADR 0014).
//!
//! Build the package with maturin (`maturin develop` in this directory). The Python API mirrors
//! the WebAssembly package's, in Python's style:
//!
//! * [`Expression`] compiles an expression once; `evaluate`, `evaluate_json` and
//!   `evaluate_document` run it against any number of documents, with bindings and the
//!   evaluation's options as keyword arguments.
//! * [`Document`] converts a document once, for many evaluations, from any thread.
//! * Python callables in the bindings, or registered with `register_function`, are called as
//!   host functions, and the JSONata functions they receive arrive as `jlens.Function`s.
//!   `evaluate_with_audit` returns the audit and the explanation as Python values.
//! * `Expression.reads` tells which fields of the input and the bindings an expression reads,
//!   and [`union_reads`] unites what several expressions read (ADR 0016).
//!
//! Errors raise `jlens.JlensError`, whose offsets count code points, the unit of Python's `str`.
//! The type stubs are in `python/jlens/_jlens.pyi`.

mod convert;
mod host;
mod source;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, RwLock, Weak};

use jlens_core::{Bindings, Options, audit};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::gc::{PyTraverseError, PyVisit};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};
use serde_json::Value as Json;

use convert::{JlensError, Registered, to_py};
use host::{Function, Host};
use source::PySource;

/// A compiled JSONata expression.
///
/// Compile once, then evaluate against any number of documents, from any thread.
#[pyclass(frozen, module = "jlens")]
pub struct Expression {
    inner: jlens_core::Expression,
    /// The source, shared with the [`Host`]s of the evaluations.
    source: Arc<str>,
    registered: RwLock<Vec<Registered>>,
}

/// Why `evaluate_json` and `evaluate_document` failed, carried out of the code that runs
/// detached from the interpreter.
enum Failure {
    Input(String),
    Evaluation(jlens_core::Error),
}

#[pymethods]
impl Expression {
    /// Compiles a JSONata expression. Raises a `JlensError` on syntax errors.
    #[new]
    fn new(py: Python<'_>, source: &str) -> PyResult<Self> {
        let inner =
            jlens_core::Expression::compile(source).map_err(|e| convert::error(py, &e, source))?;
        Ok(Expression {
            inner,
            source: source.into(),
            registered: RwLock::default(),
        })
    }

    /// Makes `implementation` available to the expression as `$name`, in every evaluation. With
    /// a `signature` (such as `<n-n?:n>`), arguments are checked before each call, as for
    /// built-in functions. A function of the same name in the bindings takes precedence.
    /// Raises a `JlensError` when the signature is invalid.
    #[pyo3(signature = (name, implementation, signature=None))]
    fn register_function(
        &self,
        py: Python<'_>,
        name: &str,
        implementation: &Bound<'_, PyAny>,
        signature: Option<String>,
    ) -> PyResult<()> {
        if !implementation.is_callable() {
            return Err(PyTypeError::new_err("implementation must be callable"));
        }
        let arity = match &signature {
            Some(signature) => jlens_core::signature::Signature::parse(signature)
                .map_err(|e| convert::error(py, &e, signature))?
                .params
                .len(),
            None => host::arity(implementation),
        };
        let name = name.trim_start_matches('$').to_owned();
        let mut registered = self.registered.write().unwrap_or_else(|e| e.into_inner());
        registered.retain(|r| r.name != name);
        registered.push(Registered {
            name,
            function: implementation.clone().unbind(),
            signature,
            arity,
        });
        Ok(())
    }

    /// The source text of the expression.
    #[getter]
    fn source(&self) -> &str {
        self.inner.source()
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let source = PyString::new(py, self.inner.source()).repr()?;
        Ok(format!("Expression({source})"))
    }

    /// Evaluates the expression against `input`, read one level at a time as the expression
    /// reaches it. `None` means no input document, and a result of `None` an undefined result
    /// or `null`.
    #[pyo3(signature = (input=None, bindings=None, **options))]
    fn evaluate<'py>(
        &self,
        py: Python<'py>,
        input: Option<&Bound<'py, PyAny>>,
        bindings: Option<&Bound<'py, PyAny>>,
        options: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (bindings, host) = self.bindings(py, bindings)?;
        let options = convert::options(options)?;
        let result = match input.filter(|input| !input.is_none()) {
            Some(input) => {
                let source = PySource::Input(input.clone());
                self.inner.evaluate_source(&source, &bindings, &options)
            }
            None => self.inner.evaluate_with(None, &bindings, &options),
        }
        .map_err(|e| self.error(py, &e, host.as_deref()))?;
        match result {
            Some(value) => to_py(py, &value),
            None => Ok(py.None().into_bound(py)),
        }
    }

    /// Like `evaluate`, with the input document and the result as JSON text: `None` for an
    /// undefined result, `"null"` for `null`. The evaluation lets other Python threads run, and
    /// attaches again to call a host function written in Python.
    #[pyo3(signature = (input=None, bindings=None, **options))]
    fn evaluate_json(
        &self,
        py: Python<'_>,
        input: Option<&str>,
        bindings: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Option<String>> {
        let (bindings, host) = self.bindings(py, bindings)?;
        let options = convert::options(options)?;
        let result = py.detach(|| {
            let document = match input {
                Some(text) => Some(
                    jlens_core::Document::parse(text)
                        .map_err(|e| Failure::Input(format!("input is not valid JSON: {e}")))?,
                ),
                None => None,
            };
            self.run(document.as_ref(), &bindings, &options)
                .map(|result| result.map(|v| jlens_core::format_json(&v, false)))
        });
        result.map_err(|e| self.failure(py, e, host.as_deref()))
    }

    /// Evaluates the expression against a `Document` converted beforehand. The evaluation lets
    /// other Python threads run.
    #[pyo3(signature = (document, bindings=None, **options))]
    fn evaluate_document<'py>(
        &self,
        py: Python<'py>,
        document: &Bound<'py, Document>,
        bindings: Option<&Bound<'py, PyAny>>,
        options: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (bindings, host) = self.bindings(py, bindings)?;
        let options = convert::options(options)?;
        let json = &document.get().json;
        let result = py
            .detach(|| self.run(Some(&converted(json)), &bindings, &options))
            .map_err(|e| self.failure(py, e, host.as_deref()))?;
        match result {
            Some(value) => to_py(py, &value),
            None => Ok(py.None().into_bound(py)),
        }
    }

    /// Evaluates the expression and records how the result was reached, at `level` (`"off"`,
    /// `"decisions"`, the default, or `"full"`) within the capture `limits`. The input is
    /// converted whole first. Evaluation errors are not raised: they are reported in the result,
    /// a `dict` with `result`, `error`, `audit`, `explanation` and `explanation_text`.
    #[pyo3(signature = (input=None, bindings=None, *, level=None, limits=None, **options))]
    fn evaluate_with_audit<'py>(
        &self,
        py: Python<'py>,
        input: Option<&Bound<'py, PyAny>>,
        bindings: Option<&Bound<'py, PyAny>>,
        level: Option<&str>,
        limits: Option<&Bound<'py, PyDict>>,
        options: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let input = match input.filter(|input| !input.is_none()) {
            Some(input) => Some(convert::to_json(input, "the input")?),
            None => None,
        };
        let audit_options = convert::audit_options(level, limits)?;
        let (bindings, _host) = self.bindings(py, bindings)?;
        let options = convert::options(options)?;
        let (result, tree) =
            self.inner
                .evaluate_audited(input.as_ref(), &bindings, &options, &audit_options);
        let explanation = audit::explain(&tree);
        let out = PyDict::new(py);
        match result {
            Ok(value) => {
                let value = match value {
                    Some(value) => to_py(py, &value)?,
                    None => py.None().into_bound(py),
                };
                out.set_item("result", value)?;
                out.set_item("error", py.None())?;
            }
            Err(err) => {
                out.set_item("result", py.None())?;
                out.set_item("error", convert::error_dict(py, &err, &self.source)?)?;
            }
        }
        let mut tree = serde_json::to_value(&tree).map_err(json_error)?;
        convert::audit_spans_to_chars(&self.source, &mut tree);
        out.set_item("audit", to_py(py, &tree)?)?;
        let lines = serde_json::to_value(&explanation).map_err(json_error)?;
        out.set_item("explanation", to_py(py, &lines)?)?;
        out.set_item("explanation_text", explanation.to_string())?;
        Ok(out)
    }

    /// The fields of the input and of the bindings the expression reads (ADR 0016), as a `dict`
    /// in the JSON form. The functions in `bindings`, and those registered on the expression,
    /// read only their arguments, since a Python function does not receive the context of its
    /// call; a call to a function the analysis does not know reads its context whole.
    #[pyo3(signature = (bindings=None))]
    fn reads<'py>(
        &self,
        py: Python<'py>,
        bindings: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (bindings, _host) = self.bindings(py, bindings)?;
        let reads = serde_json::to_value(self.inner.reads_with(&bindings)).map_err(json_error)?;
        to_py(py, &reads)
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        if let Ok(registered) = self.registered.try_read() {
            for r in registered.iter() {
                visit.call(&r.function)?;
            }
        }
        Ok(())
    }

    fn __clear__(&self) {
        let mut registered = self.registered.write().unwrap_or_else(|e| e.into_inner());
        drop(std::mem::take(&mut *registered));
    }
}

fn json_error(e: serde_json::Error) -> PyErr {
    PyValueError::new_err(e.to_string())
}

impl Expression {
    fn run(
        &self,
        document: Option<&jlens_core::Document>,
        bindings: &Bindings,
        options: &Options,
    ) -> Result<Option<Json>, Failure> {
        match document {
            Some(document) => self.inner.evaluate_document(document, bindings, options),
            None => self.inner.evaluate_with(None, bindings, options),
        }
        .map_err(Failure::Evaluation)
    }

    /// The exception for an evaluation error: a `JlensError`, or the exception a Python
    /// function raised.
    fn error(&self, py: Python<'_>, err: &jlens_core::Error, host: Option<&Host>) -> PyErr {
        host.and_then(|host| host.exception(py, err))
            .unwrap_or_else(|| convert::error(py, err, &self.source))
    }

    fn failure(&self, py: Python<'_>, failure: Failure, host: Option<&Host>) -> PyErr {
        match failure {
            Failure::Input(message) => PyValueError::new_err(message),
            Failure::Evaluation(err) => self.error(py, &err, host),
        }
    }

    /// Reads the `bindings` argument, after the functions registered on the expression.
    fn bindings(
        &self,
        py: Python<'_>,
        bindings: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<(Bindings, Option<Arc<Host>>)> {
        // Copied, so that no Python code runs under the lock.
        let registered: Vec<Registered> = self
            .registered
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|r| Registered {
                name: r.name.clone(),
                function: r.function.clone_ref(py),
                signature: r.signature.clone(),
                arity: r.arity,
            })
            .collect();
        convert::bindings(py, bindings, &registered, &self.source)
    }
}

/// A JSON document converted once, for evaluating several expressions (or one several times)
/// against it. It can be shared between threads: each thread converts it into jlens's internal
/// representation the first time it evaluates against it, and keeps that copy while the
/// document lives.
#[pyclass(frozen, module = "jlens")]
pub struct Document {
    json: Arc<Json>,
}

#[pymethods]
impl Document {
    /// Converts a Python value, with the rules of `evaluate`'s input.
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Document {
            json: Arc::new(convert::to_json(value, "the document")?),
        })
    }

    /// Parses JSON text.
    #[staticmethod]
    fn from_json(py: Python<'_>, text: &str) -> PyResult<Self> {
        let json = py
            .detach(|| serde_json::from_str(text))
            .map_err(|e| PyValueError::new_err(format!("the document is not valid JSON: {e}")))?;
        Ok(Document {
            json: Arc::new(json),
        })
    }
}

thread_local! {
    /// The documents this thread has converted, each with a weak reference to its JSON.
    static CONVERTED: RefCell<Vec<(Weak<Json>, Rc<jlens_core::Document>)>> =
        const { RefCell::new(Vec::new()) };
}

/// This thread's conversion of `json`, made on its first use. Conversions whose documents are
/// gone are dropped first.
fn converted(json: &Arc<Json>) -> Rc<jlens_core::Document> {
    CONVERTED.with_borrow_mut(|cache| {
        cache.retain(|(weak, _)| weak.strong_count() > 0);
        if let Some((_, document)) = cache
            .iter()
            .find(|(weak, _)| std::ptr::eq(weak.as_ptr(), Arc::as_ptr(json)))
        {
            return document.clone();
        }
        let document = Rc::new(jlens_core::Document::new(json));
        cache.push((Arc::downgrade(json), document.clone()));
        document
    })
}

/// What any of several expressions reads: the union of their `reads()`. Raises a `TypeError`
/// when `reads` is not a list or a tuple, and a `ValueError` when an item is not in the form
/// `reads()` returns.
#[pyfunction]
fn union_reads<'py>(reads: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let Json::Array(items) = convert::to_json(reads, "reads")? else {
        return Err(PyTypeError::new_err("reads must be a list"));
    };
    let mut union = jlens_core::reads::Reads::default();
    for item in items {
        let item: jlens_core::reads::Reads = serde_json::from_value(item)
            .map_err(|e| PyValueError::new_err(format!("invalid reads: {e}")))?;
        union = union.union(&item);
    }
    let union = serde_json::to_value(&union).map_err(json_error)?;
    to_py(reads.py(), &union)
}

/// The native module of the `jlens` package.
#[pymodule(gil_used = false)]
fn _jlens(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<Expression>()?;
    m.add_class::<Document>()?;
    m.add_class::<Function>()?;
    m.add_function(wrap_pyfunction!(union_reads, m)?)?;
    m.add("JlensError", m.py().get_type::<JlensError>())?;
    Ok(())
}
