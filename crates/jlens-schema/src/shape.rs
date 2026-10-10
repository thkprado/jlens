//! The structural metadata model and its inference from sample documents.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Maximum number of distinct values tracked as enumeration candidates.
const MAX_ENUM: usize = 10;
/// Maximum number of examples kept per shape.
const MAX_EXAMPLES: usize = 3;

/// JSON types, as in JSON Schema (`integer` is a number without a fractional part).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JsonType {
    /// `null`
    Null,
    /// `true` / `false`
    Boolean,
    /// A number without a fractional part.
    Integer,
    /// Any number.
    Number,
    /// A string.
    String,
    /// An array.
    Array,
    /// An object.
    Object,
}

impl JsonType {
    /// The JSON Schema name of the type.
    pub fn name(self) -> &'static str {
        match self {
            JsonType::Null => "null",
            JsonType::Boolean => "boolean",
            JsonType::Integer => "integer",
            JsonType::Number => "number",
            JsonType::String => "string",
            JsonType::Array => "array",
            JsonType::Object => "object",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "null" => JsonType::Null,
            "boolean" => JsonType::Boolean,
            "integer" => JsonType::Integer,
            "number" => JsonType::Number,
            "string" => JsonType::String,
            "array" => JsonType::Array,
            "object" => JsonType::Object,
            _ => return None,
        })
    }
}

/// The structure of the values found at one place in the documents.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shape {
    /// The types seen. `Integer` is dropped as soon as a non-integral number is seen.
    pub types: Vec<JsonType>,
    /// Fields of the objects, in first-seen order.
    #[serde(skip_serializing_if = "IndexMap::is_empty", default)]
    pub fields: IndexMap<String, Field>,
    /// The shape of array items.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub items: Option<Box<Shape>>,
    /// Smallest number seen.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub minimum: Option<f64>,
    /// Largest number seen.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub maximum: Option<f64>,
    /// A string format shared by every string seen: `date-time`, `date`, `time`, `email`,
    /// `uuid` or `uri`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub format: Option<String>,
    /// The allowed values, when known (from an override, a JSON Schema, or inferred from a small
    /// set of repeated values).
    #[serde(rename = "enum", skip_serializing_if = "Option::is_none", default)]
    pub enumeration: Option<Vec<JsonValue>>,
    /// A few example values.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub examples: Vec<JsonValue>,
    /// What the value means.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    /// A short display name.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub label: Option<String>,
    /// Hidden values are not suggested by completion.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub hidden: bool,

    // Inference state.
    #[serde(skip)]
    pub(crate) count: u64,
    #[serde(skip)]
    pub(crate) objects: u64,
    #[serde(skip)]
    pub(crate) strings: u64,
    /// Distinct scalar values seen, `None` once there are too many.
    #[serde(skip)]
    pub(crate) distinct: Option<Vec<JsonValue>>,
    /// The common string format so far: `None` before any string, `Some(None)` when they differ.
    #[serde(skip)]
    pub(crate) common_format: Option<Option<&'static str>>,
}

/// A field of an object shape.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    /// The shape of the field's values.
    pub shape: Shape,
    /// Present in every object seen.
    pub required: bool,
    /// Fraction of the objects that have the field (1.0 when required).
    pub presence: f64,
}

/// Structural metadata about a family of JSON documents.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schema {
    /// The shape of the documents themselves.
    pub root: Shape,
    /// Number of sample documents the schema was inferred from (0 when imported).
    pub samples: u64,
}

impl Schema {
    /// An empty schema, to be filled with [`Schema::observe`].
    pub fn new() -> Self {
        Schema::default()
    }

    /// Infers a schema from sample documents.
    ///
    /// ```
    /// use jlens_schema::Schema;
    /// use serde_json::json;
    ///
    /// let schema = Schema::infer(&[
    ///     json!({"id": 1, "tags": ["a"], "owner": {"name": "Ann"}}),
    ///     json!({"id": 2, "tags": [], "owner": null}),
    /// ]);
    /// let id = &schema.root.fields["id"];
    /// assert!(id.required);
    /// assert_eq!(id.shape.type_summary(), "integer");
    /// assert_eq!(schema.root.fields["owner"].shape.type_summary(), "object | null");
    /// ```
    pub fn infer(samples: &[JsonValue]) -> Self {
        let mut schema = Schema::new();
        for sample in samples {
            schema.observe(sample);
        }
        schema
    }

    /// Adds one more sample document.
    pub fn observe(&mut self, sample: &JsonValue) {
        self.samples += 1;
        self.root.observe(sample);
    }

    /// The shape at a path such as `items[].type` (`[]` steps into array items; an empty path or
    /// `$` is the root).
    pub fn shape_at(&self, path: &str) -> Option<&Shape> {
        let mut shape = &self.root;
        for (name, arrays) in parse_path(path)? {
            if let Some(name) = name {
                shape = &shape.fields.get(&name)?.shape;
            }
            for _ in 0..arrays {
                shape = shape.items.as_deref()?;
            }
        }
        Some(shape)
    }

    fn shape_at_mut(&mut self, path: &str) -> Option<&mut Shape> {
        let mut shape = &mut self.root;
        for (name, arrays) in parse_path(path)? {
            if let Some(name) = name {
                shape = &mut shape.fields.get_mut(&name)?.shape;
            }
            for _ in 0..arrays {
                shape = shape.items.as_deref_mut()?;
            }
        }
        Some(shape)
    }

    /// Applies overrides: a map from paths (see [`Schema::shape_at`]) to settings. Unknown
    /// paths are created, so overrides can also declare fields the samples did not contain.
    /// Returns the paths that could not be applied.
    ///
    /// ```
    /// use jlens_schema::{Override, Schema};
    /// use serde_json::json;
    ///
    /// let mut schema = Schema::infer(&[json!({"items": [{"type": "a"}]})]);
    /// let overrides: std::collections::BTreeMap<String, Override> = serde_json::from_value(json!({
    ///     "items[].type": {"description": "Product category", "enum": ["a", "b", "c"]},
    ///     "items[].internalCode": {"hidden": true}
    /// })).unwrap();
    /// assert!(schema.apply_overrides(&overrides).is_empty());
    /// let ty = schema.shape_at("items[].type").unwrap();
    /// assert_eq!(ty.description.as_deref(), Some("Product category"));
    /// ```
    pub fn apply_overrides(&mut self, overrides: &BTreeMap<String, Override>) -> Vec<String> {
        let mut failed = Vec::new();
        for (path, o) in overrides {
            if self.shape_at(path).is_none() && !self.create_path(path) {
                failed.push(path.clone());
                continue;
            }
            let Some(shape) = self.shape_at_mut(path) else {
                failed.push(path.clone());
                continue;
            };
            if let Some(d) = &o.description {
                shape.description = Some(d.clone());
            }
            if let Some(l) = &o.label {
                shape.label = Some(l.clone());
            }
            if let Some(h) = o.hidden {
                shape.hidden = h;
            }
            if let Some(e) = &o.enumeration {
                shape.enumeration = Some(e.clone());
            }
            if let Some(f) = &o.format {
                shape.format = Some(f.clone());
            }
            if let Some(types) = &o.types {
                shape.types = types.clone();
            }
            if let Some(examples) = &o.examples {
                shape.examples = examples.clone();
            }
        }
        failed
    }

    fn create_path(&mut self, path: &str) -> bool {
        let Some(steps) = parse_path(path) else {
            return false;
        };
        let mut shape = &mut self.root;
        for (name, arrays) in steps {
            if let Some(name) = name {
                if !shape.types.contains(&JsonType::Object) {
                    shape.types.push(JsonType::Object);
                }
                shape = &mut shape
                    .fields
                    .entry(name)
                    .or_insert_with(|| Field {
                        presence: 0.0,
                        ..Field::default()
                    })
                    .shape;
            }
            for _ in 0..arrays {
                if !shape.types.contains(&JsonType::Array) {
                    shape.types.push(JsonType::Array);
                }
                shape = shape.items.get_or_insert_with(Default::default);
            }
        }
        true
    }
}

/// Settings applied to the shape at one path (see [`Schema::apply_overrides`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Override {
    /// What the value means.
    pub description: Option<String>,
    /// A short display name.
    pub label: Option<String>,
    /// Hide from completion.
    pub hidden: Option<bool>,
    /// The allowed values.
    #[serde(rename = "enum")]
    pub enumeration: Option<Vec<JsonValue>>,
    /// The string format.
    pub format: Option<String>,
    /// The types, replacing the inferred ones.
    #[serde(rename = "type")]
    pub types: Option<Vec<JsonType>>,
    /// Example values.
    pub examples: Option<Vec<JsonValue>>,
}

/// Parses `a.b[].c` into `[(Some("a"), 0), (Some("b"), 1), (Some("c"), 0)]`.
fn parse_path(path: &str) -> Option<Vec<(Option<String>, usize)>> {
    let path = path.trim();
    let path = path.strip_prefix('$').unwrap_or(path);
    let path = path.strip_prefix('.').unwrap_or(path);
    if path.is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for segment in path.split('.') {
        let mut name = segment;
        let mut arrays = 0;
        while let Some(rest) = name.strip_suffix("[]") {
            name = rest;
            arrays += 1;
        }
        let name = name.trim_matches('`');
        out.push(((!name.is_empty()).then(|| name.to_owned()), arrays));
    }
    Some(out)
}

impl Shape {
    /// Records one value.
    pub fn observe(&mut self, value: &JsonValue) {
        self.count += 1;
        let ty = match value {
            JsonValue::Null => JsonType::Null,
            JsonValue::Bool(_) => JsonType::Boolean,
            JsonValue::Number(n) => {
                let v = n.as_f64().unwrap_or(0.0);
                self.minimum = Some(self.minimum.map_or(v, |m| m.min(v)));
                self.maximum = Some(self.maximum.map_or(v, |m| m.max(v)));
                // Respect how the number was written: `1500.0` is a number, `1500` an integer.
                if (n.is_i64() || n.is_u64()) && v.fract() == 0.0 {
                    JsonType::Integer
                } else {
                    JsonType::Number
                }
            }
            JsonValue::String(s) => {
                self.strings += 1;
                let format = detect_format(s);
                self.common_format = Some(match self.common_format {
                    None => format,
                    Some(prev) if prev == format => prev,
                    Some(_) => None,
                });
                self.format = self.common_format.flatten().map(str::to_owned);
                JsonType::String
            }
            JsonValue::Array(items) => {
                let item_shape = self.items.get_or_insert_with(Default::default);
                for item in items {
                    item_shape.observe(item);
                }
                JsonType::Array
            }
            JsonValue::Object(map) => {
                self.objects += 1;
                for (k, v) in map {
                    self.fields.entry(k.clone()).or_default().shape.observe(v);
                }
                for field in self.fields.values_mut() {
                    field.required = field.shape.count == self.objects;
                    field.presence = field.shape.count as f64 / self.objects as f64;
                }
                JsonType::Object
            }
        };
        self.add_type(ty);
        if matches!(
            ty,
            JsonType::String | JsonType::Integer | JsonType::Number | JsonType::Boolean
        ) {
            if self.count == 1 {
                self.distinct = Some(Vec::new());
            }
            if let Some(distinct) = &mut self.distinct {
                if !distinct.contains(value) {
                    distinct.push(value.clone());
                }
                if distinct.len() > MAX_ENUM {
                    self.distinct = None;
                }
            }
            if self.examples.len() < MAX_EXAMPLES && !self.examples.contains(value) {
                self.examples.push(value.clone());
            }
        } else if !matches!(ty, JsonType::Null) {
            self.distinct = None;
        }
        // Repeated values from a small set look like an enumeration.
        self.enumeration = self
            .distinct
            .as_ref()
            .filter(|d| d.len() >= 2 && (d.len() as u64) < self.count)
            .cloned();
    }

    fn add_type(&mut self, ty: JsonType) {
        let ty = if ty == JsonType::Integer && self.types.contains(&JsonType::Number) {
            JsonType::Number
        } else {
            ty
        };
        if ty == JsonType::Number {
            self.types.retain(|t| *t != JsonType::Integer);
        }
        if !self.types.contains(&ty) {
            self.types.push(ty);
            self.types.sort();
        }
    }

    /// Whether objects with fields were seen here.
    pub fn is_object(&self) -> bool {
        self.types.contains(&JsonType::Object)
    }

    /// A short type description: `string`, `array<object>`, `integer | null`, ...
    pub fn type_summary(&self) -> String {
        if self.types.is_empty() {
            return "unknown".into();
        }
        let parts: Vec<String> = self
            .types
            .iter()
            .filter(|t| **t != JsonType::Null)
            .chain(self.types.iter().filter(|t| **t == JsonType::Null))
            .map(|t| match t {
                JsonType::Array => match &self.items {
                    Some(items) if !items.types.is_empty() => {
                        format!("array<{}>", items.type_summary())
                    }
                    _ => "array".into(),
                },
                other => other.name().into(),
            })
            .collect();
        parts.join(" | ")
    }
}

/// Recognizes common string formats.
fn detect_format(s: &str) -> Option<&'static str> {
    let b = s.as_bytes();
    let digits =
        |r: std::ops::Range<usize>| b.get(r).is_some_and(|d| d.iter().all(u8::is_ascii_digit));
    let is_date = b.len() >= 10
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10);
    let is_time_at =
        |i: usize| b.len() >= i + 5 && digits(i..i + 2) && b[i + 2] == b':' && digits(i + 3..i + 5);
    if is_date && b.len() == 10 {
        return Some("date");
    }
    if is_date && b.len() > 10 && matches!(b[10], b'T' | b't' | b' ') && is_time_at(11) {
        return Some("date-time");
    }
    if is_time_at(0) && (b.len() == 5 || (b.len() >= 8 && b[5] == b':' && digits(6..8))) {
        return Some("time");
    }
    let hex = |r: std::ops::Range<usize>| {
        b.get(r)
            .is_some_and(|d| d.iter().all(u8::is_ascii_hexdigit))
    };
    if b.len() == 36
        && hex(0..8)
        && b[8] == b'-'
        && hex(9..13)
        && b[13] == b'-'
        && hex(14..18)
        && b[18] == b'-'
        && hex(19..23)
        && b[23] == b'-'
        && hex(24..36)
    {
        return Some("uuid");
    }
    if let Some((user, domain)) = s.split_once('@')
        && !user.is_empty()
        && !user.contains(char::is_whitespace)
        && domain.contains('.')
        && !domain.contains(['@', ' '])
        && !domain.starts_with('.')
        && !domain.ends_with('.')
    {
        return Some("email");
    }
    if let Some((scheme, rest)) = s.split_once("://")
        && !scheme.is_empty()
        && scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        && !rest.is_empty()
        && !rest.contains(char::is_whitespace)
    {
        return Some("uri");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn infers_types_presence_and_items() {
        let schema = Schema::infer(&[
            json!({"id": 1, "price": 2.5, "items": [{"type": "a"}, {"type": "a", "extra": true}]}),
            json!({"id": 2, "price": 3, "items": []}),
        ]);
        let root = &schema.root;
        assert_eq!(root.fields["price"].shape.type_summary(), "number");
        assert_eq!(root.fields["items"].shape.type_summary(), "array<object>");
        let item = schema.shape_at("items[]").unwrap();
        assert!(item.fields["type"].required);
        assert!(!item.fields["extra"].required);
        assert_eq!(item.fields["extra"].presence, 0.5);
        assert_eq!(item.fields["type"].shape.enumeration, None);
        assert_eq!(schema.shape_at("id").unwrap().minimum, Some(1.0));
    }

    #[test]
    fn detects_formats_and_enumerations() {
        let schema = Schema::infer(&[
            json!({"at": "2024-01-02T10:00:00Z", "day": "2024-01-02", "mail": "a@b.co", "kind": "x"}),
            json!({"at": "2024-01-03T11:30:00+01:00", "day": "2024-01-03", "mail": "c@d.org", "kind": "y"}),
            json!({"at": "2024-01-04T12:00:00Z", "day": "2024-01-04", "mail": "e@f.io", "kind": "x"}),
        ]);
        let field = |n: &str| &schema.root.fields[n].shape;
        assert_eq!(field("at").format.as_deref(), Some("date-time"));
        assert_eq!(field("day").format.as_deref(), Some("date"));
        assert_eq!(field("mail").format.as_deref(), Some("email"));
        assert_eq!(
            field("kind").enumeration,
            Some(vec![json!("x"), json!("y")])
        );
        assert_eq!(field("mail").enumeration, None);
    }

    #[test]
    fn paths() {
        assert_eq!(
            parse_path("$.items[].`a b`").unwrap(),
            vec![(Some("items".into()), 1), (Some("a b".into()), 0)]
        );
        assert_eq!(parse_path("").unwrap(), vec![]);
    }
}
