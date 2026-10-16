//! Results written straight into Python objects (ADR 0017).
//!
//! A result is not built as `serde_json` values first: the core walks it and the builder makes
//! the Python objects, with one `str` per key for the whole result, which Python hashes once.

use std::collections::HashMap;

use foldhash::fast::RandomState;
use jlens_core::input::{Sink, Source};
use jlens_core::output::Builder;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyList, PyString};

/// Builds a result as Python objects: `None`, `bool`, `int` for an integral number below
/// 10^21, `float`, `str`, `list` and `dict`.
pub struct PyBuilder<'py> {
    py: Python<'py>,
    /// The keys made so far.
    keys: HashMap<Box<str>, Bound<'py, PyString>, RandomState>,
    /// The first exception raised while building, raised once the result is written.
    error: Option<PyErr>,
}

impl<'py> PyBuilder<'py> {
    pub fn new(py: Python<'py>) -> Self {
        PyBuilder {
            py,
            keys: HashMap::default(),
            error: None,
        }
    }

    /// The result written: `None` for an undefined one, or the first exception raised.
    pub fn finish(self, written: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
        match self.error {
            Some(error) => Err(error),
            None => Ok(written.unwrap_or_else(|| self.py.None().into_bound(self.py))),
        }
    }

    /// `value`, or `None` when building it failed, keeping the failure.
    fn kept<T>(&mut self, value: PyResult<Bound<'py, T>>) -> Bound<'py, PyAny> {
        match value {
            Ok(value) => value.into_any(),
            Err(error) => {
                self.error.get_or_insert(error);
                self.py.None().into_bound(self.py)
            }
        }
    }

    fn key(&mut self, key: &str) -> &Bound<'py, PyString> {
        let py = self.py;
        if !self.keys.contains_key(key) {
            self.keys.insert(key.into(), PyString::new(py, key));
        }
        &self.keys[key]
    }
}

impl<'py> Builder for PyBuilder<'py> {
    type Value = Bound<'py, PyAny>;
    type Array = Vec<Bound<'py, PyAny>>;
    type Object = Bound<'py, PyDict>;

    fn null(&mut self) -> Self::Value {
        self.py.None().into_bound(self.py)
    }

    fn bool(&mut self, value: bool) -> Self::Value {
        PyBool::new(self.py, value).to_owned().into_any()
    }

    fn number(&mut self, value: f64) -> Self::Value {
        let py = self.py;
        let number = if value.fract() == 0.0 && value.abs() < 1e21 {
            if value.abs() < 9.2e18 {
                (value as i64).into_pyobject(py).map(Bound::into_any)
            } else {
                (value as i128).into_pyobject(py).map(Bound::into_any)
            }
        } else {
            value.into_pyobject(py).map(Bound::into_any)
        };
        self.kept(number.map_err(PyErr::from))
    }

    fn string(&mut self, value: &str) -> Self::Value {
        PyString::new(self.py, value).into_any()
    }

    fn array(&mut self, len: usize) -> Self::Array {
        Vec::with_capacity(len)
    }

    fn push(&mut self, array: &mut Self::Array, item: Self::Value) {
        array.push(item);
    }

    fn end_array(&mut self, array: Self::Array) -> Self::Value {
        let list = PyList::new(self.py, array);
        self.kept(list)
    }

    fn object(&mut self, _len: usize) -> Self::Object {
        PyDict::new(self.py)
    }

    fn insert(&mut self, object: &mut Self::Object, key: &str, value: Self::Value) {
        if let Err(error) = object.set_item(self.key(key), value) {
            self.error.get_or_insert(error);
        }
    }

    fn end_object(&mut self, object: Self::Object) -> Self::Value {
        object.into_any()
    }
}

/// No input document: an evaluation of `evaluate` without one.
pub struct NoInput;

impl Source<'_> for NoInput {
    fn read(&self, _sink: &mut Sink<'_, '_>) -> Result<(), String> {
        Ok(())
    }
}
