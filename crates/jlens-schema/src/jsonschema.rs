//! Conversion between [`Schema`] and JSON Schema (draft 2020-12).
//!
//! JSON Schema is the interchange format: `jlens schema infer` writes it, editors and the
//! language service read it, and people can edit it by hand. jlens-specific information travels
//! in `x-jlens-*` keywords that other tools ignore:
//!
//! | Keyword | Meaning |
//! |---|---|
//! | `title` | The shape's label |
//! | `x-jlens-presence` | Fraction of objects that have an optional field |
//! | `x-jlens-hidden` | Hide from completion |
//! | `x-jlens-samples` | (root) Number of sample documents inferred from |

use serde_json::{Map, Value as JsonValue, json};

use crate::shape::{Field, JsonType, Schema, Shape};

const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

impl Schema {
    /// Converts to a JSON Schema document.
    ///
    /// ```
    /// use jlens_schema::Schema;
    /// use serde_json::json;
    ///
    /// let schema = Schema::infer(&[json!({"name": "a", "tags": ["x"]})]);
    /// let doc = schema.to_json_schema();
    /// assert_eq!(doc["properties"]["tags"]["items"]["type"], "string");
    /// assert_eq!(Schema::from_json_schema(&doc).unwrap().root, schema.root.without_state());
    /// ```
    pub fn to_json_schema(&self) -> JsonValue {
        let mut doc = export(&self.root);
        if let JsonValue::Object(map) = &mut doc {
            let mut out = Map::new();
            out.insert("$schema".into(), DIALECT.into());
            out.extend(std::mem::take(map));
            if self.samples > 0 {
                out.insert("x-jlens-samples".into(), self.samples.into());
            }
            *map = out;
        }
        doc
    }

    /// Reads a JSON Schema document. Supports `type`, `properties`, `required`, `items`,
    /// `enum`, `const`, `format`, `minimum`, `maximum`, `title`, `description`, `examples`,
    /// local `$ref`s (`#/$defs/...`, `#/definitions/...`), and `anyOf`/`oneOf`/`allOf` (whose
    /// branches are merged). Other keywords are ignored.
    pub fn from_json_schema(doc: &JsonValue) -> Result<Schema, String> {
        if !doc.is_object() && !doc.is_boolean() {
            return Err("a JSON Schema must be an object or a boolean".into());
        }
        let root = import(doc, doc, 0)?;
        let samples = doc
            .get("x-jlens-samples")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        Ok(Schema { root, samples })
    }
}

fn export(shape: &Shape) -> JsonValue {
    let mut out = Map::new();
    let types: Vec<JsonValue> = shape.types.iter().map(|t| t.name().into()).collect();
    match types.len() {
        0 => {}
        1 => {
            out.insert("type".into(), types[0].clone());
        }
        _ => {
            out.insert("type".into(), types.into());
        }
    }
    if let Some(label) = &shape.label {
        out.insert("title".into(), label.clone().into());
    }
    if let Some(d) = &shape.description {
        out.insert("description".into(), d.clone().into());
    }
    if let Some(f) = &shape.format {
        out.insert("format".into(), f.clone().into());
    }
    if let Some(e) = &shape.enumeration {
        out.insert("enum".into(), e.clone().into());
    }
    if let Some(m) = shape.minimum {
        out.insert("minimum".into(), json_number(m));
    }
    if let Some(m) = shape.maximum {
        out.insert("maximum".into(), json_number(m));
    }
    if !shape.examples.is_empty() {
        out.insert("examples".into(), shape.examples.clone().into());
    }
    if !shape.fields.is_empty() {
        let mut props = Map::new();
        let mut required = Vec::new();
        for (name, field) in &shape.fields {
            let mut p = export(&field.shape);
            if field.required {
                required.push(JsonValue::from(name.clone()));
            } else if let JsonValue::Object(m) = &mut p
                && field.presence > 0.0
            {
                m.insert("x-jlens-presence".into(), json_number(field.presence));
            }
            props.insert(name.clone(), p);
        }
        out.insert("properties".into(), props.into());
        if !required.is_empty() {
            out.insert("required".into(), required.into());
        }
    }
    if let Some(items) = &shape.items {
        out.insert("items".into(), export(items));
    }
    if shape.hidden {
        out.insert("x-jlens-hidden".into(), true.into());
    }
    out.into()
}

fn json_number(n: f64) -> JsonValue {
    if n.fract() == 0.0 && n.abs() < 9e15 {
        (n as i64).into()
    } else {
        json!(n)
    }
}

fn resolve<'a>(root: &'a JsonValue, reference: &str) -> Option<&'a JsonValue> {
    let pointer = reference.strip_prefix('#')?;
    root.pointer(pointer)
}

fn import(node: &JsonValue, root: &JsonValue, depth: usize) -> Result<Shape, String> {
    if depth > 64 {
        return Err("$ref nesting is too deep (recursive schema?)".into());
    }
    let JsonValue::Object(map) = node else {
        return Ok(Shape::default()); // `true` / `false`: nothing known
    };
    let mut shape = match map.get("$ref").and_then(JsonValue::as_str) {
        Some(r) => {
            let target = resolve(root, r).ok_or_else(|| format!("cannot resolve $ref {r}"))?;
            import(target, root, depth + 1)?
        }
        None => Shape::default(),
    };
    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(JsonValue::Array(branches)) = map.get(key) {
            for branch in branches {
                let b = import(branch, root, depth + 1)?;
                merge(&mut shape, b);
            }
        }
    }
    match map.get("type") {
        Some(JsonValue::String(t)) => add_type(&mut shape, t),
        Some(JsonValue::Array(ts)) => {
            for t in ts.iter().filter_map(JsonValue::as_str) {
                add_type(&mut shape, t);
            }
        }
        _ => {}
    }
    if let Some(JsonValue::Object(props)) = map.get("properties") {
        let required: Vec<&str> = map
            .get("required")
            .and_then(JsonValue::as_array)
            .map(|r| r.iter().filter_map(JsonValue::as_str).collect())
            .unwrap_or_default();
        for (name, p) in props {
            let field_shape = import(p, root, depth + 1)?;
            let is_required = required.contains(&name.as_str());
            let presence = p
                .get("x-jlens-presence")
                .and_then(JsonValue::as_f64)
                .unwrap_or(if is_required { 1.0 } else { 0.0 });
            shape.fields.insert(
                name.clone(),
                Field {
                    shape: field_shape,
                    required: is_required,
                    presence,
                },
            );
        }
        add_type(&mut shape, "object");
    }
    match map.get("items") {
        Some(items @ JsonValue::Object(_)) => {
            shape.items = Some(Box::new(import(items, root, depth + 1)?));
            add_type(&mut shape, "array");
        }
        Some(JsonValue::Array(tuple)) => {
            // Draft-07 tuple form: merge the item schemas.
            let mut merged = Shape::default();
            for item in tuple {
                merge(&mut merged, import(item, root, depth + 1)?);
            }
            shape.items = Some(Box::new(merged));
            add_type(&mut shape, "array");
        }
        _ => {}
    }
    if let Some(JsonValue::Array(values)) = map.get("enum") {
        shape.enumeration = Some(values.clone());
    }
    if let Some(value) = map.get("const") {
        shape.enumeration = Some(vec![value.clone()]);
    }
    if let Some(f) = map.get("format").and_then(JsonValue::as_str) {
        shape.format = Some(f.to_owned());
    }
    shape.minimum = map
        .get("minimum")
        .and_then(JsonValue::as_f64)
        .or(shape.minimum);
    shape.maximum = map
        .get("maximum")
        .and_then(JsonValue::as_f64)
        .or(shape.maximum);
    if let Some(t) = map.get("title").and_then(JsonValue::as_str) {
        shape.label = Some(t.to_owned());
    }
    if let Some(d) = map.get("description").and_then(JsonValue::as_str) {
        shape.description = Some(d.to_owned());
    }
    if let Some(JsonValue::Array(examples)) = map.get("examples") {
        shape.examples = examples.clone();
    }
    if map.get("x-jlens-hidden").and_then(JsonValue::as_bool) == Some(true) {
        shape.hidden = true;
    }
    Ok(shape)
}

fn add_type(shape: &mut Shape, name: &str) {
    if let Some(t) = JsonType::from_name(name)
        && !shape.types.contains(&t)
    {
        shape.types.push(t);
        shape.types.sort();
    }
}

/// Merges `other` into `shape` (union of types and fields).
fn merge(shape: &mut Shape, other: Shape) {
    for t in other.types {
        if !shape.types.contains(&t) {
            shape.types.push(t);
        }
    }
    shape.types.sort();
    for (name, field) in other.fields {
        match shape.fields.get_mut(&name) {
            Some(existing) => {
                existing.required &= field.required;
                merge(&mut existing.shape, field.shape);
            }
            None => {
                shape.fields.insert(name, field);
            }
        }
    }
    match (&mut shape.items, other.items) {
        (Some(a), Some(b)) => merge(a, *b),
        (slot @ None, Some(b)) => *slot = Some(b),
        _ => {}
    }
    shape.enumeration = shape.enumeration.take().or(other.enumeration);
    shape.format = shape.format.take().or(other.format);
    shape.description = shape.description.take().or(other.description);
    shape.label = shape.label.take().or(other.label);
}

impl Shape {
    /// A copy without inference bookkeeping, as produced by a JSON Schema round trip.
    pub fn without_state(&self) -> Shape {
        let mut copy = Shape {
            types: self.types.clone(),
            fields: Default::default(),
            items: self.items.as_ref().map(|i| Box::new(i.without_state())),
            minimum: self.minimum,
            maximum: self.maximum,
            format: self.format.clone(),
            enumeration: self.enumeration.clone(),
            examples: self.examples.clone(),
            description: self.description.clone(),
            label: self.label.clone(),
            hidden: self.hidden,
            ..Shape::default()
        };
        for (k, f) in &self.fields {
            copy.fields.insert(
                k.clone(),
                Field {
                    shape: f.shape.without_state(),
                    required: f.required,
                    presence: f.presence,
                },
            );
        }
        copy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json_schema() {
        let mut schema = Schema::infer(&[
            json!({"id": 1, "kind": "a", "tags": ["x"], "owner": {"name": "Ann"}, "at": "2024-01-01"}),
            json!({"id": 2.5, "kind": "a", "tags": [], "at": "2024-02-01"}),
            json!({"id": 3, "kind": "b", "tags": ["y", "y"], "owner": null, "at": "2024-03-01"}),
        ]);
        schema.root.fields["kind"].shape.description = Some("Kind of thing".into());
        let doc = schema.to_json_schema();
        assert_eq!(doc["$schema"], DIALECT);
        assert_eq!(doc["required"], json!(["id", "kind", "tags", "at"]));
        assert_eq!(
            doc["properties"]["owner"]["x-jlens-presence"],
            json!(2.0 / 3.0)
        );
        let back = Schema::from_json_schema(&doc).unwrap();
        assert_eq!(back.root, schema.root.without_state());
        assert_eq!(back.samples, 3);
    }

    #[test]
    fn imports_refs_and_combinators() {
        let doc = json!({
            "$defs": {"item": {"type": "object", "properties": {"sku": {"type": "string"}}}},
            "type": "object",
            "properties": {
                "items": {"type": "array", "items": {"$ref": "#/$defs/item"}},
                "status": {"anyOf": [{"const": "open"}, {"type": "null"}]}
            }
        });
        let schema = Schema::from_json_schema(&doc).unwrap();
        assert!(schema.shape_at("items[].sku").is_some());
        assert_eq!(schema.shape_at("status").unwrap().type_summary(), "null");
        assert_eq!(
            schema.shape_at("status").unwrap().enumeration,
            Some(vec![json!("open")])
        );
    }
}
