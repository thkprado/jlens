//! Results written straight into another representation (ADR 0017).
//!
//! [`Expression::evaluate`](crate::Expression::evaluate) returns its result as a
//! `serde_json::Value`. An application that needs the result in another representation (a host
//! language's objects, another JSON library) implements [`Builder`] for it and evaluates with
//! [`Expression::evaluate_source_to`](crate::Expression::evaluate_source_to) or
//! [`Expression::evaluate_document_to`](crate::Expression::evaluate_document_to), so that the
//! result is not built twice. It is the counterpart of [`input::Source`](crate::input::Source).
//!
//! The entry points give the result to a closure as an [`Output`], which
//! [`Output::write`] writes with a builder, within the size limit of a result
//! (`Options::max_result_size`) and the nesting limit, as `evaluate` converts it.
//!
//! ```
//! use jlens_core::output::Builder;
//! use jlens_core::{Bindings, Document, Expression, Options};
//!
//! /// Writes a result as compact text, with its keys in order.
//! struct Text;
//!
//! impl Builder for Text {
//!     type Value = String;
//!     type Array = Vec<String>;
//!     type Object = Vec<String>;
//!
//!     fn null(&mut self) -> String {
//!         "null".into()
//!     }
//!     fn bool(&mut self, value: bool) -> String {
//!         value.to_string()
//!     }
//!     fn number(&mut self, value: f64) -> String {
//!         value.to_string()
//!     }
//!     fn string(&mut self, value: &str) -> String {
//!         format!("{value:?}")
//!     }
//!     fn array(&mut self, len: usize) -> Vec<String> {
//!         Vec::with_capacity(len)
//!     }
//!     fn push(&mut self, array: &mut Vec<String>, item: String) {
//!         array.push(item);
//!     }
//!     fn end_array(&mut self, array: Vec<String>) -> String {
//!         format!("[{}]", array.join(","))
//!     }
//!     fn object(&mut self, len: usize) -> Vec<String> {
//!         Vec::with_capacity(len)
//!     }
//!     fn insert(&mut self, object: &mut Vec<String>, key: &str, value: String) {
//!         object.push(format!("{key:?}:{value}"));
//!     }
//!     fn end_object(&mut self, object: Vec<String>) -> String {
//!         format!("{{{}}}", object.join(","))
//!     }
//! }
//!
//! let document = Document::new(&serde_json::json!({"items": [{"price": 2}, {"price": 3.5}]}));
//! let expr = Expression::compile("{'total': $sum(items.price), 'none': $x}").unwrap();
//! let result = expr.evaluate_document_to(&document, &Bindings::new(), &Options::default(), |out| {
//!     out.write(&mut Text)
//! });
//! assert_eq!(result.unwrap().as_deref(), Some(r#"{"total":5.5}"#));
//! ```

use std::rc::Rc;

use crate::Result;
use crate::value::{Shared, Value};

/// Builds a result in another representation, one value at a time (see the
/// [module](self) documentation).
///
/// The values of a result are written in order, depth first: an array is started with
/// [`array`](Builder::array), given its items with [`push`](Builder::push) and finished with
/// [`end_array`](Builder::end_array); an object likewise, with each field given with its key.
/// Undefined values and functions are left out of objects and written as `null` in arrays, as
/// `JSON.stringify` does.
///
/// The methods cannot fail. A builder whose representation can fail (a host language that runs
/// out of memory) keeps its first failure, returns any value of its type, and reports the failure
/// once the evaluation has returned.
pub trait Builder {
    /// A value built: the result, or a part of it.
    type Value;
    /// An array being built.
    type Array;
    /// An object being built.
    type Object;

    /// Builds `null`.
    fn null(&mut self) -> Self::Value;

    /// Builds a boolean.
    fn bool(&mut self, value: bool) -> Self::Value;

    /// Builds a number, always finite: a number that is not finite is written with
    /// [`null`](Builder::null), as JSON has none. An integer is an `f64` without a fraction;
    /// jsonata-js prints one below 10^21 without a decimal point.
    fn number(&mut self, value: f64) -> Self::Value;

    /// Builds a string.
    fn string(&mut self, value: &str) -> Self::Value;

    /// Starts an array of `len` items.
    fn array(&mut self, len: usize) -> Self::Array;

    /// Adds an item to an array.
    fn push(&mut self, array: &mut Self::Array, item: Self::Value);

    /// Finishes an array.
    fn end_array(&mut self, array: Self::Array) -> Self::Value;

    /// Starts an object of at most `len` fields: the undefined ones are left out.
    fn object(&mut self, len: usize) -> Self::Object;

    /// Adds a field to an object. Keys are distinct within an object, and come in order. The same
    /// keys come back in many objects of a result, as in an array of records: a builder whose keys
    /// cost to create may keep them.
    fn insert(&mut self, object: &mut Self::Object, key: &str, value: Self::Value);

    /// Finishes an object.
    fn end_object(&mut self, object: Self::Object) -> Self::Value;

    /// Builds an array or object of the input, which a source wrote as `member` of a
    /// [`Sources`](crate::input::Sources) and the evaluation left unchanged, as a whole: `None`,
    /// the default, builds it value by value instead. A builder over the objects its input was
    /// read from can return the object itself, as a member stands for its source's object. Such
    /// a value is not counted against the size limit or the nesting limit, since nothing of it
    /// is built, and what of it was left unread stays unread.
    fn input(&mut self, member: Member) -> Option<Self::Value> {
        let _ = member;
        None
    }
}

/// An array or object of the input, read from a [`Sources`](crate::input::Sources): what
/// [`Builder::input`] is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
    /// The address of the sources, which identifies them while they live.
    sources: usize,
    index: u32,
}

impl Member {
    pub(crate) fn new<S: ?Sized>(sources: &Rc<S>, index: u32) -> Member {
        Member {
            sources: Rc::as_ptr(sources).cast::<()>().addr(),
            index,
        }
    }

    /// Whether the member is one of `sources`: the sources a builder's input was read from.
    pub fn of<S: ?Sized>(&self, sources: &Rc<S>) -> bool {
        Rc::as_ptr(sources).cast::<()>().addr() == self.sources
    }

    /// The member's index among its sources.
    pub fn index(&self) -> u32 {
        self.index
    }
}

/// The result of an evaluation, not written yet. [`Output::write`] writes it.
pub struct Output<'r, 'a> {
    pub(crate) value: &'r Value<'a>,
    pub(crate) limit: usize,
    pub(crate) shared: Option<&'r Shared>,
}

impl Output<'_, '_> {
    /// Writes the result with `builder`: `None` when it is undefined. A result whose compact JSON
    /// text would be longer than the size limit of a result, or nested more than 1,000 levels
    /// deep, fails with `U1001`, and writing stops there. Writing reads what the evaluation left
    /// unread of the input: a level that a source fails to read fails with `U1004`.
    pub fn write<B: Builder>(self, builder: &mut B) -> Result<Option<B::Value>> {
        let limit = self.limit;
        let mut budget = limit;
        let written = self
            .value
            .write_spending(builder, &mut budget, 0)
            .map_err(|e| {
                e.error(|| format!("The result would exceed the size limit of {limit} bytes"))
            })?;
        match self.shared {
            Some(shared) if shared.failed() => Err(shared.failure()),
            _ => Ok(written),
        }
    }
}
