//! Conversions at the Python boundary: values, options, offsets and errors.

use std::borrow::Cow;
use std::time::Duration;

use jlens_core::{Bindings, Options};
use pyo3::exceptions::{PyException, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::critical_section::with_critical_section;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};
use serde_json::Value as Json;

pyo3::create_exception!(
    jlens,
    JlensError,
    PyException,
    "A JSONata error: `code` is its JSONata code (such as `S0201` or `T2001`), and `position`, \
     `start`, `end`, `token` and `value` say where and on what it happened, when known."
);

/// The message of a cycle, as `json.dumps` gives it.
pub const CIRCULAR: &str = "Circular reference detected";

/// The deepest value [`to_json`] converts, as jlens-core's own limit on values.
const MAX_NESTING: usize = 1000;

/// A Python value that is not an array or an object.
pub enum Scalar<'v> {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Cow<'v, str>),
}

/// Reads a Python value as JSON's `null`, a boolean, a number or a string. Returns `None` for a
/// `dict`, `list` or `tuple`, and the message `json.dumps` would give for anything else. Numbers
/// that are not finite become `null`, and an `int` beyond 64 bits the nearest `float`.
pub fn scalar<'v>(value: &'v Bound<'_, PyAny>) -> Result<Option<Scalar<'v>>, String> {
    Ok(Some(if value.is_none() {
        Scalar::Null
    } else if let Ok(b) = value.cast::<PyBool>() {
        Scalar::Bool(b.is_true())
    } else if value.is_instance_of::<PyInt>() {
        match value.extract::<i64>() {
            Ok(n) => Scalar::Int(n),
            Err(_) => float(value.extract::<f64>().map_err(|e| e.to_string())?),
        }
    } else if value.is_instance_of::<PyFloat>() {
        float(value.extract::<f64>().map_err(|e| e.to_string())?)
    } else if let Ok(s) = value.cast::<PyString>() {
        Scalar::String(text(s))
    } else if value.is_instance_of::<PyDict>()
        || value.is_instance_of::<PyList>()
        || value.is_instance_of::<PyTuple>()
    {
        return Ok(None);
    } else {
        return Err(format!(
            "Object of type {} is not JSON serializable",
            type_name(value)
        ));
    }))
}

fn float(n: f64) -> Scalar<'static> {
    if n.is_finite() {
        Scalar::Float(n)
    } else {
        Scalar::Null
    }
}

/// A `str` as UTF-8. A lone surrogate, which UTF-8 cannot hold, becomes U+FFFD, as it does in
/// the WebAssembly package.
fn text<'v>(s: &'v Bound<'_, PyString>) -> Cow<'v, str> {
    s.to_cow().unwrap_or_else(|_| {
        let units = s
            .call_method1("encode", ("utf-16-le", "surrogatepass"))
            .and_then(|bytes| bytes.extract::<Vec<u8>>())
            .unwrap_or_default();
        let units: Vec<u16> = units
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        Cow::Owned(String::from_utf16_lossy(&units))
    })
}

/// Reads a key of a `dict`, which must be a `str`.
pub fn key<'v>(key: &'v Bound<'_, PyAny>) -> Result<Cow<'v, str>, String> {
    match key.cast::<PyString>() {
        Ok(s) => Ok(text(s)),
        Err(_) => Err(format!("keys must be str, not {}", type_name(key))),
    }
}

fn type_name(value: &Bound<'_, PyAny>) -> String {
    value
        .get_type()
        .name()
        .map_or_else(|_| "?".into(), |name| name.to_string())
}

/// Converts a Python value whole, with the rules of the evaluation's input: a binding, or a
/// `Document`.
pub fn to_json(value: &Bound<'_, PyAny>, what: &str) -> PyResult<Json> {
    let mut ancestors = Vec::new();
    to_json_level(value, &mut ancestors).map_err(|e| match e {
        Invalid::Type(message) => {
            PyTypeError::new_err(format!("{what} cannot be converted to JSON: {message}"))
        }
        Invalid::Value(message) => {
            PyValueError::new_err(format!("{what} cannot be converted to JSON: {message}"))
        }
    })
}

enum Invalid {
    Type(String),
    Value(String),
}

fn to_json_level<'py>(
    value: &Bound<'py, PyAny>,
    ancestors: &mut Vec<Bound<'py, PyAny>>,
) -> Result<Json, Invalid> {
    match scalar(value).map_err(Invalid::Type)? {
        Some(Scalar::Null) => return Ok(Json::Null),
        Some(Scalar::Bool(b)) => return Ok(Json::Bool(b)),
        Some(Scalar::Int(n)) => return Ok(Json::from(n)),
        Some(Scalar::Float(n)) => return Ok(Json::from(n)),
        Some(Scalar::String(s)) => return Ok(Json::String(s.into_owned())),
        None => {}
    }
    if ancestors.iter().any(|a| a.is(value)) {
        return Err(Invalid::Value(CIRCULAR.into()));
    }
    if ancestors.len() >= MAX_NESTING {
        return Err(Invalid::Value(format!(
            "it is nested more than {MAX_NESTING} levels deep"
        )));
    }
    // The members are taken under the lock, and converted after it, so that the lock of a
    // nested value never waits while this one is held.
    let members = with_critical_section(value, || members(value));
    ancestors.push(value.clone());
    let result = jlens_core::stack::grow(|| -> Result<Json, Invalid> {
        Ok(match members.map_err(Invalid::Type)? {
            Members::Items(items) => Json::Array(
                items
                    .iter()
                    .map(|item| to_json_level(item, ancestors))
                    .collect::<Result<_, _>>()?,
            ),
            Members::Fields(fields) => {
                let mut map = serde_json::Map::with_capacity(fields.len());
                for (key, item) in &fields {
                    let key = self::key(key).map_err(Invalid::Type)?.into_owned();
                    map.insert(key, to_json_level(item, ancestors)?);
                }
                Json::Object(map)
            }
        })
    });
    ancestors.pop();
    result
}

enum Members<'py> {
    Items(Vec<Bound<'py, PyAny>>),
    Fields(Vec<(Bound<'py, PyAny>, Bound<'py, PyAny>)>),
}

fn members<'py>(value: &Bound<'py, PyAny>) -> Result<Members<'py>, String> {
    if let Ok(dict) = value.cast::<PyDict>() {
        Ok(Members::Fields(dict.iter().collect()))
    } else if let Ok(list) = value.cast::<PyList>() {
        Ok(Members::Items(list.iter().collect()))
    } else {
        let tuple = value.cast::<PyTuple>().map_err(|e| e.to_string())?;
        Ok(Members::Items(tuple.iter().collect()))
    }
}

/// Converts a result into Python objects. An integral number below 10^21 becomes an `int`, as
/// `json.loads` reads the number jsonata-js writes for it; other numbers become a `float`.
pub fn to_py<'py>(py: Python<'py>, value: &Json) -> PyResult<Bound<'py, PyAny>> {
    jlens_core::stack::grow(|| {
        Ok(match value {
            Json::Null => py.None().into_bound(py),
            Json::Bool(b) => PyBool::new(py, *b).to_owned().into_any(),
            Json::Number(n) => {
                if let Some(i) = n.as_i64() {
                    i.into_pyobject(py)?.into_any()
                } else if let Some(u) = n.as_u64() {
                    u.into_pyobject(py)?.into_any()
                } else {
                    let f = n.as_f64().unwrap_or(f64::NAN);
                    if f.fract() == 0.0 && f.abs() < 1e21 {
                        (f as i128).into_pyobject(py)?.into_any()
                    } else {
                        f.into_pyobject(py)?.into_any()
                    }
                }
            }
            Json::String(s) => PyString::new(py, s).into_any(),
            Json::Array(items) => {
                let list = PyList::empty(py);
                for item in items {
                    list.append(to_py(py, item)?)?;
                }
                list.into_any()
            }
            Json::Object(map) => {
                let dict = PyDict::new(py);
                for (key, item) in map {
                    dict.set_item(key, to_py(py, item)?)?;
                }
                dict.into_any()
            }
        })
    })
}

/// Reads the `bindings` argument: `None` or a `dict` of names (without the `$`) to values.
pub fn bindings(value: Option<&Bound<'_, PyAny>>) -> PyResult<Bindings> {
    let mut out = Bindings::new();
    let Some(value) = value.filter(|v| !v.is_none()) else {
        return Ok(out);
    };
    let dict = value
        .cast::<PyDict>()
        .map_err(|_| PyTypeError::new_err("bindings must be a dict"))?;
    let entries: Vec<_> = with_critical_section(value, || dict.iter().collect());
    for (name, value) in entries {
        let name = key(&name)
            .map_err(|_| PyTypeError::new_err("the names of the bindings must be str"))?
            .into_owned();
        let json = to_json(&value, &format!("binding {name}"))?;
        out = out.var(name, json);
    }
    Ok(out)
}

/// Reads the evaluation options, given as keyword arguments: `timeout` (seconds), `max_depth`,
/// `max_string_length`, `max_array_length`, `max_result_size`, `now` and `seed`. `None` leaves
/// an option at its default.
pub fn options(options: Option<&Bound<'_, PyDict>>) -> PyResult<Options> {
    let mut out = Options::default();
    let Some(options) = options else {
        return Ok(out);
    };
    for (name, value) in options.iter() {
        if value.is_none() {
            continue;
        }
        let name: String = name.extract()?;
        let count = || -> PyResult<usize> {
            if value.is_instance_of::<PyBool>() || !value.is_instance_of::<PyInt>() {
                return Err(PyTypeError::new_err(format!("{name} must be an int")));
            }
            value
                .extract()
                .map_err(|_| PyValueError::new_err(format!("{name} must not be negative")))
        };
        match name.as_str() {
            "timeout" => {
                let seconds = number(&value)
                    .ok_or_else(|| PyTypeError::new_err("timeout must be a number of seconds"))?;
                out.timeout = Some(Duration::try_from_secs_f64(seconds).map_err(|_| {
                    PyValueError::new_err("timeout must be a non-negative number of seconds")
                })?);
            }
            "max_depth" => out.max_depth = Some(count()?),
            "max_string_length" => out.max_string_length = Some(count()?),
            "max_array_length" => out.max_array_length = Some(count()?),
            "max_result_size" => out.max_result_size = Some(count()?),
            "seed" => out.seed = Some(count()? as u64),
            "now" => out.now = Some(now(&value)?),
            _ => {
                return Err(PyTypeError::new_err(format!(
                    "unexpected keyword argument '{name}'"
                )));
            }
        }
    }
    Ok(out)
}

/// An `int` or a `float`, not a `bool`.
fn number(value: &Bound<'_, PyAny>) -> Option<f64> {
    if value.is_instance_of::<PyBool>() {
        return None;
    }
    if value.is_instance_of::<PyInt>() || value.is_instance_of::<PyFloat>() {
        value.extract().ok()
    } else {
        None
    }
}

/// Reads `now`: milliseconds since the epoch, a `datetime` (through its `timestamp()`) or an
/// ISO 8601 timestamp.
fn now(value: &Bound<'_, PyAny>) -> PyResult<i64> {
    let invalid = || {
        PyValueError::new_err(
            "now must be milliseconds since the epoch, a datetime or an ISO 8601 timestamp",
        )
    };
    let millis = if let Some(n) = number(value) {
        n
    } else if let Ok(text) = value.cast::<PyString>() {
        let expr = jlens_core::Expression::compile("$toMillis($t)").expect("a valid expression");
        let bindings = Bindings::new().var("t", text.to_cow()?.as_ref());
        match expr.evaluate(None, &bindings) {
            Ok(Some(Json::Number(n))) => n.as_f64().ok_or_else(invalid)?,
            _ => return Err(invalid()),
        }
    } else if value.hasattr("timestamp")? {
        value.call_method0("timestamp")?.extract::<f64>()? * 1000.0
    } else {
        return Err(PyTypeError::new_err(
            "now must be milliseconds since the epoch, a datetime or an ISO 8601 timestamp",
        ));
    };
    if !millis.is_finite() || millis.abs() >= 9.0e15 {
        return Err(invalid());
    }
    Ok(millis.round() as i64)
}

/// A `JlensError` for `err`, raised by the expression `source`. Its offsets count code points,
/// the unit of Python's `str`.
pub fn error(py: Python<'_>, err: &jlens_core::Error, source: &str) -> PyErr {
    let exception = JlensError::new_err(err.message());
    let instance = exception.value(py);
    let span = err.span();
    let offset = |offset: Option<usize>| -> Bound<'_, PyAny> {
        match offset {
            Some(offset) => offset
                .into_pyobject(py)
                .map_or_else(|e| match e {}, Bound::into_any),
            None => py.None().into_bound(py),
        }
    };
    let value = match err.value().map(|v| to_py(py, v)).transpose() {
        Ok(value) => value.unwrap_or_else(|| py.None().into_bound(py)),
        Err(e) => return e,
    };
    let fields = [
        ("code", PyString::new(py, err.code()).into_any()),
        (
            "position",
            offset(err.position().map(|p| utf16_to_chars(source, p))),
        ),
        (
            "start",
            offset(span.map(|s| bytes_to_chars(source, s.start))),
        ),
        ("end", offset(span.map(|s| bytes_to_chars(source, s.end)))),
        (
            "token",
            err.token().map_or_else(
                || py.None().into_bound(py),
                |t| PyString::new(py, &t).into_any(),
            ),
        ),
        ("value", value),
    ];
    for (name, value) in fields {
        if let Err(e) = instance.setattr(name, value) {
            return e;
        }
    }
    exception
}

/// The offset in code points of an offset in UTF-16 code units.
fn utf16_to_chars(text: &str, units: usize) -> usize {
    let mut seen = 0;
    for (chars, c) in text.chars().enumerate() {
        if seen >= units {
            return chars;
        }
        seen += c.len_utf16();
    }
    text.chars().count()
}

/// The offset in code points of an offset in bytes.
fn bytes_to_chars(text: &str, bytes: usize) -> usize {
    text.char_indices().take_while(|(i, _)| *i < bytes).count()
}
