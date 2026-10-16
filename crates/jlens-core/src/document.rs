//! Input documents converted once, for many evaluations.

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::Value as JsonValue;

use crate::reads::Read;
use crate::value::{Keys, Map, Value};

/// A JSON document in jlens's internal representation.
///
/// Every `evaluate` call converts the parts of its input document that the expression reads
/// from `serde_json::Value`, one level at a time. An expression that reads most of a large
/// document can spend more on that than on the evaluation itself. To evaluate several
/// expressions (a set of rules, say) or the same one several times against one document,
/// convert it once into a `Document` and use
/// [`Expression::evaluate_document`](crate::Expression::evaluate_document).
///
/// ```
/// use jlens_core::{Bindings, Document, Expression, Options};
///
/// let doc = Document::parse(r#"{"items": [{"price": 2}, {"price": 3}]}"#).unwrap();
/// let rules = ["$sum(items.price) > 4", "$count(items) < 10"];
/// for rule in rules {
///     let expr = Expression::compile(rule).unwrap();
///     let result = expr.evaluate_document(&doc, &Bindings::new(), &Options::default());
///     assert_eq!(result.unwrap(), Some(serde_json::json!(true)));
/// }
/// ```
///
/// Evaluations never modify the document. The one exception is a transform (`|...|`) whose
/// expression rebinds `$clone` to a function that does not copy its argument.
///
/// A `Document` is not `Send`. Convert it on the thread that evaluates against it.
pub struct Document {
    value: Value<'static>,
}

impl Document {
    /// Converts a `serde_json` value.
    pub fn new(json: &JsonValue) -> Self {
        Document {
            value: Value::from_json(json),
        }
    }

    /// Parses JSON text straight into the internal representation. This is faster than parsing
    /// into a `serde_json::Value` and converting it.
    pub fn parse(text: &str) -> serde_json::Result<Self> {
        Ok(Document {
            value: parse_value(text, &mut Keys::default())?,
        })
    }

    /// Parses JSON text cut by `read`, skipping the fields it leaves out, as
    /// [`Read::cut`] would cut the whole document. Give it what the expressions to evaluate
    /// read of their input (one expression's [`Reads::input`], or the union of several), and
    /// the document serves each of them as the whole one would.
    ///
    /// The fields skipped are checked as JSON syntax but not converted, so two values that
    /// [`parse`](Document::parse) rejects pass when skipped: a number too large for a float,
    /// and a string with a lone surrogate escape.
    ///
    /// ```
    /// use jlens_core::{Bindings, Document, Expression, Options};
    ///
    /// let text = r#"{"items": [{"price": 2, "notes": "…"}, {"price": 3, "notes": "…"}]}"#;
    /// let rules = ["$sum(items.price) > 4", "$count(items) < 10"];
    /// let exprs: Vec<_> = rules.iter().map(|r| Expression::compile(r).unwrap()).collect();
    /// let reads = exprs.iter().map(|e| e.reads()).reduce(|a, b| a.union(&b)).unwrap();
    /// let doc = Document::parse_cut(text, reads.input().unwrap()).unwrap();
    /// assert_eq!(doc.to_json(), serde_json::json!({"items": [{"price": 2}, {"price": 3}]}));
    /// for expr in &exprs {
    ///     let result = expr.evaluate_document(&doc, &Bindings::new(), &Options::default());
    ///     assert_eq!(result.unwrap(), Some(serde_json::json!(true)));
    /// }
    /// ```
    ///
    /// [`Reads::input`]: crate::reads::Reads::input
    pub fn parse_cut(text: &str, read: &Read) -> serde_json::Result<Self> {
        let mut keys = Keys::default();
        let mut deserializer = serde_json::Deserializer::from_str(text);
        let value = CutSeed(&mut keys, read, PhantomData).deserialize(&mut deserializer)?;
        deserializer.end()?;
        Ok(Document { value })
    }

    /// The document as JSON.
    pub fn to_json(&self) -> JsonValue {
        self.value.to_json().unwrap_or(JsonValue::Null)
    }

    pub(crate) fn value(&self) -> &Value<'static> {
        &self.value
    }
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Document").field(&self.to_json()).finish()
    }
}

impl From<&JsonValue> for Document {
    fn from(json: &JsonValue) -> Self {
        Document::new(json)
    }
}

/// Parses JSON text into a value, sharing the keys of its objects through `keys`.
pub(crate) fn parse_value<'a>(text: &str, keys: &mut Keys) -> serde_json::Result<Value<'a>> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let value = ValueSeed(keys, PhantomData).deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

/// Deserializes a value, sharing the keys of its objects through one table.
struct ValueSeed<'k, 'a>(&'k mut Keys, PhantomData<Value<'a>>);

impl<'de, 'a> DeserializeSeed<'de> for ValueSeed<'_, 'a> {
    type Value = Value<'a>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value<'a>, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de, 'a> Visitor<'de> for ValueSeed<'_, 'a> {
    type Value = Value<'a>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, b: bool) -> Result<Value<'a>, E> {
        Ok(Value::Bool(b))
    }

    fn visit_i64<E>(self, n: i64) -> Result<Value<'a>, E> {
        Ok(Value::Number(n as f64))
    }

    fn visit_u64<E>(self, n: u64) -> Result<Value<'a>, E> {
        Ok(Value::Number(n as f64))
    }

    fn visit_f64<E>(self, n: f64) -> Result<Value<'a>, E> {
        Ok(Value::Number(n))
    }

    fn visit_str<E>(self, s: &str) -> Result<Value<'a>, E> {
        Ok(Value::string(s))
    }

    fn visit_unit<E>(self) -> Result<Value<'a>, E> {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Value<'a>, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value<'a>, A::Error> {
        let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(item) = seq.next_element_seed(ValueSeed(&mut *self.0, PhantomData))? {
            items.push(item);
        }
        Ok(Value::array(items, 0))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value<'a>, A::Error> {
        let mut map =
            Map::with_capacity_and_hasher(access.size_hint().unwrap_or(0), Default::default());
        // A repeated key keeps its first position and takes the last value, as in `JSON.parse`.
        while let Some(key) = access.next_key_seed(KeySeed(self.0))? {
            let value = access.next_value_seed(ValueSeed(&mut *self.0, PhantomData))?;
            map.insert(key, value);
        }
        Ok(Value::object(map))
    }
}

/// Deserializes a value cut by a `Read`, skipping the fields it leaves out.
struct CutSeed<'k, 'r, 'a>(&'k mut Keys, &'r Read, PhantomData<Value<'a>>);

impl<'de, 'a> DeserializeSeed<'de> for CutSeed<'_, '_, 'a> {
    type Value = Value<'a>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value<'a>, D::Error> {
        match self.1 {
            Read::Whole => ValueSeed(self.0, PhantomData).deserialize(deserializer),
            Read::Fields(_) => deserializer.deserialize_any(self),
        }
    }
}

impl<'de, 'a> Visitor<'de> for CutSeed<'_, '_, 'a> {
    type Value = Value<'a>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, b: bool) -> Result<Value<'a>, E> {
        Ok(Value::Bool(b))
    }

    fn visit_i64<E>(self, n: i64) -> Result<Value<'a>, E> {
        Ok(Value::Number(n as f64))
    }

    fn visit_u64<E>(self, n: u64) -> Result<Value<'a>, E> {
        Ok(Value::Number(n as f64))
    }

    fn visit_f64<E>(self, n: f64) -> Result<Value<'a>, E> {
        Ok(Value::Number(n))
    }

    fn visit_str<E>(self, s: &str) -> Result<Value<'a>, E> {
        Ok(Value::string(s))
    }

    fn visit_unit<E>(self) -> Result<Value<'a>, E> {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Value<'a>, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value<'a>, A::Error> {
        // The items of an array are cut by the `Read` of the array.
        let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(item) = seq.next_element_seed(CutSeed(&mut *self.0, self.1, PhantomData))? {
            items.push(item);
        }
        Ok(Value::array(items, 0))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value<'a>, A::Error> {
        let Read::Fields(fields) = self.1 else {
            return ValueSeed(self.0, PhantomData).visit_map(access);
        };
        let mut map = Map::with_capacity_and_hasher(fields.len(), Default::default());
        while let Some(field) = access.next_key_seed(FieldSeed(&mut *self.0, fields))? {
            match field {
                Some((key, read)) => {
                    let value = access.next_value_seed(CutSeed(&mut *self.0, read, PhantomData))?;
                    map.insert(key, value);
                }
                None => {
                    access.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(Value::object(map))
    }
}

/// Deserializes a key into the shared one of the table, with its `Read`, or into `None` when
/// the field is not read.
struct FieldSeed<'k, 'r>(&'k mut Keys, &'r BTreeMap<String, Read>);

impl<'de, 'r> DeserializeSeed<'de> for FieldSeed<'_, 'r> {
    type Value = Option<(Rc<str>, &'r Read)>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_str(self)
    }
}

impl<'de, 'r> Visitor<'de> for FieldSeed<'_, 'r> {
    type Value = Option<(Rc<str>, &'r Read)>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a string")
    }

    fn visit_str<E>(self, s: &str) -> Result<Self::Value, E> {
        Ok(self.1.get(s).map(|read| (self.0.get(s), read)))
    }
}

/// Deserializes a key into the shared one of the table.
struct KeySeed<'k>(&'k mut Keys);

impl<'de> DeserializeSeed<'de> for KeySeed<'_> {
    type Value = Rc<str>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Rc<str>, D::Error> {
        deserializer.deserialize_str(self)
    }
}

impl<'de> Visitor<'de> for KeySeed<'_> {
    type Value = Rc<str>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a string")
    }

    fn visit_str<E>(self, s: &str) -> Result<Rc<str>, E> {
        Ok(self.0.get(s))
    }
}
