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
//! | `x-jlens-enum-inferred` | The `enum` was inferred from samples, so it is not warned about |
//! | `x-jlens-domain` | An open domain: an identifier the application fetches the values of |
//! | `x-jlens-samples` | (root) Number of sample documents inferred from |

use std::collections::HashMap;

use indexmap::IndexSet;
use serde_json::{Map, Value as JsonValue, json};

use crate::shape::{Definitions, Field, JsonType, Schema, Shape};

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
            if !self.definitions.is_empty() {
                let definitions: Map<String, JsonValue> = self
                    .definitions
                    .iter()
                    .map(|(name, shape)| (name.clone(), export(shape)))
                    .collect();
                out.insert("$defs".into(), definitions.into());
            }
            if self.samples > 0 {
                out.insert("x-jlens-samples".into(), self.samples.into());
            }
            *map = out;
        }
        doc
    }

    /// Reads a JSON Schema document. Supports `type`, `properties`, `required`, `items`,
    /// `enum`, `const`, `format`, `minimum`, `maximum`, `title`, `description`, `examples`,
    /// local `$ref`s (`#`, `#/$defs/...`, `#/definitions/...`), and `anyOf`/`oneOf`/`allOf`
    /// (whose branches are merged). Other keywords are ignored.
    ///
    /// The target of a `$ref` is copied in its place. A recursive one, which leads back to
    /// itself, cannot be: its target becomes a definition of the schema
    /// ([`Schema::definitions`]) that the shape refers to by name ([`Shape::references`]), and
    /// the language service follows it as deep as an expression goes.
    ///
    /// ```
    /// use jlens_schema::{LanguageService, Schema};
    /// use serde_json::json;
    ///
    /// let schema = Schema::from_json_schema(&json!({
    ///     "type": "object",
    ///     "properties": {
    ///         "name": {"type": "string"},
    ///         "children": {"type": "array", "items": {"$ref": "#"}}
    ///     }
    /// }))
    /// .unwrap();
    /// let service = LanguageService::new(Some(schema));
    /// let warnings = service.diagnostics("children.children.children.nane");
    /// assert_eq!(warnings[0].code, "W0201");
    /// ```
    pub fn from_json_schema(doc: &JsonValue) -> Result<Schema, String> {
        if !doc.is_object() && !doc.is_boolean() {
            return Err("a JSON Schema must be an object or a boolean".into());
        }
        let mut importer = Importer::new(doc);
        let root = if importer.recursive.contains_key(ROOT) {
            Shape {
                references: vec![importer.define(ROOT)?],
                ..Shape::default()
            }
        } else {
            importer.import(doc, 0)?
        };
        // The definitions found on the way, and those their own shapes refer to.
        while let Some((name, target)) = importer.pending.pop() {
            let shape = importer.import(target, 0)?;
            importer.definitions.insert(name, shape);
        }
        let samples = doc
            .get("x-jlens-samples")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        Ok(Schema {
            root,
            samples,
            definitions: importer.definitions,
        })
    }
}

/// The `$ref` to the document itself.
const ROOT: &str = "#";
/// Where [`Schema::to_json_schema`] writes the definitions.
const DEFINITIONS: &str = "#/$defs/";

/// The `$ref` to the definition `name`, as a JSON Pointer.
fn reference_to(name: &str) -> String {
    format!(
        "{DEFINITIONS}{}",
        name.replace('~', "~0").replace('/', "~1")
    )
}

fn export(shape: &Shape) -> JsonValue {
    let mut out = Map::new();
    match shape.references.as_slice() {
        [] => {}
        [name] => {
            out.insert("$ref".into(), reference_to(name).into());
        }
        names => {
            let branches: Vec<JsonValue> = names
                .iter()
                .map(|name| json!({"$ref": reference_to(name)}))
                .collect();
            out.insert("anyOf".into(), branches.into());
        }
    }
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
        if shape.enumeration_inferred {
            out.insert("x-jlens-enum-inferred".into(), true.into());
        }
    }
    if let Some(d) = &shape.domain {
        out.insert("x-jlens-domain".into(), d.clone().into());
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

/// Every `$ref` under `node`, itself included, wherever it is written.
fn references_under<'d>(node: &'d JsonValue, out: &mut IndexSet<&'d str>) {
    // Iterative, so that a deep document cannot overflow the stack.
    let mut pending = vec![node];
    while let Some(node) = pending.pop() {
        match node {
            JsonValue::Object(map) => {
                if let Some(reference) = map.get("$ref").and_then(JsonValue::as_str) {
                    out.insert(reference);
                }
                pending.extend(map.values());
            }
            JsonValue::Array(items) => pending.extend(items),
            _ => {}
        }
    }
}

/// For each node of a graph, whether a path leads from it back to it (Tarjan's strongly
/// connected components, without recursion).
fn on_a_cycle(edges: &[Vec<usize>]) -> Vec<bool> {
    const UNSEEN: usize = usize::MAX;
    let count = edges.len();
    let mut index = vec![UNSEEN; count];
    let mut low = vec![0; count];
    let mut on_stack = vec![false; count];
    let mut stack = Vec::new();
    let mut out = vec![false; count];
    let mut next = 0;
    for start in 0..count {
        if index[start] != UNSEEN {
            continue;
        }
        let mut work = vec![(start, 0)];
        index[start] = next;
        low[start] = next;
        next += 1;
        stack.push(start);
        on_stack[start] = true;
        while let Some(&(node, edge)) = work.last() {
            if let Some(&target) = edges[node].get(edge) {
                let top = work.len() - 1;
                work[top].1 += 1;
                if index[target] == UNSEEN {
                    index[target] = next;
                    low[target] = next;
                    next += 1;
                    stack.push(target);
                    on_stack[target] = true;
                    work.push((target, 0));
                } else if on_stack[target] {
                    low[node] = low[node].min(index[target]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[node]);
            }
            if low[node] == index[node] {
                let mut component = Vec::new();
                while let Some(member) = stack.pop() {
                    on_stack[member] = false;
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                let cycle = component.len() > 1 || edges[node].contains(&node);
                for member in component {
                    out[member] = cycle;
                }
            }
        }
    }
    out
}

/// The reading of one JSON Schema document.
struct Importer<'d> {
    root: &'d JsonValue,
    /// The recursive `$ref`s, those that lead back to themselves, each with the name of its
    /// definition once a shape refers to it.
    recursive: HashMap<&'d str, Option<String>>,
    definitions: Definitions,
    /// The definitions still to read: their names and their targets.
    pending: Vec<(String, &'d JsonValue)>,
}

impl<'d> Importer<'d> {
    fn new(root: &'d JsonValue) -> Self {
        let mut references = IndexSet::new();
        references_under(root, &mut references);
        // An edge from each `$ref` to those under its target. The root's are all of them.
        let edges: Vec<Vec<usize>> = references
            .iter()
            .map(|reference| {
                if *reference == ROOT {
                    return (0..references.len()).collect();
                }
                let mut under = IndexSet::new();
                if let Some(target) = resolve(root, reference) {
                    references_under(target, &mut under);
                }
                under
                    .iter()
                    .filter_map(|r| references.get_index_of(r))
                    .collect()
            })
            .collect();
        let recursive = references
            .iter()
            .zip(on_a_cycle(&edges))
            .filter(|(_, cycle)| *cycle)
            .map(|(reference, _)| (*reference, None))
            .collect();
        Importer {
            root,
            recursive,
            definitions: Definitions::new(),
            pending: Vec::new(),
        }
    }

    /// The name of the definition a recursive `$ref` leads to, which is read later.
    fn define(&mut self, reference: &'d str) -> Result<String, String> {
        if let Some(Some(name)) = self.recursive.get(reference) {
            return Ok(name.clone());
        }
        let target = resolve(self.root, reference)
            .ok_or_else(|| format!("cannot resolve $ref {reference}"))?;
        // `#/$defs/node` is named `node`, as the export writes it back; any other by its text.
        let mut name = match reference.strip_prefix(DEFINITIONS) {
            Some(name) if !name.contains('/') => name.replace("~1", "/").replace("~0", "~"),
            _ => reference.to_owned(),
        };
        while self.definitions.contains_key(&name) {
            name.push('\'');
        }
        // Its place among the definitions is that of the first reference to it.
        self.definitions.insert(name.clone(), Shape::default());
        self.pending.push((name.clone(), target));
        self.recursive.insert(reference, Some(name.clone()));
        Ok(name)
    }

    fn import(&mut self, node: &'d JsonValue, depth: usize) -> Result<Shape, String> {
        if depth > 64 {
            return Err("the schema is nested too deep".into());
        }
        let JsonValue::Object(map) = node else {
            return Ok(Shape::default()); // `true` / `false`: nothing known
        };
        let mut shape = match map.get("$ref").and_then(JsonValue::as_str) {
            Some(r) if self.recursive.contains_key(r) => Shape {
                references: vec![self.define(r)?],
                ..Shape::default()
            },
            Some(r) => {
                let target =
                    resolve(self.root, r).ok_or_else(|| format!("cannot resolve $ref {r}"))?;
                self.import(target, depth + 1)?
            }
            None => Shape::default(),
        };
        for key in ["allOf", "anyOf", "oneOf"] {
            if let Some(JsonValue::Array(branches)) = map.get(key) {
                for branch in branches {
                    let b = self.import(branch, depth + 1)?;
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
                let field_shape = self.import(p, depth + 1)?;
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
                shape.items = Some(Box::new(self.import(items, depth + 1)?));
                add_type(&mut shape, "array");
            }
            Some(JsonValue::Array(tuple)) => {
                // Draft-07 tuple form: merge the item schemas.
                let mut merged = Shape::default();
                for item in tuple {
                    merge(&mut merged, self.import(item, depth + 1)?);
                }
                shape.items = Some(Box::new(merged));
                add_type(&mut shape, "array");
            }
            _ => {}
        }
        if let Some(JsonValue::Array(values)) = map.get("enum") {
            shape.enumeration = Some(values.clone());
            shape.enumeration_inferred = map
                .get("x-jlens-enum-inferred")
                .and_then(JsonValue::as_bool)
                == Some(true);
        }
        if let Some(value) = map.get("const") {
            shape.enumeration = Some(vec![value.clone()]);
            shape.enumeration_inferred = false;
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
        if let Some(d) = map.get("x-jlens-domain").and_then(JsonValue::as_str) {
            shape.domain = Some(d.to_owned());
        }
        if map.get("x-jlens-hidden").and_then(JsonValue::as_bool) == Some(true) {
            shape.hidden = true;
        }
        Ok(shape)
    }
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
    for name in other.references {
        if !shape.references.contains(&name) {
            shape.references.push(name);
        }
    }
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
    if shape.enumeration.is_none() {
        shape.enumeration = other.enumeration;
        shape.enumeration_inferred = other.enumeration_inferred;
    }
    shape.domain = shape.domain.take().or(other.domain);
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
            enumeration_inferred: self.enumeration_inferred,
            domain: self.domain.clone(),
            examples: self.examples.clone(),
            description: self.description.clone(),
            label: self.label.clone(),
            hidden: self.hidden,
            references: self.references.clone(),
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
        schema.root.fields["owner"].shape.fields["name"]
            .shape
            .domain = Some("people".into());
        let doc = schema.to_json_schema();
        assert_eq!(doc["$schema"], DIALECT);
        assert_eq!(doc["required"], json!(["id", "kind", "tags", "at"]));
        assert_eq!(
            doc["properties"]["owner"]["x-jlens-presence"],
            json!(2.0 / 3.0)
        );
        // `kind` was seen as `a` and `b`: its enumeration is a guess, and stays one.
        assert_eq!(doc["properties"]["kind"]["x-jlens-enum-inferred"], true);
        assert_eq!(
            doc["properties"]["owner"]["properties"]["name"]["x-jlens-domain"],
            "people"
        );
        let back = Schema::from_json_schema(&doc).unwrap();
        assert_eq!(back.root, schema.root.without_state());
        assert!(back.root.fields["kind"].shape.enumeration_inferred);
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
        assert!(!schema.shape_at("status").unwrap().enumeration_inferred);
    }
}
