//! Input documents converted once, for many evaluations.

use std::fmt;

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value as JsonValue;

use crate::value::{Map, Value};

/// A JSON document in jlens's internal representation.
///
/// Every `evaluate` call converts its input document from `serde_json::Value`. For a large
/// document this can cost more than the evaluation itself. To evaluate several expressions (a
/// set of rules, say) or the same one several times against one document, convert it once
/// into a `Document` and use [`Expression::evaluate_document`](crate::Expression::evaluate_document).
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
    value: Value,
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
        let mut deserializer = serde_json::Deserializer::from_str(text);
        let value = Value::deserialize(&mut deserializer)?;
        deserializer.end()?;
        Ok(Document { value })
    }

    /// The document as JSON.
    pub fn to_json(&self) -> JsonValue {
        self.value.to_json().unwrap_or(JsonValue::Null)
    }

    pub(crate) fn value(&self) -> &Value {
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

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, b: bool) -> Result<Value, E> {
        Ok(Value::Bool(b))
    }

    fn visit_i64<E>(self, n: i64) -> Result<Value, E> {
        Ok(Value::Number(n as f64))
    }

    fn visit_u64<E>(self, n: u64) -> Result<Value, E> {
        Ok(Value::Number(n as f64))
    }

    fn visit_f64<E>(self, n: f64) -> Result<Value, E> {
        Ok(Value::Number(n))
    }

    fn visit_str<E>(self, s: &str) -> Result<Value, E> {
        Ok(Value::string(s))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(item) = seq.next_element::<Value>()? {
            items.push(item);
        }
        Ok(Value::array(items, 0))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        let mut map =
            Map::with_capacity_and_hasher(access.size_hint().unwrap_or(0), Default::default());
        // A repeated key keeps its first position and takes the last value, as in `JSON.parse`.
        while let Some((key, value)) = access.next_entry::<String, Value>()? {
            map.insert(key, value);
        }
        Ok(Value::object(map))
    }
}
