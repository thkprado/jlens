//! The input of `evaluate`, read from the Python value one level at a time (ADR 0010, ADR 0014).
//!
//! A `dict` (or a subclass) is read as an object, a `list` or a `tuple` as an array; `str`,
//! `int`, `float`, `bool` and `None` as JSON's values. Anything else, a key that is not a `str`
//! or a cycle stops the evaluation with `U1004`, with the message `json.dumps` would give. No
//! Python code runs while a level is read, and a `dict` or `list` is locked while it is read,
//! so that another thread of free-threaded Python cannot change it halfway.
//!
//! The arrays and objects of a level are written as members of one [`Level`], so that each
//! costs the core one allocation, not two.
//!
//! A path step looks its one field up in a `dict` not read whole yet (`read_field`), with the
//! key's `str` taken from [`Keys`] when the expression names it, so that a lookup neither builds
//! a `str` nor hashes it. That lookup can call the `__eq__` of a key that is not a `str`, whose
//! hash collides with the key's; such a `dict` fails when it is read whole. An index into a
//! `list` or a `tuple` not read whole yet reads the items it picks (`length`, `read_item`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use jlens_core::input::{Sink, Source, Sources};
use pyo3::prelude::*;
use pyo3::sync::critical_section::with_critical_section;
use pyo3::types::{PyDict, PyList, PyString, PyTuple};

use crate::convert::{self, Scalar};

/// The fields an expression reads of its input, as interned `str`s, made once per expression.
pub type Keys = HashMap<Box<str>, Py<PyString>, foldhash::fast::RandomState>;

/// The input itself, the only value of its level, and the keys of the expression.
pub struct PySource<'k, 'py>(pub Bound<'py, PyAny>, pub &'k Keys);

/// A level that holds arrays or objects: its value, the chain of its ancestors, to detect cycles
/// as `json.dumps` does, and its arrays and objects, the members the core reads.
struct Level<'k, 'py> {
    /// The array or object; none for the level of the input.
    value: Option<Bound<'py, PyAny>>,
    keys: &'k Keys,
    /// The level whose member `value` is.
    up: Option<Rc<Level<'k, 'py>>>,
    /// The first member, the level made for it, and the others: a level holding one array or
    /// object allocates nothing more.
    first: Bound<'py, PyAny>,
    others: RefCell<Vec<Bound<'py, PyAny>>>,
    /// This level, so that its members' levels can point to it.
    this: Weak<Level<'k, 'py>>,
}

impl<'k, 'py> Level<'k, 'py> {
    /// Calls `f` with the member at `index`.
    fn with<R>(&self, index: u32, f: impl FnOnce(&Bound<'py, PyAny>) -> R) -> R {
        match index.checked_sub(1) {
            None => f(&self.first),
            Some(i) => f(&self.others.borrow()[i as usize]),
        }
    }
}

/// Writes the values of a level being read, `value` (none for the input), a member of `up`.
struct Writer<'l, 'k, 'py> {
    keys: &'k Keys,
    value: Option<&'l Bound<'py, PyAny>>,
    up: Option<&'l Rc<Level<'k, 'py>>>,
    /// The level of the arrays and objects written, made when the first one is met.
    level: Option<Rc<Level<'k, 'py>>>,
}

impl<'l, 'k, 'py> Writer<'l, 'k, 'py> {
    fn new(
        keys: &'k Keys,
        value: Option<&'l Bound<'py, PyAny>>,
        up: Option<&'l Rc<Level<'k, 'py>>>,
    ) -> Self {
        Writer {
            keys,
            value,
            up,
            level: None,
        }
    }

    /// Writes `item`, a value of the level.
    fn put<'a>(&mut self, sink: &mut Sink<'_, 'a>, item: &Bound<'py, PyAny>) -> Result<(), String>
    where
        'k: 'a,
        'py: 'a,
    {
        match convert::scalar(item)? {
            Some(Scalar::Null) => sink.null(),
            Some(Scalar::Bool(b)) => sink.bool(b),
            Some(Scalar::Int(n)) => sink.number(n as f64),
            Some(Scalar::Float(n)) => sink.number(n),
            Some(Scalar::String(s)) => sink.string(&s),
            None => {
                let mut up = self.up.map(|up| &**up);
                let mut ancestor = self.value;
                while let Some(value) = ancestor {
                    if value.is(item) {
                        return Err(convert::CIRCULAR.into());
                    }
                    ancestor = up.and_then(|level| level.value.as_ref());
                    up = up.and_then(|level| level.up.as_deref());
                }
                let mut made = false;
                let level = self.level.get_or_insert_with(|| {
                    made = true;
                    Rc::new_cyclic(|this| Level {
                        value: self.value.cloned(),
                        keys: self.keys,
                        up: self.up.cloned(),
                        first: item.clone(),
                        others: RefCell::default(),
                        this: this.clone(),
                    })
                });
                let index = match made {
                    true => 0,
                    false => {
                        let mut others = level.others.borrow_mut();
                        others.push(item.clone());
                        u32::try_from(others.len())
                            .map_err(|_| "too many lists and dicts in one level".to_owned())?
                    }
                };
                if item.is_exact_instance_of::<PyDict>() || item.is_instance_of::<PyDict>() {
                    sink.object_of(level, index);
                } else {
                    sink.array_of(level, index);
                }
            }
        }
        Ok(())
    }

    /// Writes the items of the array, or the fields of the object, `value`.
    fn read<'a>(mut self, value: &Bound<'py, PyAny>, sink: &mut Sink<'_, 'a>) -> Result<(), String>
    where
        'k: 'a,
        'py: 'a,
    {
        if let Ok(dict) = value.cast::<PyDict>() {
            with_critical_section(value, || {
                sink.reserve(dict.len());
                for (key, item) in dict.iter() {
                    sink.key(&convert::key(&key)?);
                    self.put(sink, &item)?;
                }
                Ok(())
            })
        } else if let Ok(list) = value.cast::<PyList>() {
            with_critical_section(value, || {
                let len = list.len();
                sink.reserve(len);
                for i in 0..len {
                    let item = list.get_item(i).map_err(|e| e.to_string())?;
                    self.put(sink, &item)?;
                }
                Ok(())
            })
        } else {
            let tuple = value.cast::<PyTuple>().map_err(|e| e.to_string())?;
            sink.reserve(tuple.len());
            for item in tuple.iter() {
                self.put(sink, &item)?;
            }
            Ok(())
        }
    }
}

impl<'k, 'py, 'a> Source<'a> for PySource<'k, 'py>
where
    'k: 'a,
    'py: 'a,
{
    fn read(&self, sink: &mut Sink<'_, 'a>) -> Result<(), String> {
        Writer::new(self.1, None, None).put(sink, &self.0)
    }
}

impl<'k, 'py, 'a> Sources<'a> for Level<'k, 'py>
where
    'k: 'a,
    'py: 'a,
{
    fn read(&self, index: u32, sink: &mut Sink<'_, 'a>) -> Result<(), String> {
        let this = self.this.upgrade();
        self.with(index, |member| {
            Writer::new(self.keys, Some(member), this.as_ref()).read(member, sink)
        })
    }

    fn read_field(&self, index: u32, key: &str, sink: &mut Sink<'_, 'a>) -> Result<bool, String> {
        self.with(index, |member| {
            let Ok(dict) = member.cast::<PyDict>() else {
                return Ok(false);
            };
            let item = match self.keys.get(key) {
                Some(key) => dict.get_item(key.bind(member.py())),
                None => dict.get_item(key),
            };
            if let Some(item) = item.map_err(|e| e.to_string())? {
                let this = self.this.upgrade();
                Writer::new(self.keys, Some(member), this.as_ref()).put(sink, &item)?;
            }
            Ok(true)
        })
    }

    fn length(&self, index: u32) -> Option<usize> {
        self.with(index, |member| {
            if let Ok(list) = member.cast::<PyList>() {
                Some(list.len())
            } else {
                member.cast::<PyTuple>().ok().map(|tuple| tuple.len())
            }
        })
    }

    fn read_item(&self, index: u32, item: usize, sink: &mut Sink<'_, 'a>) -> Result<bool, String> {
        self.with(index, |member| {
            let this = self.this.upgrade();
            let mut writer = Writer::new(self.keys, Some(member), this.as_ref());
            if let Ok(list) = member.cast::<PyList>() {
                // Code that ran since the length was read may have made the list shorter: an
                // item past its end is undefined.
                with_critical_section(member, || match list.get_item(item) {
                    Ok(item) => writer.put(sink, &item),
                    Err(_) => Ok(()),
                })?;
            } else if let Ok(tuple) = member.cast::<PyTuple>() {
                writer.put(sink, &tuple.get_item(item).map_err(|e| e.to_string())?)?;
            } else {
                return Ok(false);
            }
            Ok(true)
        })
    }
}
