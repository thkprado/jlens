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
//!
//! A key is a `u32` index into the keys of the result, in the order they first came: an index
//! equal to the number of keys so far is a new key, whose `u32` length and UTF-8 bytes follow.
//! So each distinct key crosses once, and the host makes one string for it. A field's value
//! comes before its key because the core builds a value before it inserts it.

use std::collections::HashMap;

use foldhash::fast::RandomState;
use jlens_core::output::Builder;

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

/// 2^63: the integral numbers from -2^63 up to it, not included, fit an `i64`.
const I64_BOUND: f64 = 9_223_372_036_854_775_808.0;

/// Writes a result in the encoding above, after the bytes already in `out` (a reply's first
/// byte).
pub struct Encoder {
    /// The reply being written.
    pub out: Vec<u8>,
    /// The index of each key written so far.
    keys: HashMap<Box<str>, u32, RandomState>,
}

impl Encoder {
    /// An encoder that writes after `out`.
    pub fn new(out: Vec<u8>) -> Self {
        Encoder {
            out,
            keys: HashMap::default(),
        }
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
}
