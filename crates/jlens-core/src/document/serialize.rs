//! A `serde::Serializer` that writes Rust values straight into the internal representation,
//! for [`Document::from_serialize`](super::Document::from_serialize).
//!
//! It gives the value that `serde_json::to_value` followed by `Document::new` gives: the same
//! keys in the same order, the same numbers and the same errors. The paths that no common type
//! takes (128-bit integers, keys that are not strings, `serde_json`'s private number and raw
//! value tokens) go through `serde_json` itself, so that they follow it whatever its features.

use std::rc::Rc;

use serde::Serialize;
use serde::ser::{self, Error as _, Impossible};
use serde_json::Error;
use serde_json::Value as JsonValue;

use crate::value::{Keys, MAX_NESTING, Map, Value, nest};

type Result<T> = std::result::Result<T, Error>;

/// The token `serde_json` serializes a number as, with its `arbitrary_precision` feature.
const NUMBER_TOKEN: &str = "$serde_json::private::Number";
/// The token `serde_json` serializes a `RawValue` as, with its `raw_value` feature.
const RAW_VALUE_TOKEN: &str = "$serde_json::private::RawValue";

/// Serializes `value` at nesting level `depth`, sharing the keys of its objects through `keys`.
pub(super) fn to_value<T: ?Sized + Serialize>(
    value: &T,
    keys: &mut Keys,
    depth: usize,
) -> Result<Value<'static>> {
    nest(depth, || value.serialize(Serializer { keys, depth }))
}

/// A value that only `serde_json` converts, then converted as `Document::new` would.
fn through_json<T: ?Sized + Serialize>(value: &T) -> Result<Value<'static>> {
    serde_json::to_value(value).map(|json| Value::from_json(&json))
}

pub(super) struct Serializer<'k> {
    keys: &'k mut Keys,
    depth: usize,
}

impl<'k> Serializer<'k> {
    fn nested(depth: usize) -> Result<usize> {
        if depth >= MAX_NESTING {
            return Err(Error::custom(format!(
                "a value is nested more than {MAX_NESTING} levels deep"
            )));
        }
        Ok(depth + 1)
    }

    fn seq(self, len: usize) -> Result<Seq<'k>> {
        Ok(Seq {
            depth: Self::nested(self.depth)?,
            keys: self.keys,
            items: Vec::with_capacity(len),
            variant: None,
        })
    }

    fn map(self, len: usize, variant: Option<Rc<str>>) -> Result<Object<'k>> {
        Ok(Object {
            depth: Self::nested(self.depth)?,
            keys: self.keys,
            map: Map::with_capacity_and_hasher(len, Default::default()),
            next_key: None,
            variant,
        })
    }

    /// `{variant: value}`, as `serde_json` writes an enum's newtype, tuple and struct variants.
    fn variant(keys: &mut Keys, variant: &str, value: Value<'static>) -> Value<'static> {
        let mut map = Map::with_capacity_and_hasher(1, Default::default());
        map.insert(keys.get(variant), value);
        Value::object(map)
    }
}

impl<'k> ser::Serializer for Serializer<'k> {
    type Ok = Value<'static>;
    type Error = Error;
    type SerializeSeq = Seq<'k>;
    type SerializeTuple = Seq<'k>;
    type SerializeTupleStruct = Seq<'k>;
    type SerializeTupleVariant = Seq<'k>;
    type SerializeMap = Object<'k>;
    type SerializeStruct = Struct<'k>;
    type SerializeStructVariant = Object<'k>;

    fn serialize_bool(self, v: bool) -> Result<Value<'static>> {
        Ok(Value::Bool(v))
    }

    fn serialize_i8(self, v: i8) -> Result<Value<'static>> {
        Ok(Value::Number(v.into()))
    }

    fn serialize_i16(self, v: i16) -> Result<Value<'static>> {
        Ok(Value::Number(v.into()))
    }

    fn serialize_i32(self, v: i32) -> Result<Value<'static>> {
        Ok(Value::Number(v.into()))
    }

    fn serialize_i64(self, v: i64) -> Result<Value<'static>> {
        Ok(Value::Number(v as f64))
    }

    fn serialize_i128(self, v: i128) -> Result<Value<'static>> {
        through_json(&v)
    }

    fn serialize_u8(self, v: u8) -> Result<Value<'static>> {
        Ok(Value::Number(v.into()))
    }

    fn serialize_u16(self, v: u16) -> Result<Value<'static>> {
        Ok(Value::Number(v.into()))
    }

    fn serialize_u32(self, v: u32) -> Result<Value<'static>> {
        Ok(Value::Number(v.into()))
    }

    fn serialize_u64(self, v: u64) -> Result<Value<'static>> {
        Ok(Value::Number(v as f64))
    }

    fn serialize_u128(self, v: u128) -> Result<Value<'static>> {
        through_json(&v)
    }

    fn serialize_f32(self, v: f32) -> Result<Value<'static>> {
        self.serialize_f64(v.into())
    }

    /// JSON has no NaN or infinity: `serde_json` writes `null` for them.
    fn serialize_f64(self, v: f64) -> Result<Value<'static>> {
        Ok(if v.is_finite() {
            Value::Number(v)
        } else {
            Value::Null
        })
    }

    fn serialize_char(self, v: char) -> Result<Value<'static>> {
        Ok(Value::string(v.encode_utf8(&mut [0; 4]) as &str))
    }

    fn serialize_str(self, v: &str) -> Result<Value<'static>> {
        Ok(Value::string(v))
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Value<'static>> {
        Ok(Value::array(
            v.iter().map(|&b| Value::Number(b.into())).collect(),
            0,
        ))
    }

    fn serialize_none(self) -> Result<Value<'static>> {
        Ok(Value::Null)
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Value<'static>> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Value<'static>> {
        Ok(Value::Null)
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<Value<'static>> {
        Ok(Value::Null)
    }

    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<Value<'static>> {
        Ok(Value::string(variant))
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<Value<'static>> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Value<'static>> {
        let depth = Self::nested(self.depth)?;
        let value = to_value(value, self.keys, depth)?;
        Ok(Self::variant(self.keys, variant, value))
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Seq<'k>> {
        self.seq(len.unwrap_or(0))
    }

    fn serialize_tuple(self, len: usize) -> Result<Seq<'k>> {
        self.seq(len)
    }

    fn serialize_tuple_struct(self, _: &'static str, len: usize) -> Result<Seq<'k>> {
        self.seq(len)
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Seq<'k>> {
        let variant = self.keys.get(variant);
        let mut seq = self.seq(len)?;
        seq.variant = Some(variant);
        Ok(seq)
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Object<'k>> {
        self.map(len.unwrap_or(0), None)
    }

    fn serialize_struct(self, name: &'static str, len: usize) -> Result<Struct<'k>> {
        match name {
            NUMBER_TOKEN | RAW_VALUE_TOKEN => Ok(Struct::Token(name, None)),
            _ => self.map(len, None).map(Struct::Object),
        }
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Object<'k>> {
        let variant = self.keys.get(variant);
        self.map(len, Some(variant))
    }

    fn collect_str<T: ?Sized + std::fmt::Display>(self, value: &T) -> Result<Value<'static>> {
        Ok(Value::string(value.to_string()))
    }
}

/// An array being serialized, or the array of a tuple variant.
pub(super) struct Seq<'k> {
    keys: &'k mut Keys,
    depth: usize,
    items: Vec<Value<'static>>,
    variant: Option<Rc<str>>,
}

impl Seq<'_> {
    fn push<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        self.items.push(to_value(value, self.keys, self.depth)?);
        Ok(())
    }

    fn finish(self) -> Value<'static> {
        let array = Value::array(self.items, 0);
        match self.variant {
            None => array,
            Some(variant) => {
                let mut map = Map::with_capacity_and_hasher(1, Default::default());
                map.insert(variant, array);
                Value::object(map)
            }
        }
    }
}

impl ser::SerializeSeq for Seq<'_> {
    type Ok = Value<'static>;
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        self.push(value)
    }

    fn end(self) -> Result<Value<'static>> {
        Ok(self.finish())
    }
}

impl ser::SerializeTuple for Seq<'_> {
    type Ok = Value<'static>;
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        self.push(value)
    }

    fn end(self) -> Result<Value<'static>> {
        Ok(self.finish())
    }
}

impl ser::SerializeTupleStruct for Seq<'_> {
    type Ok = Value<'static>;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        self.push(value)
    }

    fn end(self) -> Result<Value<'static>> {
        Ok(self.finish())
    }
}

impl ser::SerializeTupleVariant for Seq<'_> {
    type Ok = Value<'static>;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        self.push(value)
    }

    fn end(self) -> Result<Value<'static>> {
        Ok(self.finish())
    }
}

/// An object being serialized (a map or a struct), or the object of a struct variant.
pub(super) struct Object<'k> {
    keys: &'k mut Keys,
    depth: usize,
    map: Map<'static>,
    next_key: Option<Rc<str>>,
    variant: Option<Rc<str>>,
}

impl Object<'_> {
    /// A repeated key keeps its first position and takes the last value, as in a
    /// `serde_json::Map` with `preserve_order`.
    fn insert<T: ?Sized + Serialize>(&mut self, key: Rc<str>, value: &T) -> Result<()> {
        let value = to_value(value, self.keys, self.depth)?;
        self.map.insert(key, value);
        Ok(())
    }

    fn finish(self) -> Value<'static> {
        let object = Value::object(self.map);
        match self.variant {
            None => object,
            Some(variant) => {
                let mut map = Map::with_capacity_and_hasher(1, Default::default());
                map.insert(variant, object);
                Value::object(map)
            }
        }
    }
}

impl ser::SerializeMap for Object<'_> {
    type Ok = Value<'static>;
    type Error = Error;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<()> {
        self.next_key = Some(key.serialize(KeySerializer(self.keys))?);
        Ok(())
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        let key = self
            .next_key
            .take()
            .ok_or_else(|| Error::custom("serialize_value called before serialize_key"))?;
        self.insert(key, value)
    }

    fn end(self) -> Result<Value<'static>> {
        Ok(self.finish())
    }
}

impl ser::SerializeStructVariant for Object<'_> {
    type Ok = Value<'static>;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<()> {
        let key = self.keys.get(key);
        self.insert(key, value)
    }

    fn end(self) -> Result<Value<'static>> {
        Ok(self.finish())
    }
}

/// A struct being serialized, or one of `serde_json`'s private tokens, which only `serde_json`
/// reads.
pub(super) enum Struct<'k> {
    Object(Object<'k>),
    Token(&'static str, Option<Value<'static>>),
}

impl ser::SerializeStruct for Struct<'_> {
    type Ok = Value<'static>;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<()> {
        match self {
            Struct::Object(object) => {
                let key = object.keys.get(key);
                object.insert(key, value)
            }
            Struct::Token(token, out) => {
                *out = Some(through_json(&Token(token, key, value))?);
                Ok(())
            }
        }
    }

    fn end(self) -> Result<Value<'static>> {
        match self {
            Struct::Object(object) => Ok(object.finish()),
            Struct::Token(_, out) => Ok(out.unwrap_or(Value::Null)),
        }
    }
}

/// A `serde_json` token written again as the one-field struct it came as, for `serde_json`.
struct Token<'t, T: ?Sized>(&'static str, &'static str, &'t T);

impl<T: ?Sized + Serialize> Serialize for Token<'_, T> {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use ser::SerializeStruct;
        let mut s = serializer.serialize_struct(self.0, 1)?;
        s.serialize_field(self.1, self.2)?;
        s.end()
    }
}

/// Serializes the key of a map: a string is shared through the table of keys; anything else
/// becomes the key `serde_json` would make of it, or its error.
struct KeySerializer<'k>(&'k mut Keys);

/// The key `serde_json` makes of `key`, with its error when it makes none.
fn json_key<T: ?Sized + Serialize>(key: &T) -> Result<String> {
    struct One<'t, T: ?Sized>(&'t T);
    impl<T: ?Sized + Serialize> Serialize for One<'_, T> {
        fn serialize<S: ser::Serializer>(
            &self,
            serializer: S,
        ) -> std::result::Result<S::Ok, S::Error> {
            use ser::SerializeMap;
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(self.0, &())?;
            map.end()
        }
    }
    match serde_json::to_value(One(key))? {
        JsonValue::Object(map) => Ok(map.into_iter().next().map(|(k, _)| k).unwrap_or_default()),
        _ => Err(Error::custom("a map key did not serialize as a key")),
    }
}

macro_rules! through_json_key {
    ($($method:ident: $ty:ty),*) => {$(
        fn $method(self, v: $ty) -> Result<Rc<str>> {
            Ok(self.0.get(&json_key(&v)?))
        }
    )*};
}

impl ser::Serializer for KeySerializer<'_> {
    type Ok = Rc<str>;
    type Error = Error;
    type SerializeSeq = Impossible<Rc<str>, Error>;
    type SerializeTuple = Impossible<Rc<str>, Error>;
    type SerializeTupleStruct = Impossible<Rc<str>, Error>;
    type SerializeTupleVariant = Impossible<Rc<str>, Error>;
    type SerializeMap = Impossible<Rc<str>, Error>;
    type SerializeStruct = Impossible<Rc<str>, Error>;
    type SerializeStructVariant = Impossible<Rc<str>, Error>;

    fn serialize_str(self, v: &str) -> Result<Rc<str>> {
        Ok(self.0.get(v))
    }

    fn serialize_unit_variant(self, _: &'static str, _: u32, v: &'static str) -> Result<Rc<str>> {
        Ok(self.0.get(v))
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<Rc<str>> {
        value.serialize(self)
    }

    fn collect_str<T: ?Sized + std::fmt::Display>(self, value: &T) -> Result<Rc<str>> {
        Ok(self.0.get(&value.to_string()))
    }

    through_json_key!(
        serialize_bool: bool, serialize_char: char,
        serialize_i8: i8, serialize_i16: i16, serialize_i32: i32, serialize_i64: i64,
        serialize_i128: i128,
        serialize_u8: u8, serialize_u16: u16, serialize_u32: u32, serialize_u64: u64,
        serialize_u128: u128,
        serialize_f32: f32, serialize_f64: f64
    );

    fn serialize_bytes(self, _: &[u8]) -> Result<Rc<str>> {
        Err(not_a_key())
    }

    fn serialize_none(self) -> Result<Rc<str>> {
        Err(not_a_key())
    }

    fn serialize_some<T: ?Sized + Serialize>(self, _: &T) -> Result<Rc<str>> {
        Err(not_a_key())
    }

    fn serialize_unit(self) -> Result<Rc<str>> {
        Err(not_a_key())
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<Rc<str>> {
        Err(not_a_key())
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<Rc<str>> {
        Err(not_a_key())
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq> {
        Err(not_a_key())
    }

    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple> {
        Err(not_a_key())
    }

    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct> {
        Err(not_a_key())
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant> {
        Err(not_a_key())
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap> {
        Err(not_a_key())
    }

    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct> {
        Err(not_a_key())
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant> {
        Err(not_a_key())
    }
}

/// `serde_json`'s error for a key that is not a string, number, boolean or character.
fn not_a_key() -> Error {
    match json_key(&()) {
        Err(e) => e,
        Ok(_) => Error::custom("key must be a string"),
    }
}
