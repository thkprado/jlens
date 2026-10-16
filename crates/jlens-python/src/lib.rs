//! Python bindings for jlens: the native module `jlens._jlens` of the `jlens` package (ADR 0014).
//!
//! Build the package with maturin (`maturin develop` in this directory). The Python API mirrors
//! the WebAssembly package's, in Python's style:
//!
//! * [`Expression`] compiles an expression once; `evaluate`, `evaluate_json` and
//!   `evaluate_document` run it against any number of documents, with bindings and the
//!   evaluation's options as keyword arguments.
//! * [`Document`] converts a document once, for many evaluations, from any thread.
//!
//! Errors raise `jlens.JlensError`, whose offsets count code points, the unit of Python's `str`.
//! The type stubs are in `python/jlens/_jlens.pyi`.

mod convert;
mod source;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Weak};

use jlens_core::{Bindings, Options};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};
use serde_json::Value as Json;

use convert::{JlensError, to_py};
use source::PySource;

/// A compiled JSONata expression.
///
/// Compile once, then evaluate against any number of documents, from any thread.
#[pyclass(frozen, module = "jlens")]
pub struct Expression {
    inner: jlens_core::Expression,
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
        Ok(Expression { inner })
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
        let bindings = convert::bindings(bindings)?;
        let options = convert::options(options)?;
        let result = match input.filter(|input| !input.is_none()) {
            Some(input) => {
                let source = PySource::Input(input.clone());
                self.inner.evaluate_source(&source, &bindings, &options)
            }
            None => self.inner.evaluate_with(None, &bindings, &options),
        }
        .map_err(|e| self.error(py, &e))?;
        match result {
            Some(value) => to_py(py, &value),
            None => Ok(py.None().into_bound(py)),
        }
    }

    /// Like `evaluate`, with the input document and the result as JSON text: `None` for an
    /// undefined result, `"null"` for `null`. The evaluation lets other Python threads run.
    #[pyo3(signature = (input=None, bindings=None, **options))]
    fn evaluate_json(
        &self,
        py: Python<'_>,
        input: Option<&str>,
        bindings: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Option<String>> {
        let bindings = convert::bindings(bindings)?;
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
        result.map_err(|e| self.failure(py, e))
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
        let bindings = convert::bindings(bindings)?;
        let options = convert::options(options)?;
        let json = &document.get().json;
        let result = py
            .detach(|| self.run(Some(&converted(json)), &bindings, &options))
            .map_err(|e| self.failure(py, e))?;
        match result {
            Some(value) => to_py(py, &value),
            None => Ok(py.None().into_bound(py)),
        }
    }
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

    fn error(&self, py: Python<'_>, err: &jlens_core::Error) -> PyErr {
        convert::error(py, err, self.inner.source())
    }

    fn failure(&self, py: Python<'_>, failure: Failure) -> PyErr {
        match failure {
            Failure::Input(message) => PyValueError::new_err(message),
            Failure::Evaluation(err) => self.error(py, &err),
        }
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

/// The native module of the `jlens` package.
#[pymodule(gil_used = false)]
fn _jlens(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<Expression>()?;
    m.add_class::<Document>()?;
    m.add("JlensError", m.py().get_type::<JlensError>())?;
    Ok(())
}
