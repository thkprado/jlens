//! The input of `evaluate`, read from the Python value one level at a time (ADR 0010, ADR 0014).
//!
//! A `dict` (or a subclass) is read as an object, a `list` or a `tuple` as an array; `str`,
//! `int`, `float`, `bool` and `None` as JSON's values. Anything else, a key that is not a `str`
//! or a cycle stops the evaluation with `U1004`, with the message `json.dumps` would give. No
//! Python code runs while a level is read, and a `dict` or `list` is locked while it is read,
//! so that another thread of free-threaded Python cannot change it halfway.

use std::rc::Rc;

use jlens_core::input::{Sink, Source};
use pyo3::prelude::*;
use pyo3::sync::critical_section::with_critical_section;
use pyo3::types::{PyDict, PyList, PyTuple};

use crate::convert::{self, Scalar};

/// A level of a Python value.
pub enum PySource<'py> {
    /// The input itself, the only value of its level.
    Input(Bound<'py, PyAny>),
    /// An array or an object, with its ancestors.
    Level(Rc<Node<'py>>),
}

/// An array or an object, and the chain of its ancestors, to detect cycles as `json.dumps` does.
pub struct Node<'py> {
    value: Bound<'py, PyAny>,
    up: Option<Rc<Node<'py>>>,
}

impl<'py> PySource<'py> {
    /// Writes `value`, whose parent is `up`.
    fn put<'a>(
        sink: &mut Sink<'_, 'a>,
        value: &Bound<'py, PyAny>,
        up: Option<&Rc<Node<'py>>>,
    ) -> Result<(), String>
    where
        'py: 'a,
    {
        match convert::scalar(value)? {
            Some(Scalar::Null) => sink.null(),
            Some(Scalar::Bool(b)) => sink.bool(b),
            Some(Scalar::Int(n)) => sink.number(n as f64),
            Some(Scalar::Float(n)) => sink.number(n),
            Some(Scalar::String(s)) => sink.string(&s),
            None => {
                let mut ancestor = up;
                while let Some(node) = ancestor {
                    if node.value.is(value) {
                        return Err(convert::CIRCULAR.into());
                    }
                    ancestor = node.up.as_ref();
                }
                let node = PySource::Level(Rc::new(Node {
                    value: value.clone(),
                    up: up.cloned(),
                }));
                if value.is_instance_of::<PyDict>() {
                    sink.object(node);
                } else {
                    sink.array(node);
                }
            }
        }
        Ok(())
    }
}

impl<'py, 'a> Source<'a> for PySource<'py>
where
    'py: 'a,
{
    fn read(&self, sink: &mut Sink<'_, 'a>) -> Result<(), String> {
        let node = match self {
            PySource::Input(value) => return Self::put(sink, value, None),
            PySource::Level(node) => node,
        };
        let value = &node.value;
        if let Ok(dict) = value.cast::<PyDict>() {
            with_critical_section(value, || {
                sink.reserve(dict.len());
                for (key, item) in dict.iter() {
                    sink.key(&convert::key(&key)?);
                    Self::put(sink, &item, Some(node))?;
                }
                Ok(())
            })
        } else if let Ok(list) = value.cast::<PyList>() {
            with_critical_section(value, || {
                let len = list.len();
                sink.reserve(len);
                for i in 0..len {
                    let item = list.get_item(i).map_err(|e| e.to_string())?;
                    Self::put(sink, &item, Some(node))?;
                }
                Ok(())
            })
        } else {
            let tuple = value.cast::<PyTuple>().map_err(|e| e.to_string())?;
            sink.reserve(tuple.len());
            for item in tuple.iter() {
                Self::put(sink, &item, Some(node))?;
            }
            Ok(())
        }
    }
}
