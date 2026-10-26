//! Inputs that are the host's own values, read one level at a time through a function of the
//! host, its reader, which Rust calls back (ADR 0010, ADR 0021; 0.25 #6).
//!
//! The host numbers the arrays and objects of its input as it meets them, from 0: they are
//! the members of one [`Sources`] for the whole evaluation, known by their numbers. The reader
//! is called as
//!
//! ```text
//! reader(context, op, node, index, key, key_len) -> u64
//! ```
//!
//! with `context` as the evaluation was given it, and `op` one of:
//!
//! | `op` | Constant | The reader | It returns |
//! |---|---|---|---|
//! | 0 | [`READ`] | writes the items of array `node`, or the fields of object `node` | a level |
//! | 1 | [`FIELD`] | writes the value of the field of object `node` whose key is the `key_len` bytes of UTF-8 at address `key`, nothing when there is none | a level |
//! | 2 | [`LENGTH`] | | the number of items of array `node`, or `u64::MAX` when it cannot tell |
//! | 3 | [`ITEM`] | writes item `index` of array `node`, nothing past its end | a level |
//!
//! The key's bytes are valid during the call only. A level is the handle of a buffer, which
//! stays the host's: Rust reads it before it calls the reader again, so one buffer can serve
//! every call. Its bytes, integers little-endian:
//!
//! - a `u32`: the number of bytes that follow, which may be fewer than the buffer holds;
//! - a kind byte, and what the kind says follows:
//!   - [`VALUES`]: a `u32` count, then that many values;
//!   - [`FIELDS`]: a `u32` count, then that many fields, each a key (a `u32` length and UTF-8)
//!     then its value;
//!   - [`FAILED`]: the UTF-8 message of the reader's failure, which stops the evaluation with
//!     `U1004`;
//!   - [`DECLINED`], for [`FIELD`] and [`ITEM`] only: nothing, and the whole level is read
//!     instead, as for a reader that cannot read one field or item alone.
//!
//! A value is a tag of [`crate::values`]: `NULL`, `FALSE`, `TRUE`, `INTEGER` or `FLOAT` with
//! their bytes (a `FLOAT` that is not finite is `null`, as JSON has none), `STRING` with its
//! length and UTF-8; `ARRAY` or `OBJECT` with the `u32` number of the node, not its contents.
//! The evaluation's input itself is given as a level of [`VALUES`] holding one value, before
//! the evaluation starts. A level that is not in this form fails with `U1004` too.
//!
//! A reply in values gives an array or object of the input that the result holds unchanged as
//! the tag [`INPUT`] and its node's number, so that the host can return its own object.

use std::rc::{Rc, Weak};

use jlens_core::input::{Sink, Source, Sources};

use crate::buffers;
use crate::values::{ARRAY, FALSE, FLOAT, INTEGER, NULL, OBJECT, STRING, TRUE};

/// The host's reader of levels (see the [module](self) documentation).
pub type Reader =
    extern "C" fn(context: u64, op: u32, node: u32, index: u64, key: usize, key_len: usize) -> u64;

/// The reader's operation that writes a whole level.
pub const READ: u32 = 0;
/// The reader's operation that writes one field of an object.
pub const FIELD: u32 = 1;
/// The reader's operation that tells the number of items of an array.
pub const LENGTH: u32 = 2;
/// The reader's operation that writes one item of an array.
pub const ITEM: u32 = 3;

/// The kind of a level of values: an array's items, or the one value of a field, an item or
/// the input.
pub const VALUES: u8 = 0;
/// The kind of a level of fields.
pub const FIELDS: u8 = 1;
/// The kind of a level the reader failed to write.
pub const FAILED: u8 = 2;
/// The kind of a field or item the reader does not read alone.
pub const DECLINED: u8 = 3;

/// The tag of an array or object of the input, in a reply in values: a `u32` node number
/// follows.
pub const INPUT: u8 = 8;

/// The host's arrays and objects, read through its reader.
pub struct Host {
    reader: Reader,
    context: u64,
    /// The host itself, for the members its levels write.
    this: Weak<Host>,
}

impl Host {
    /// The arrays and objects of an evaluation, read through `reader`, given `context`.
    pub fn new(reader: Reader, context: u64) -> Rc<Host> {
        Rc::new_cyclic(|this| Host {
            reader,
            context,
            this: this.clone(),
        })
    }

    /// Calls the reader and writes the level it returns to `sink`: `false` when it declined.
    fn level(
        &self,
        op: u32,
        node: u32,
        index: u64,
        key: &str,
        sink: &mut Sink<'_, '_>,
    ) -> Result<bool, String> {
        let address = key.as_ptr().expose_provenance();
        let level =
            crate::upcall(|| (self.reader)(self.context, op, node, index, address, key.len()));
        // A member is read through its `Rc`, so the host is alive.
        let this = self.this.upgrade().ok_or("the input is gone")?;
        write(level, &this, op != READ, sink)
    }
}

/// Writes the level in buffer `level` to `sink`, its arrays and objects as members of `host`:
/// `false` when the reader declined, which it `may`.
fn write(level: u64, host: &Rc<Host>, may: bool, sink: &mut Sink<'_, '_>) -> Result<bool, String> {
    buffers::with(level, |bytes| {
        let mut bytes = Bytes(bytes);
        let len = bytes.u32()? as usize;
        let mut bytes = Bytes(bytes.0.get(..len).ok_or("a level longer than its buffer")?);
        match bytes.byte()? {
            VALUES => {
                let count = bytes.u32()? as usize;
                sink.reserve(count);
                for _ in 0..count {
                    bytes.value(host, sink)?;
                }
            }
            FIELDS => {
                let count = bytes.u32()? as usize;
                sink.reserve(count);
                for _ in 0..count {
                    sink.key(bytes.text()?);
                    bytes.value(host, sink)?;
                }
            }
            FAILED => return Err(String::from_utf8_lossy(bytes.0).into_owned()),
            DECLINED if may => return Ok(false),
            kind => return Err(format!("a level of kind {kind} where none is expected")),
        }
        match bytes.0.is_empty() {
            true => Ok(true),
            false => Err("bytes left over after a level".into()),
        }
    })
    .ok_or_else(|| format!("the reader returned no level (buffer {level})"))?
}

/// The bytes of a level not decoded yet.
struct Bytes<'b>(&'b [u8]);

impl<'b> Bytes<'b> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let (head, rest) = self.0.split_first_chunk().ok_or("a level cut short")?;
        self.0 = rest;
        Ok(*head)
    }

    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take::<1>()?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        self.take().map(u32::from_le_bytes)
    }

    fn text(&mut self) -> Result<&'b str, String> {
        let len = self.u32()? as usize;
        if self.0.len() < len {
            return Err("a level cut short".into());
        }
        let (text, rest) = self.0.split_at(len);
        self.0 = rest;
        std::str::from_utf8(text).map_err(|e| e.to_string())
    }

    fn value(&mut self, host: &Rc<Host>, sink: &mut Sink<'_, '_>) -> Result<(), String> {
        match self.byte()? {
            NULL => sink.null(),
            FALSE => sink.bool(false),
            TRUE => sink.bool(true),
            INTEGER => sink.number(i64::from_le_bytes(self.take()?) as f64),
            FLOAT => match f64::from_le_bytes(self.take()?) {
                n if n.is_finite() => sink.number(n),
                _ => sink.null(),
            },
            STRING => sink.string(self.text()?),
            ARRAY => sink.array_of(host, self.u32()?),
            OBJECT => sink.object_of(host, self.u32()?),
            tag => return Err(format!("a value of unknown tag {tag}")),
        }
        Ok(())
    }
}

/// The input of an evaluation: the level in buffer `input`, read before the evaluation starts.
pub struct Input<'h> {
    /// The host's arrays and objects.
    pub host: &'h Rc<Host>,
    /// The buffer holding the input as a level of one value.
    pub input: u64,
}

impl Source<'_> for Input<'_> {
    fn read(&self, sink: &mut Sink<'_, '_>) -> Result<(), String> {
        write(self.input, self.host, false, sink).map(drop)
    }
}

impl Sources<'_> for Host {
    fn read(&self, index: u32, sink: &mut Sink<'_, '_>) -> Result<(), String> {
        self.level(READ, index, 0, "", sink).map(drop)
    }

    fn read_field(&self, index: u32, key: &str, sink: &mut Sink<'_, '_>) -> Result<bool, String> {
        self.level(FIELD, index, 0, key, sink)
    }

    fn length(&self, index: u32) -> Option<usize> {
        let length = crate::upcall(|| (self.reader)(self.context, LENGTH, index, 0, 0, 0));
        usize::try_from(length).ok().filter(|_| length != u64::MAX)
    }

    fn read_item(&self, index: u32, item: usize, sink: &mut Sink<'_, '_>) -> Result<bool, String> {
        self.level(ITEM, index, item as u64, "", sink)
    }
}
