//! Results written as values for the host to build its own objects from (ADR 0017), in a tagged
//! binary encoding that it decodes with no parser and no call back into Rust (0.25 #5).
//!
//! A value is a tag byte, then what the tag says follows, integers little-endian:
//!
//! | Tag | Constant | What follows |
//! |---|---|---|
//! | 0 | [`NULL`] | nothing |
//! | 1 | [`FALSE`] | nothing |
//! | 2 | [`TRUE`] | nothing |
//! | 3 | [`INTEGER`] | an `i64`: an integral number from -2^63 to 2^63 - 1, a `long`'s range |
//! | 4 | [`FLOAT`] | an `f64`: any other number, always finite |
//! | 5 | [`STRING`] | a `u32` length, then that many bytes of UTF-8 |
//! | 6 | [`ARRAY`] | a `u32` count, then that many values |
//! | 7 | [`OBJECT`] | a `u32` count, then that many fields: a value, then its key |
//! | 8 | [`INPUT`] | a `u32`: the number of an array or object of the input, unchanged ([`crate::source`]) |
//! | 9 | [`FUNCTION`] | a `u32`: the arity of a function among a host function's arguments ([`crate::host`]) |
//! | 10 | [`UNDEFINED`] | nothing: an undefined argument of a host function, or of a function it calls |
//!
//! A key is a `u32` index into the keys of the result, in the order they first came: an index
//! equal to the number of keys so far is a new key, whose `u32` length and UTF-8 bytes follow.
//! So each distinct key crosses once, and the host makes one string for it. A field's value
//! comes before its key because the core builds a value before it inserts it.
//!
//! The host writes values in the same encoding, for its host functions' results, the arguments
//! of the functions they call and the variables of its bindings ([`Decoder`]). It writes no
//! `INPUT` and no `FUNCTION`, and `UNDEFINED` only as an argument.

use std::collections::HashMap;
use std::rc::Rc;

use foldhash::fast::RandomState;
use jlens_core::Error;
use jlens_core::output::{Builder, Member};
use serde_json::{Map, Number, Value};

use crate::source::{Host, INPUT};

/// The tag of `null`.
pub const NULL: u8 = 0;
/// The tag of `false`.
pub const FALSE: u8 = 1;
/// The tag of `true`.
pub const TRUE: u8 = 2;
/// The tag of an integral number in the range of an `i64`, as one.
pub const INTEGER: u8 = 3;
/// The tag of any other number, as an `f64`.
pub const FLOAT: u8 = 4;
/// The tag of a string.
pub const STRING: u8 = 5;
/// The tag of an array.
pub const ARRAY: u8 = 6;
/// The tag of an object.
pub const OBJECT: u8 = 7;
/// The tag of a function among a host function's arguments: its `u32` arity follows.
pub const FUNCTION: u8 = 9;
/// The tag of an undefined argument.
pub const UNDEFINED: u8 = 10;

/// The deepest the host's values may nest, as deep as the core lets a result nest: deeper, a
/// value would be dropped by recursion.
const MAX_NESTING: usize = 1_000;

/// 2^63: the integral numbers from -2^63 up to it, not included, fit an `i64`.
const I64_BOUND: f64 = 9_223_372_036_854_775_808.0;

/// Writes a result in the encoding above, after the bytes already in `out` (a reply's first
/// byte).
pub struct Encoder {
    /// The reply being written.
    pub out: Vec<u8>,
    /// The index of each key written so far.
    keys: HashMap<Box<str>, u32, RandomState>,
    /// The host's arrays and objects the input was read from, written by their numbers.
    input: Option<Rc<Host>>,
}

impl Encoder {
    /// An encoder that writes after `out`.
    pub fn new(out: Vec<u8>) -> Self {
        Encoder {
            out,
            keys: HashMap::default(),
            input: None,
        }
    }

    /// An encoder that writes after `out`, and writes the arrays and objects of `input` the
    /// result holds unchanged by their numbers ([`INPUT`]).
    pub fn with_input(out: Vec<u8>, input: Rc<Host>) -> Self {
        Encoder {
            input: Some(input),
            ..Encoder::new(out)
        }
    }

    /// Writes JSON `value`.
    pub fn json(&mut self, value: &Value) {
        jlens_core::stack::grow(|| match value {
            Value::Null => self.null(),
            Value::Bool(value) => self.bool(*value),
            Value::Number(n) => self.number(n.as_f64().unwrap_or_default()),
            Value::String(value) => self.string(value),
            Value::Array(items) => {
                let mut array = self.array(items.len());
                for item in items {
                    self.json(item);
                    self.push(&mut array, ());
                }
                self.end_array(array);
            }
            Value::Object(fields) => {
                let mut object = self.object(fields.len());
                for (key, value) in fields {
                    self.json(value);
                    self.insert(&mut object, key, ());
                }
                self.end_object(object);
            }
        })
    }

    /// Writes `count` in a `u32`, as counts and lengths are.
    pub fn count(&mut self, count: usize) {
        self.u32(count);
    }

    fn u32(&mut self, value: usize) {
        // A result is at most `max_result_size` bytes of JSON, below 4 GiB unless an
        // application raised it that far; its counts and lengths are below that.
        self.out
            .extend_from_slice(&u32::try_from(value).unwrap_or(u32::MAX).to_le_bytes());
    }

    fn text(&mut self, value: &str) {
        self.u32(value.len());
        self.out.extend_from_slice(value.as_bytes());
    }

    /// Writes a `u32` placeholder for a count: where to patch it.
    fn placeholder(&mut self) -> usize {
        let at = self.out.len();
        self.out.extend_from_slice(&[0; 4]);
        at
    }

    fn patch(&mut self, at: usize, count: u32) {
        self.out[at..at + 4].copy_from_slice(&count.to_le_bytes());
    }
}

/// An array or an object being written: where its count goes, and the count so far.
pub struct Open {
    at: usize,
    count: u32,
}

impl Builder for Encoder {
    // The values are written into `out` as they come, so a value is nothing.
    type Value = ();
    type Array = Open;
    type Object = Open;

    fn null(&mut self) {
        self.out.push(NULL);
    }

    fn bool(&mut self, value: bool) {
        self.out.push(if value { TRUE } else { FALSE });
    }

    fn number(&mut self, value: f64) {
        if value.fract() == 0.0 && (-I64_BOUND..I64_BOUND).contains(&value) {
            self.out.push(INTEGER);
            // In range and integral: the cast is exact.
            self.out.extend_from_slice(&(value as i64).to_le_bytes());
        } else {
            self.out.push(FLOAT);
            self.out.extend_from_slice(&value.to_le_bytes());
        }
    }

    fn string(&mut self, value: &str) {
        self.out.push(STRING);
        self.text(value);
    }

    fn array(&mut self, _len: usize) -> Open {
        self.out.push(ARRAY);
        Open {
            at: self.placeholder(),
            count: 0,
        }
    }

    fn push(&mut self, array: &mut Open, _item: ()) {
        array.count += 1;
    }

    fn end_array(&mut self, array: Open) {
        self.patch(array.at, array.count);
    }

    fn object(&mut self, _len: usize) -> Open {
        self.out.push(OBJECT);
        Open {
            at: self.placeholder(),
            count: 0,
        }
    }

    fn insert(&mut self, object: &mut Open, key: &str, _value: ()) {
        object.count += 1;
        let next = u32::try_from(self.keys.len()).unwrap_or(u32::MAX);
        match self.keys.get(key) {
            Some(&index) => self.out.extend_from_slice(&index.to_le_bytes()),
            None => {
                self.keys.insert(key.into(), next);
                self.out.extend_from_slice(&next.to_le_bytes());
                self.text(key);
            }
        }
    }

    fn end_object(&mut self, object: Open) {
        self.patch(object.at, object.count);
    }

    fn input(&mut self, member: Member) -> Option<()> {
        if !member.of(self.input.as_ref()?) {
            return None;
        }
        self.out.push(INPUT);
        self.out.extend_from_slice(&member.index().to_le_bytes());
        Some(())
    }
}

/// Reads the values the host writes, as JSON: a number as an `f64` or an `i64`, one that is not
/// finite as `null`.
pub struct Decoder<'b> {
    bytes: &'b [u8],
    keys: Vec<String>,
}

impl<'b> Decoder<'b> {
    /// A decoder of `bytes`.
    pub fn new(bytes: &'b [u8]) -> Self {
        Decoder {
            bytes,
            keys: Vec::new(),
        }
    }

    /// Fails unless every byte was read.
    pub fn end(&self) -> Result<(), Error> {
        match self.bytes.is_empty() {
            true => Ok(()),
            false => Err(malformed("bytes left over after the values")),
        }
    }

    /// The next value.
    pub fn value(&mut self) -> Result<Value, Error> {
        self.nested(0)
    }

    /// The next value, `None` for [`UNDEFINED`].
    pub fn argument(&mut self) -> Result<Option<Value>, Error> {
        match self.bytes.first() {
            Some(&UNDEFINED) => {
                self.bytes = &self.bytes[1..];
                Ok(None)
            }
            _ => self.value().map(Some),
        }
    }

    /// The arguments of a call: an [`ARRAY`] of values and [`UNDEFINED`]s.
    pub fn arguments(&mut self) -> Result<Vec<Option<Value>>, Error> {
        if self.byte()? != ARRAY {
            return Err(malformed("arguments that are not an array"));
        }
        let count = self.u32()?;
        // Each argument takes a byte at least.
        let mut arguments = Vec::with_capacity(count.min(self.bytes.len()));
        for _ in 0..count {
            arguments.push(self.argument()?);
        }
        Ok(arguments)
    }

    fn nested(&mut self, depth: usize) -> Result<Value, Error> {
        if depth > MAX_NESTING {
            return Err(Error::new("U1001").with(
                "message",
                format!("A value is nested more than {MAX_NESTING} levels deep"),
            ));
        }
        jlens_core::stack::grow(|| {
            Ok(match self.byte()? {
                NULL => Value::Null,
                FALSE => Value::Bool(false),
                TRUE => Value::Bool(true),
                INTEGER => Value::from(i64::from_le_bytes(self.take()?)),
                FLOAT => Number::from_f64(f64::from_le_bytes(self.take()?))
                    .map_or(Value::Null, Value::Number),
                STRING => Value::String(self.text()?.to_owned()),
                ARRAY => {
                    let count = self.u32()?;
                    let mut items = Vec::with_capacity(count.min(self.bytes.len()));
                    for _ in 0..count {
                        items.push(self.nested(depth + 1)?);
                    }
                    Value::Array(items)
                }
                OBJECT => {
                    let count = self.u32()?;
                    let mut fields = Map::with_capacity(count.min(self.bytes.len()));
                    for _ in 0..count {
                        let value = self.nested(depth + 1)?;
                        fields.insert(self.key()?, value);
                    }
                    Value::Object(fields)
                }
                tag => return Err(malformed(&format!("a value of unknown tag {tag}"))),
            })
        })
    }

    fn key(&mut self) -> Result<String, Error> {
        let index = self.u32()?;
        if index == self.keys.len() {
            let key = self.text()?.to_owned();
            self.keys.push(key);
        }
        self.keys
            .get(index)
            .cloned()
            .ok_or_else(|| malformed(&format!("key {index} out of order")))
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let (head, rest) = self
            .bytes
            .split_first_chunk()
            .ok_or_else(|| malformed("values cut short"))?;
        self.bytes = rest;
        Ok(*head)
    }

    fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.take::<1>()?[0])
    }

    fn u32(&mut self) -> Result<usize, Error> {
        Ok(u32::from_le_bytes(self.take()?) as usize)
    }

    fn text(&mut self) -> Result<&'b str, Error> {
        let len = self.u32()?;
        if self.bytes.len() < len {
            return Err(malformed("values cut short"));
        }
        let (text, rest) = self.bytes.split_at(len);
        self.bytes = rest;
        std::str::from_utf8(text).map_err(|e| malformed(&e.to_string()))
    }
}

/// The `U2005` error of values not in the encoding.
fn malformed(message: &str) -> Error {
    Error::new("U2005")
        .with("token", "values")
        .with("message", message)
}
