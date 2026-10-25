//! A decoder of the values replies (`jlens_capi::values`), as the host's would be.

// Each test binary uses part of it.
#![allow(dead_code)]

use jlens_capi::values::{ARRAY, FALSE, FLOAT, INTEGER, NULL, OBJECT, STRING, TRUE};
use serde_json::{Map, Value};

/// The value encoded in `bytes`, after a reply's status byte. Panics on bytes not in the
/// encoding, or left over.
pub fn decode(bytes: &[u8]) -> Value {
    let mut decoder = Decoder {
        bytes,
        at: 0,
        keys: Vec::new(),
    };
    let value = decoder.value();
    assert_eq!(decoder.at, bytes.len(), "bytes left over");
    value
}

/// `value` with every number as an `f64`, so that a decoded `INTEGER` and JSON text's
/// `u64` compare by value.
pub fn as_f64(value: Value) -> Value {
    match value {
        Value::Number(n) => n
            .as_f64()
            .and_then(serde_json::Number::from_f64)
            .map_or(Value::Null, Value::Number),
        Value::Array(items) => Value::Array(items.into_iter().map(as_f64).collect()),
        Value::Object(fields) => {
            Value::Object(fields.into_iter().map(|(k, v)| (k, as_f64(v))).collect())
        }
        other => other,
    }
}

struct Decoder<'b> {
    bytes: &'b [u8],
    at: usize,
    keys: Vec<String>,
}

impl Decoder<'_> {
    fn take(&mut self, n: usize) -> &[u8] {
        let taken = &self.bytes[self.at..self.at + n];
        self.at += n;
        taken
    }

    fn u32(&mut self) -> usize {
        u32::from_le_bytes(self.take(4).try_into().unwrap()) as usize
    }

    fn text(&mut self) -> String {
        let len = self.u32();
        String::from_utf8(self.take(len).to_vec()).expect("UTF-8")
    }

    fn value(&mut self) -> Value {
        let tag = self.take(1)[0];
        match tag {
            NULL => Value::Null,
            FALSE => Value::Bool(false),
            TRUE => Value::Bool(true),
            INTEGER => i64::from_le_bytes(self.take(8).try_into().unwrap()).into(),
            FLOAT => {
                let value = f64::from_le_bytes(self.take(8).try_into().unwrap());
                assert!(value.is_finite());
                assert!(
                    value.fract() != 0.0
                        || !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0)
                            .contains(&value),
                    "{value} is an INTEGER"
                );
                value.into()
            }
            STRING => self.text().into(),
            ARRAY => {
                let count = self.u32();
                Value::Array((0..count).map(|_| self.value()).collect())
            }
            OBJECT => {
                let count = self.u32();
                let mut fields = Map::new();
                for _ in 0..count {
                    let value = self.value();
                    let index = self.u32();
                    let key = match index.cmp(&self.keys.len()) {
                        std::cmp::Ordering::Less => self.keys[index].clone(),
                        std::cmp::Ordering::Equal => {
                            let key = self.text();
                            self.keys.push(key.clone());
                            key
                        }
                        std::cmp::Ordering::Greater => panic!("key {index} out of order"),
                    };
                    assert!(fields.insert(key, value).is_none(), "a key twice");
                }
                Value::Object(fields)
            }
            tag => panic!("unknown tag {tag}"),
        }
    }
}
