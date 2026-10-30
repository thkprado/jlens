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
//! [`ARRAY_WITH`] and [`OBJECT_WITH`] give a node's number and its contents too, as a level of
//! [`VALUES`] or [`FIELDS`] gives them without the kind byte, so that the node is read without
//! calling the reader again: a small array or object, written with the level that holds it
//! (0.26 #4). [`OBJECT_SOME`] gives some of an object's fields, those the expression reads
//! (ADR 0016), a field it reads that the object lacks as [`UNDEFINED`]: the reader is called
//! for any other field, and for the object whole, so that the answer never depends on which
//! fields were given. Contents hold no `ARRAY_WITH`, `OBJECT_WITH` or `OBJECT_SOME`, so that a
//! level stays as large as its values and theirs.
//!
//! The evaluation's input itself is given as a level of [`VALUES`] holding one value, before
//! the evaluation starts. A level that is not in this form fails with `U1004` too, and so does
//! reading an array or object nested more than 1,000 levels deep, as deep as JSON text may
//! nest: the depth of a node is that of the level that last wrote it, plus one, so that a
//! level holding its own node, an array inside itself, stops there rather than being read for
//! ever (0.26 #9).
//!
//! A reply in values gives an array or object of the input that the result holds unchanged as
//! the tag [`INPUT`] and its node's number, so that the host can return its own object.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use jlens_core::input::{Sink, Source, Sources};

use crate::buffers::{self, Buffer};
use crate::values::{
    ARRAY, FALSE, FLOAT, INTEGER, MAX_NESTING, NULL, OBJECT, STRING, TRUE, UNDEFINED,
};

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
/// The tag of an array written with its items, in a level: a `u32` node number, a `u32` count,
/// then that many values.
pub const ARRAY_WITH: u8 = 11;
/// The tag of an object written with its fields, in a level: a `u32` node number, a `u32`
/// count, then that many fields.
pub const OBJECT_WITH: u8 = 12;
/// The tag of an object written with some of its fields, in a level: a `u32` node number, a
/// `u32` count, then that many fields, a field the object lacks with the value tag
/// [`UNDEFINED`].
pub const OBJECT_SOME: u8 = 13;

/// The host's arrays and objects, read through its reader.
pub struct Host {
    reader: Reader,
    context: u64,
    /// The host itself, for the members its levels write.
    this: Weak<Host>,
    /// The buffer of the last level and its handle: a host reuses its buffer, and finding it
    /// again by its handle would take the lock of the table of buffers at each call.
    last: RefCell<Option<(u64, Buffer)>>,
    held: RefCell<Held>,
    depths: RefCell<Depths>,
}

/// The nodes written with their contents, read from the levels that hold them.
#[derive(Default)]
struct Held {
    /// The levels holding contents, copied out of the reader's buffer.
    levels: Vec<Box<[u8]>>,
    /// By node number: the index of its level plus one (0 for a node not held), where its
    /// contents start in it, and the tag it was written with.
    nodes: Vec<(u32, u32, u8)>,
}

/// The depth of each node written, by its number: how many arrays and objects hold it, itself
/// included.
#[derive(Default)]
struct Depths {
    of: Vec<u16>,
    /// The bytes of the levels decoded so far. The host numbers its nodes as it writes them,
    /// from 0, so none has a number this large: one that has is out of the form, which keeps
    /// the tables by node number no longer than the levels.
    read: usize,
}

/// The depths a level's decoding records.
struct Nest<'d> {
    depths: &'d mut Depths,
    /// The depth of the level's arrays and objects.
    depth: u16,
}

impl Nest<'_> {
    /// Records that node `node` is at the level's depth.
    fn node(&mut self, node: u32) -> Result<(), String> {
        let slot = node as usize;
        if slot >= self.depths.read {
            return Err(format!("node {node} is numbered past the nodes written"));
        }
        let of = &mut self.depths.of;
        if slot >= of.len() {
            of.resize(slot + 1, 0);
        }
        of[slot] = self.depth;
        Ok(())
    }
}

impl Held {
    /// The contents of node `node`, when it was written with them, and the tag it was written
    /// with.
    fn contents(&self, node: u32) -> Option<(Bytes<'_>, u8)> {
        let &(level, at, tag) = self.nodes.get(node as usize)?;
        let level = self.levels.get((level as usize).checked_sub(1)?)?;
        Some((Bytes(level.get(at as usize..)?), tag))
    }
}

/// What a level's decoding records of the nodes written with their contents.
struct Record<'h, 'b> {
    held: &'h mut Held,
    /// The level being decoded, and its index in `held` once copied there.
    level: &'b [u8],
    index: Option<u32>,
}

impl Record<'_, '_> {
    /// Records that the contents of node `node`, written with `tag`, start at `rest`, a suffix
    /// of the level; [`Nest::node`] has checked its number.
    fn node(&mut self, node: u32, tag: u8, rest: &[u8]) {
        let slot = node as usize;
        let index = match self.index {
            Some(index) => index,
            None => {
                self.held.levels.push(self.level.into());
                *self.index.insert(self.held.levels.len() as u32)
            }
        };
        let nodes = &mut self.held.nodes;
        if slot >= nodes.len() {
            nodes.resize(slot + 1, (0, 0, 0));
        }
        nodes[slot] = (index, (self.level.len() - rest.len()) as u32, tag);
    }
}

impl Host {
    /// The arrays and objects of an evaluation, read through `reader`, given `context`.
    pub fn new(reader: Reader, context: u64) -> Rc<Host> {
        Rc::new_cyclic(|this| Host {
            reader,
            context,
            this: this.clone(),
            last: RefCell::new(None),
            held: RefCell::default(),
            depths: RefCell::default(),
        })
    }

    /// The depth of the arrays and objects that node `node` holds; an error for a node nested
    /// more than [`MAX_NESTING`] levels deep, which is not read.
    fn inside(&self, node: u32) -> Result<u16, String> {
        let depths = self.depths.borrow();
        let depth = depths.of.get(node as usize).copied().unwrap_or_default();
        match usize::from(depth) <= MAX_NESTING {
            true => Ok(depth + 1),
            false => Err(format!(
                "an array or object is nested more than {MAX_NESTING} levels deep, or inside itself"
            )),
        }
    }

    /// Calls the reader and writes the level it returns, at `depth`, to `sink`: `false` when
    /// it declined.
    fn level(
        &self,
        op: u32,
        node: u32,
        index: u64,
        key: &str,
        depth: u16,
        sink: &mut Sink<'_, '_>,
    ) -> Result<bool, String> {
        let address = key.as_ptr().expose_provenance();
        let level =
            crate::upcall(|| (self.reader)(self.context, op, node, index, address, key.len()));
        let mut last = self.last.borrow_mut();
        let buffer = match &*last {
            Some((handle, buffer)) if *handle == level => buffer,
            _ => {
                let buffer = buffers::get(level).ok_or_else(|| no_level(level))?;
                &last.insert((level, buffer)).1
            }
        };
        buffer.with(|bytes| self.write(bytes, op != READ, depth, sink))
    }

    /// Writes the level in `bytes` to `sink`, its arrays and objects as members of this host at
    /// `depth`: `false` when the reader declined, which it `may`.
    fn write(
        &self,
        bytes: &[u8],
        may: bool,
        depth: u16,
        sink: &mut Sink<'_, '_>,
    ) -> Result<bool, String> {
        // A member is read through its `Rc`, so the host is alive.
        let this = self.this.upgrade().ok_or("the input is gone")?;
        let mut bytes = Bytes(bytes);
        let len = bytes.u32()? as usize;
        let level = bytes.0.get(..len).ok_or("a level longer than its buffer")?;
        let mut bytes = Bytes(level);
        let mut depths = self.depths.borrow_mut();
        depths.read = depths.read.saturating_add(level.len());
        let mut nest = Nest {
            depths: &mut depths,
            depth,
        };
        let mut held = self.held.borrow_mut();
        let mut record = Record {
            held: &mut held,
            level,
            index: None,
        };
        let kind = bytes.byte()?;
        match kind {
            VALUES | FIELDS => {
                bytes.contents(kind == FIELDS, &this, sink, Some(&mut record), &mut nest)?
            }
            FAILED => return Err(String::from_utf8_lossy(bytes.0).into_owned()),
            DECLINED if may => return Ok(false),
            kind => return Err(format!("a level of kind {kind} where none is expected")),
        }
        match bytes.0.is_empty() {
            true => Ok(true),
            false => Err("bytes left over after a level".into()),
        }
    }

    /// Runs `f` on the contents of node `node` when it was written with them, the tag it was
    /// written with, and the depths of the nodes; `None` when it was not, or when `f` gives
    /// none.
    fn held<R>(
        &self,
        node: u32,
        f: impl FnOnce(Bytes<'_>, u8, &mut Depths) -> Option<R>,
    ) -> Option<R> {
        let held = self.held.borrow();
        let (bytes, tag) = held.contents(node)?;
        f(bytes, tag, &mut self.depths.borrow_mut())
    }
}

fn no_level(level: u64) -> String {
    format!("the reader returned no level (buffer {level})")
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

    fn utf8(&mut self) -> Result<&'b [u8], String> {
        let len = self.u32()? as usize;
        if self.0.len() < len {
            return Err("a level cut short".into());
        }
        let (text, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(text)
    }

    fn text(&mut self) -> Result<&'b str, String> {
        std::str::from_utf8(self.utf8()?).map_err(|e| e.to_string())
    }

    /// Writes a count and that many values, or `fields`, to `sink`. Nodes written with their
    /// contents are recorded in `record`; contents read from a node held have none.
    fn contents(
        &mut self,
        fields: bool,
        host: &Rc<Host>,
        sink: &mut Sink<'_, '_>,
        mut record: Option<&mut Record<'_, '_>>,
        nest: &mut Nest<'_>,
    ) -> Result<(), String> {
        let count = self.u32()? as usize;
        sink.reserve(count);
        for _ in 0..count {
            if fields {
                sink.key(self.text()?);
            }
            self.value(host, sink, record.as_deref_mut(), nest)?;
        }
        Ok(())
    }

    fn value(
        &mut self,
        host: &Rc<Host>,
        sink: &mut Sink<'_, '_>,
        record: Option<&mut Record<'_, '_>>,
        nest: &mut Nest<'_>,
    ) -> Result<(), String> {
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
            tag @ (ARRAY | OBJECT) => {
                let node = self.u32()?;
                nest.node(node)?;
                match tag {
                    ARRAY => sink.array_of(host, node),
                    _ => sink.object_of(host, node),
                }
            }
            tag @ (ARRAY_WITH | OBJECT_WITH | OBJECT_SOME) => {
                let record = record.ok_or("a node's contents inside the contents of another")?;
                let node = self.u32()?;
                nest.node(node)?;
                record.node(node, tag, self.0);
                self.skip_contents(tag)?;
                match tag {
                    ARRAY_WITH => sink.array_of(host, node),
                    _ => sink.object_of(host, node),
                }
            }
            tag => return Err(format!("a value of unknown tag {tag}")),
        }
        Ok(())
    }

    /// Skips the contents of a node written with `tag`: a count and that many values or fields,
    /// none of them with contents.
    fn skip_contents(&mut self, tag: u8) -> Result<(), String> {
        for _ in 0..self.u32()? {
            if tag != ARRAY_WITH {
                self.utf8()?;
            }
            self.skip(tag == OBJECT_SOME)?;
        }
        Ok(())
    }

    /// Skips a value without contents, which may be [`UNDEFINED`] among `some` fields.
    fn skip(&mut self, some: bool) -> Result<(), String> {
        match self.byte()? {
            NULL | FALSE | TRUE => {}
            UNDEFINED if some => {}
            INTEGER | FLOAT => drop(self.take::<8>()?),
            STRING => drop(self.utf8()?),
            ARRAY | OBJECT => drop(self.u32()?),
            ARRAY_WITH | OBJECT_WITH | OBJECT_SOME => {
                return Err("a node's contents inside the contents of another".into());
            }
            tag => return Err(format!("a value of unknown tag {tag}")),
        }
        Ok(())
    }

    /// Writes the value of field `key` of the fields that follow, nothing when there is none:
    /// `false` when they are only `some` of the object's and do not tell.
    fn field(
        &mut self,
        key: &str,
        some: bool,
        host: &Rc<Host>,
        sink: &mut Sink<'_, '_>,
        nest: &mut Nest<'_>,
    ) -> Result<bool, String> {
        for _ in 0..self.u32()? {
            if self.utf8()? != key.as_bytes() {
                self.skip(some)?;
            } else if some && self.0.first() == Some(&UNDEFINED) {
                return Ok(true);
            } else {
                return self.value(host, sink, None, nest).map(|()| true);
            }
        }
        Ok(!some)
    }

    /// Writes item `index` of the items that follow, nothing past their end.
    fn item(
        &mut self,
        index: usize,
        host: &Rc<Host>,
        sink: &mut Sink<'_, '_>,
        nest: &mut Nest<'_>,
    ) -> Result<(), String> {
        let count = self.u32()? as usize;
        if index < count {
            for _ in 0..index {
                self.skip(false)?;
            }
            self.value(host, sink, None, nest)?;
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
        buffers::with(self.input, |bytes| self.host.write(bytes, false, 1, sink))
            .ok_or_else(|| no_level(self.input))?
            .map(drop)
    }
}

impl Sources<'_> for Host {
    fn read(&self, index: u32, sink: &mut Sink<'_, '_>) -> Result<(), String> {
        let this = self.this.upgrade().ok_or("the input is gone")?;
        let depth = self.inside(index)?;
        // An object of which only some fields were written is read whole from the reader.
        match self.held(index, |mut bytes, tag, depths| {
            (tag != OBJECT_SOME).then(|| {
                let mut nest = Nest { depths, depth };
                bytes.contents(tag == OBJECT_WITH, &this, sink, None, &mut nest)
            })
        }) {
            Some(read) => read,
            None => self.level(READ, index, 0, "", depth, sink).map(drop),
        }
    }

    fn read_field(&self, index: u32, key: &str, sink: &mut Sink<'_, '_>) -> Result<bool, String> {
        let this = self.this.upgrade().ok_or("the input is gone")?;
        let depth = self.inside(index)?;
        match self.held(index, |mut bytes, tag, depths| {
            (tag != ARRAY_WITH).then(|| {
                let mut nest = Nest { depths, depth };
                bytes.field(key, tag == OBJECT_SOME, &this, sink, &mut nest)
            })
        }) {
            Some(Ok(false)) | None => self.level(FIELD, index, 0, key, depth, sink),
            Some(read) => read,
        }
    }

    fn length(&self, index: u32) -> Option<usize> {
        if let Some(count) = self.held(index, |mut bytes, tag, _| {
            (tag == ARRAY_WITH).then(|| bytes.u32())
        }) {
            return count.ok().map(|count| count as usize);
        }
        let length = crate::upcall(|| (self.reader)(self.context, LENGTH, index, 0, 0, 0));
        usize::try_from(length).ok().filter(|_| length != u64::MAX)
    }

    fn read_item(&self, index: u32, item: usize, sink: &mut Sink<'_, '_>) -> Result<bool, String> {
        let this = self.this.upgrade().ok_or("the input is gone")?;
        let depth = self.inside(index)?;
        match self.held(index, |mut bytes, tag, depths| {
            (tag == ARRAY_WITH).then(|| bytes.item(item, &this, sink, &mut Nest { depths, depth }))
        }) {
            Some(read) => read.map(|()| true),
            None => self.level(ITEM, index, item as u64, "", depth, sink),
        }
    }
}
