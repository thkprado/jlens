//! A host whose own values are `serde_json` values, read by the C API through a reader
//! (`jlens_capi::source`), as the Java package's are.
//!
//! The reader cannot read the key of a field at the address it is given without `unsafe`, which
//! the crate denies. It finds the key by its length among the keys it knows, those of the input
//! and those given with [`Host::know`], and declines when two of them have that length.

// Each test binary uses part of it.
#![allow(dead_code)]

use std::cell::RefCell;

use jlens_capi::buffers;
use jlens_capi::source::{
    ARRAY_WITH, DECLINED, FAILED, FIELD, FIELDS, ITEM, LENGTH, OBJECT_SOME, OBJECT_WITH, READ,
    Reader, VALUES,
};
use jlens_capi::values::{ARRAY, FALSE, FLOAT, INTEGER, NULL, OBJECT, STRING, TRUE, UNDEFINED};
use serde_json::Value;

/// How the reader answers, chosen by the evaluation's `context`.
pub const PLAIN: u64 = 0;
/// Declines every field and item: levels are read whole.
pub const WHOLE: u64 = 1;
/// Answers every call with the bytes of [`Host::garbage`].
pub const GARBAGE: u64 = 2;
/// Writes the arrays and objects of at most [`SMALL`] entries with their contents, when they
/// are values of the level itself, as the Java package does.
pub const WITH: u64 = 3;
/// The most entries of an array or object written with its contents, in mode [`WITH`]: few,
/// so that small inputs have both kinds.
const SMALL: usize = 2;
/// As [`WITH`], except that objects are written with some of their fields, whatever their
/// size: every other one, and [`ABSENT`] as a field they lack. The C API must give the same
/// answers whichever fields are written, as the Java package writes those the expression reads.
pub const SOME: u64 = 4;
/// The key of a field written as lacking in mode [`SOME`], unless the object has it.
const ABSENT: &str = "absent";

/// The state of this thread's host during an evaluation.
#[derive(Default)]
pub struct Host {
    /// The arrays and objects, by their numbers.
    pub nodes: Vec<Value>,
    /// The calls of the reader, as `(op, node)`.
    pub calls: Vec<(u32, u32)>,
    /// The bytes every call returns in [`GARBAGE`] mode.
    pub garbage: Vec<u8>,
    /// The keys the reader can tell by their length.
    known: Vec<String>,
    /// The keys given with [`Host::know`], kept from one evaluation to the next.
    named: Vec<String>,
    /// The level returned last, freed at the next call.
    level: u64,
    /// Whether levels write small arrays and objects with their contents ([`WITH`]).
    with: bool,
    /// Whether levels write objects with some of their fields ([`SOME`]).
    some: bool,
}

thread_local! {
    pub static HOST: RefCell<Host> = RefCell::default();
}

/// The reader, to pass to `jlens_evaluate_source`.
pub const READER: Reader = reader;

extern "C" fn reader(
    context: u64,
    op: u32,
    node: u32,
    index: u64,
    _key: usize,
    key_len: usize,
) -> u64 {
    HOST.with_borrow_mut(|host| {
        host.calls.push((op, node));
        if host.level != 0 {
            buffers::free(host.level);
            host.level = 0;
        }
        if op == LENGTH {
            return match (context, host.nodes.get(node as usize)) {
                (PLAIN, Some(Value::Array(items))) => items.len() as u64,
                _ => u64::MAX,
            };
        }
        let level = match context {
            GARBAGE => host.garbage.clone(),
            _ => host.answer(context, op, node, index, key_len),
        };
        host.level = buffers::insert(level);
        host.level
    })
}

impl Host {
    /// Resets the host for an evaluation against `input`, and returns the buffer of the input's
    /// level, which the caller frees.
    pub fn start(input: &Value) -> u64 {
        Host::start_in(input, PLAIN)
    }

    /// Resets the host for an evaluation against `input` in mode `context`, as [`Host::start`]
    /// does.
    pub fn start_in(input: &Value, context: u64) -> u64 {
        HOST.with_borrow_mut(|host| {
            *host = Host {
                garbage: std::mem::take(&mut host.garbage),
                known: host.named.clone(),
                named: std::mem::take(&mut host.named),
                with: context == WITH || context == SOME,
                some: context == SOME,
                ..Host::default()
            };
            keys(input, &mut host.known);
            let mut level = Level::new(VALUES, 1);
            host.top(&mut level, input);
            buffers::insert(level.finish())
        })
    }

    /// Sets `keys`, which the expressions name, as keys the reader knows.
    pub fn know(keys: &[&str]) {
        HOST.with_borrow_mut(|host| host.named = keys.iter().map(|&key| key.to_owned()).collect())
    }

    /// Frees the last level.
    pub fn finish() {
        HOST.with_borrow_mut(|host| {
            if host.level != 0 {
                buffers::free(host.level);
                host.level = 0;
            }
        })
    }

    fn answer(&mut self, context: u64, op: u32, node: u32, index: u64, key_len: usize) -> Vec<u8> {
        let Some(value) = self.nodes.get(node as usize).cloned() else {
            return failed(&format!("no node {node}"));
        };
        match (op, &value) {
            (READ, Value::Array(items)) => {
                let mut level = Level::new(VALUES, items.len());
                items.iter().for_each(|item| self.top(&mut level, item));
                level.finish()
            }
            (READ, Value::Object(fields)) => {
                let mut level = Level::new(FIELDS, fields.len());
                for (key, value) in fields {
                    level.text(key);
                    self.top(&mut level, value);
                }
                level.finish()
            }
            (FIELD | ITEM, _) if context == WHOLE => Level::new(DECLINED, 0).finish(),
            (FIELD, Value::Object(fields)) => {
                let mut known: Vec<&String> = self
                    .known
                    .iter()
                    .filter(|key| key.len() == key_len)
                    .collect();
                known.sort();
                known.dedup();
                let [key] = known[..] else {
                    return Level::new(DECLINED, 0).finish();
                };
                match fields.get(key) {
                    Some(value) => {
                        let mut level = Level::new(VALUES, 1);
                        self.top(&mut level, value);
                        level.finish()
                    }
                    None => Level::new(VALUES, 0).finish(),
                }
            }
            (ITEM, Value::Array(items)) => match items.get(index as usize) {
                Some(item) => {
                    let mut level = Level::new(VALUES, 1);
                    self.top(&mut level, item);
                    level.finish()
                }
                None => Level::new(VALUES, 0).finish(),
            },
            _ => failed(&format!("op {op} on {value}")),
        }
    }

    /// A value of the level itself: a small array or object with its contents in mode [`WITH`],
    /// an object with some of its fields in mode [`SOME`].
    fn top(&mut self, level: &mut Level, value: &Value) {
        let node = self.nodes.len();
        match value {
            Value::Object(fields) if self.some => {
                let absent = !fields.contains_key(ABSENT);
                let some = fields.iter().step_by(2);
                level.0.push(OBJECT_SOME);
                level.u32(node);
                level.u32(some.len() + absent as usize);
                self.nodes.push(value.clone());
                for (key, value) in some {
                    level.text(key);
                    self.value(level, value);
                }
                if absent {
                    level.text(ABSENT);
                    level.0.push(UNDEFINED);
                }
            }
            Value::Array(items) if self.with && items.len() <= SMALL => {
                level.0.push(ARRAY_WITH);
                level.u32(node);
                level.u32(items.len());
                self.nodes.push(value.clone());
                items.iter().for_each(|item| self.value(level, item));
            }
            Value::Object(fields) if self.with && fields.len() <= SMALL => {
                level.0.push(OBJECT_WITH);
                level.u32(node);
                level.u32(fields.len());
                self.nodes.push(value.clone());
                for (key, value) in fields {
                    level.text(key);
                    self.value(level, value);
                }
            }
            _ => self.value(level, value),
        }
    }

    fn value(&mut self, level: &mut Level, value: &Value) {
        match value {
            Value::Null => level.0.push(NULL),
            Value::Bool(false) => level.0.push(FALSE),
            Value::Bool(true) => level.0.push(TRUE),
            Value::Number(n) => match n.as_i64() {
                Some(n) => {
                    level.0.push(INTEGER);
                    level.0.extend_from_slice(&n.to_le_bytes());
                }
                None => {
                    level.0.push(FLOAT);
                    level
                        .0
                        .extend_from_slice(&n.as_f64().unwrap().to_le_bytes());
                }
            },
            Value::String(s) => {
                level.0.push(STRING);
                level.text(s);
            }
            Value::Array(_) | Value::Object(_) => {
                level.0.push(if value.is_array() { ARRAY } else { OBJECT });
                level.u32(self.nodes.len());
                self.nodes.push(value.clone());
            }
        }
    }
}

/// Adds the keys of `value`'s objects to `known`.
fn keys(value: &Value, known: &mut Vec<String>) {
    match value {
        Value::Array(items) => items.iter().for_each(|item| keys(item, known)),
        Value::Object(fields) => fields.iter().for_each(|(key, value)| {
            known.push(key.clone());
            keys(value, known);
        }),
        _ => {}
    }
}

/// A level being written.
struct Level(Vec<u8>);

impl Level {
    fn new(kind: u8, count: usize) -> Level {
        let mut level = Level(vec![0; 4]);
        level.0.push(kind);
        if kind == VALUES || kind == FIELDS {
            level.u32(count);
        }
        level
    }

    fn u32(&mut self, n: usize) {
        self.0.extend_from_slice(&(n as u32).to_le_bytes());
    }

    fn text(&mut self, text: &str) {
        self.u32(text.len());
        self.0.extend_from_slice(text.as_bytes());
    }

    fn finish(mut self) -> Vec<u8> {
        let len = (self.0.len() - 4) as u32;
        self.0[..4].copy_from_slice(&len.to_le_bytes());
        self.0
    }
}

/// A level of the reader's failure.
pub fn failed(message: &str) -> Vec<u8> {
    let mut level = Level::new(FAILED, 0);
    level.0.extend_from_slice(message.as_bytes());
    level.finish()
}
