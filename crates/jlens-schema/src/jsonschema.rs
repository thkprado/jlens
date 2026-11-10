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

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{Map, Value as JsonValue, json};

use crate::shape::{Definitions, Field, JsonType, Schema, Shape};
use crate::uri::{percent_decode, resolve, split_fragment};

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
        let mut doc = export(&self.root, &self.definitions);
        if let JsonValue::Object(map) = &mut doc {
            let mut out = Map::new();
            out.insert("$schema".into(), DIALECT.into());
            out.extend(std::mem::take(map));
            if !self.definitions.is_empty() {
                let definitions: Map<String, JsonValue> = self
                    .definitions
                    .iter()
                    .map(|(name, shape)| (name.clone(), export(shape, &self.definitions)))
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
    /// `$ref`, `$id`, `$anchor`, and `anyOf`/`oneOf`/`allOf` (whose branches are merged). Other
    /// keywords are ignored.
    ///
    /// The target of a `$ref` is copied in its place. A recursive one, which leads back to
    /// itself, cannot be: its target becomes a definition of the schema
    /// ([`Schema::definitions`]) that the shape refers to by name ([`Shape::references`]), and
    /// the language service follows it as deep as an expression goes. So does a target whose
    /// copies would make the schema much larger than its document.
    ///
    /// A `$ref` that leads to another document, or to a place the document does not have,
    /// leaves its place unknown: nothing is completed and nothing is warned under it.
    /// [`SchemaLoader`] takes the other documents and tells which `$ref`s it did not follow.
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
        SchemaLoader::new().load(doc).map(|(schema, _)| schema)
    }
}

/// Reads a JSON Schema whose `$ref`s lead to other documents: the application gives each
/// document with its URI, as jlens never fetches one.
///
/// A `$ref` is resolved against the `$id` of the schema it is in, or the URI its document was
/// given with, and its fragment is a JSON Pointer or an `$anchor`. A document is also known by
/// the `$id`s it declares, at its root or inside. Two URIs are the same when their texts are
/// equal once resolved.
///
/// The load does not fail for a `$ref` it cannot follow: the place is left unknown, so that
/// nothing is warned under it, and the [`LoadReport`] names it, with the document to give.
///
/// ```
/// use jlens_schema::{LanguageService, SchemaLoader};
/// use serde_json::json;
///
/// let order = json!({
///     "$id": "https://example.com/schemas/order.json",
///     "type": "object",
///     "properties": {
///         "customer": {"$ref": "common.json#/$defs/person"},
///         "carrier": {"$ref": "carrier.json"}
///     }
/// });
/// let common = json!({
///     "$defs": {"person": {"type": "object", "properties": {"name": {"type": "string"}}}}
/// });
/// let (schema, report) = SchemaLoader::new()
///     .document("https://example.com/schemas/common.json", &common)
///     .load(&order)
///     .unwrap();
/// assert_eq!(report.documents(), ["https://example.com/schemas/carrier.json"]);
/// assert_eq!(report.unresolved[0].place, "#/properties/carrier");
///
/// let service = LanguageService::new(Some(schema));
/// let codes = |text: &str| -> Vec<_> {
///     service.diagnostics(text).into_iter().map(|d| d.code).collect()
/// };
/// assert_eq!(codes("customer.nane"), ["W0201"]);
/// assert!(codes("carrier.anything").is_empty());
/// ```
#[derive(Debug, Clone, Default)]
pub struct SchemaLoader<'d> {
    documents: Vec<(String, &'d JsonValue)>,
}

impl<'d> SchemaLoader<'d> {
    /// A loader with no document: every `$ref` to another one is left unknown.
    pub fn new() -> Self {
        SchemaLoader::default()
    }

    /// Gives the document that `uri` names. Of two given with the same URI, the first counts.
    pub fn document(mut self, uri: &str, document: &'d JsonValue) -> Self {
        self.documents.push((uri.to_owned(), document));
        self
    }

    /// Reads `document`, whose `$ref`s are resolved against its `$id`. Without one, a
    /// reference to another document is taken as written (`common.json#/$defs/person` is a
    /// place of the document given as `common.json`).
    ///
    /// Fails for a document that is not a schema, or one nested more than 64 levels deep.
    pub fn load(&self, document: &JsonValue) -> Result<(Schema, LoadReport), String> {
        self.load_at("", document)
    }

    /// Reads `document` as the one that `uri` names: its `$ref`s are resolved against `uri`
    /// where no `$id` says otherwise.
    pub fn load_at(&self, uri: &str, document: &JsonValue) -> Result<(Schema, LoadReport), String> {
        if !document.is_object() && !document.is_boolean() {
            return Err("a JSON Schema must be an object or a boolean".into());
        }
        let mut importer = Importer::new(uri, document, &self.documents);
        let root = match importer.root {
            Some(root) if importer.kept[root] => Shape {
                references: vec![importer.define(root, ROOT)],
                ..Shape::default()
            },
            _ => importer.import(document, 0)?,
        };
        // The definitions found on the way, and those their own shapes refer to.
        while let Some((name, target)) = importer.pending.pop() {
            let shape = importer.import(target, 0)?;
            importer.definitions.insert(name, shape);
        }
        let samples = document
            .get("x-jlens-samples")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let report = importer.report();
        let schema = Schema {
            root,
            samples,
            definitions: importer.definitions,
        };
        Ok((schema, report))
    }
}

/// What a load did not read of its JSON Schema ([`SchemaLoader::load`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct LoadReport {
    /// The `$ref`s that were not followed, in the order the load met them. A `$ref` that
    /// nothing of the schema leads to is not read, and so not reported.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unresolved: Vec<UnresolvedRef>,
}

impl LoadReport {
    /// Whether everything was read.
    pub fn is_empty(&self) -> bool {
        self.unresolved.is_empty()
    }

    /// The URIs of the documents that were not given, each once: with them, the load would
    /// follow the `$ref`s that lead there.
    pub fn documents(&self) -> Vec<&str> {
        let mut out = Vec::new();
        for uri in self.unresolved.iter().filter_map(|u| u.document.as_deref()) {
            if !out.contains(&uri) {
                out.push(uri);
            }
        }
        out
    }
}

/// A `$ref` that a load did not follow: its place in the schema is unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct UnresolvedRef {
    /// Where the `$ref` is written: the URI of its document, as it was given, and a JSON
    /// Pointer as its fragment (`#/properties/carrier` in a document given with no URI).
    pub place: String,
    /// The `$ref` as written.
    pub reference: String,
    /// The `$ref` resolved against its base: the place it names.
    pub uri: String,
    /// The URI of the document that was not given. `None` when the document is there and has
    /// no such place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<String>,
}

/// The `$ref` to the document itself.
const ROOT: &str = "#";
/// Where [`Schema::to_json_schema`] writes the definitions.
const DEFINITIONS: &str = "#/$defs/";

/// The `$ref` to the definition `name`, as a JSON Pointer. A name the schema does not define
/// is that of a `$ref` the load did not follow: the URI it resolved to, written back as it is.
fn reference_to(name: &str, definitions: &Definitions) -> String {
    if !definitions.contains_key(name) {
        return name.to_owned();
    }
    format!(
        "{DEFINITIONS}{}",
        name.replace('~', "~0").replace('/', "~1")
    )
}

fn export(shape: &Shape, definitions: &Definitions) -> JsonValue {
    let mut out = Map::new();
    match shape.references.as_slice() {
        [] => {}
        [name] => {
            out.insert("$ref".into(), reference_to(name, definitions).into());
        }
        names => {
            let branches: Vec<JsonValue> = names
                .iter()
                .map(|name| json!({"$ref": reference_to(name, definitions)}))
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
            let mut p = export(&field.shape, definitions);
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
        out.insert("items".into(), export(items, definitions));
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

/// What tells one node of a document from every other: where it is.
fn address(node: &JsonValue) -> usize {
    std::ptr::from_ref(node) as usize
}

/// The schemas directly under the schema `map` that the import reads. Keep it in step with
/// [`Importer::import`]: a `$ref` under a keyword that is missing here is not seen by the
/// search for recursion.
fn subschemas<'d>(map: &'d Map<String, JsonValue>, out: &mut Vec<&'d JsonValue>) {
    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(JsonValue::Array(branches)) = map.get(key) {
            out.extend(branches);
        }
    }
    if let Some(JsonValue::Object(properties)) = map.get("properties") {
        out.extend(properties.values());
    }
    match map.get("items") {
        Some(items @ JsonValue::Object(_)) => out.push(items),
        Some(JsonValue::Array(tuple)) => out.extend(tuple),
        _ => {}
    }
}

/// For each node of a graph, whether a path leads from it back to it (Tarjan's strongly
/// connected components, without recursion); and the nodes in an order where each comes after
/// those it leads to, but for the ones on a cycle with it.
fn on_a_cycle(edges: &[Vec<usize>]) -> (Vec<bool>, Vec<usize>) {
    const UNSEEN: usize = usize::MAX;
    let count = edges.len();
    let mut index = vec![UNSEEN; count];
    let mut low = vec![0; count];
    let mut on_stack = vec![false; count];
    let mut stack = Vec::new();
    let mut out = vec![false; count];
    let mut order = Vec::with_capacity(count);
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
                    order.push(member);
                }
            }
        }
    }
    (out, order)
}

/// The least number of JSON values that the shapes of a schema may be read from, whatever the
/// size of its documents.
const LEAST_BUDGET: u64 = 100_000;
/// How many times the size of its documents, in JSON values, a schema may be read from.
const BUDGET_FACTOR: u64 = 16;

/// Which targets of `$ref`s are kept as definitions, not copied in the place of each `$ref`:
/// those on a cycle, which have no end; and, when the copies would read more than `budget`
/// values, those that read more than their share of it.
///
/// `edges` has, for each target, the target of each `$ref` under it, once for each time it is
/// written; `own` has the number of values under it; `root` is the document, read once.
fn kept_as_definitions(edges: &[Vec<usize>], own: &[u64], root: usize, budget: u64) -> Vec<bool> {
    let (cyclic, order) = on_a_cycle(edges);
    // What reading everything costs, with each target that reads more than `limit` kept.
    let pass = |limit: u64| {
        let mut kept = cyclic.clone();
        let mut read = vec![0_u64; edges.len()];
        let mut total = 0_u64;
        for &node in &order {
            let mut size = own[node];
            for &target in &edges[node] {
                // A target of the same cycle is kept, so its size is not asked for.
                size = size.saturating_add(if kept[target] { 1 } else { read[target] });
            }
            read[node] = size;
            if size > limit && node != root {
                kept[node] = true;
            }
            if kept[node] || node == root {
                total = total.saturating_add(size);
            }
        }
        (kept, total)
    };
    let (kept, total) = pass(u64::MAX);
    if total <= budget {
        return kept;
    }
    // Each `$ref` copies at most `limit` values: no more than the budget between them all.
    let written: u64 = edges.iter().map(|e| e.len() as u64).sum();
    pass(budget / written.max(1)).0
}

/// What a `$ref` leads to.
#[derive(Clone, Copy)]
enum Target {
    /// A place of the documents, by its index among the targets.
    Found(usize),
    /// Nothing the load has, by its index among the missing.
    Missing(usize),
}

/// The places of the documents that a URI may name, and their `$ref`s.
struct Index<'d> {
    /// From the URI of each document and of each schema with an `$id` to its root, and from
    /// `uri#anchor` to the schema with that `$anchor`.
    places: HashMap<String, &'d JsonValue>,
    /// Each schema with a `$ref`, the base it is resolved against, and its text.
    references: Vec<(&'d JsonValue, String, &'d str)>,
    /// The number of JSON values.
    values: u64,
}

impl<'d> Index<'d> {
    /// Reads `document`, given as the one `uri` names.
    fn add(&mut self, uri: &str, document: &'d JsonValue) {
        let base = split_fragment(&resolve("", uri)).0.to_owned();
        self.places.entry(base.clone()).or_insert(document);
        // Iterative, so that a deep document cannot overflow the stack. A value is read as a
        // schema, or as a map from names to schemas (`properties`), whose own names are not
        // keywords.
        let mut pending = vec![(document, std::rc::Rc::<str>::from(base), true)];
        while let Some((node, mut base, schema)) = pending.pop() {
            self.values += 1;
            match node {
                JsonValue::Object(map) if schema => {
                    let text = |key: &str| map.get(key).and_then(JsonValue::as_str);
                    match text("$id").map(|id| (id, id.strip_prefix('#'))) {
                        // Until draft-07, an anchor was an `$id` that is a fragment alone.
                        Some((_, Some(anchor))) => {
                            let place = format!("{base}#{anchor}");
                            self.places.entry(place).or_insert(node);
                        }
                        Some((id, None)) => {
                            let uri = resolve(&base, id);
                            let uri = split_fragment(&uri).0;
                            self.places.entry(uri.to_owned()).or_insert(node);
                            base = uri.into();
                        }
                        None => {}
                    }
                    if let Some(anchor) = text("$anchor") {
                        let place = format!("{base}#{anchor}");
                        self.places.entry(place).or_insert(node);
                    }
                    if let Some(reference) = text("$ref") {
                        self.references.push((node, base.to_string(), reference));
                    }
                    for (key, value) in map {
                        match key.as_str() {
                            // Values, not schemas.
                            "enum" | "const" | "examples" | "default" => {}
                            "properties" | "patternProperties" | "dependentSchemas" | "$defs"
                            | "definitions" => pending.push((value, base.clone(), false)),
                            _ => pending.push((value, base.clone(), true)),
                        }
                    }
                }
                JsonValue::Object(map) => {
                    pending.extend(map.values().map(|value| (value, base.clone(), true)));
                }
                JsonValue::Array(items) => {
                    pending.extend(items.iter().map(|value| (value, base.clone(), true)));
                }
                _ => {}
            }
        }
    }

    /// The place `uri` names; otherwise the document that is missing, or `None` when the
    /// document is there without that place.
    fn find(&self, uri: &str) -> Result<&'d JsonValue, Option<String>> {
        let (document, fragment) = split_fragment(uri);
        let root = *self
            .places
            .get(document)
            .ok_or_else(|| Some(document.to_owned()))?;
        if fragment.is_empty() {
            return Ok(root);
        }
        let fragment = percent_decode(fragment).ok_or(None)?;
        if fragment.starts_with('/') {
            root.pointer(&fragment).ok_or(None)
        } else {
            let anchor = format!("{document}#{fragment}");
            self.places.get(&anchor).copied().ok_or(None)
        }
    }
}

/// The reading of one JSON Schema, and of the documents its `$ref`s lead to.
struct Importer<'d> {
    /// The documents with the URIs they were given with, the one to read first.
    documents: Vec<(&'d str, &'d JsonValue)>,
    /// What the `$ref` of each schema leads to, by the schema's address.
    references: HashMap<usize, Target>,
    /// The places that `$ref`s lead to.
    targets: Vec<&'d JsonValue>,
    /// For each target, whether it is a definition, which the shapes refer to by name; the
    /// others are copied in the place of each `$ref`.
    kept: Vec<bool>,
    /// The name of each target's definition, once a shape refers to it.
    names: Vec<Option<String>>,
    /// The document itself among the targets, when a `$ref` leads to it.
    root: Option<usize>,
    /// The URIs that lead to nothing, which are the names their places refer to, each with
    /// the document that is missing.
    missing: IndexMap<String, Option<String>>,
    /// The `$ref`s the import met that lead to nothing: the schema's address, and its text
    /// and what it is missing.
    unresolved: IndexMap<usize, (&'d str, Option<usize>)>,
    definitions: Definitions,
    /// The definitions still to read: their names and their targets.
    pending: Vec<(String, &'d JsonValue)>,
}

impl<'d> Importer<'d> {
    fn new(uri: &'d str, document: &'d JsonValue, others: &'d [(String, &'d JsonValue)]) -> Self {
        let mut documents = vec![(uri, document)];
        documents.extend(
            others
                .iter()
                .map(|(uri, document)| (uri.as_str(), *document)),
        );
        let mut index = Index {
            places: HashMap::new(),
            references: Vec::new(),
            values: 0,
        };
        for (uri, document) in &documents {
            index.add(uri, document);
        }
        let mut targets: IndexMap<usize, &'d JsonValue> = IndexMap::new();
        let mut missing = IndexMap::new();
        let mut references = HashMap::with_capacity(index.references.len());
        for (node, base, reference) in &index.references {
            let uri = resolve(base, reference);
            let target = match index.find(&uri) {
                Ok(target) => Target::Found(targets.insert_full(address(target), target).0),
                Err(document) => Target::Missing(missing.insert_full(uri, document).0),
            };
            references.insert(address(node), target);
        }
        let root = targets.get_index_of(&address(document));
        let targets: Vec<&'d JsonValue> = targets.into_values().collect();

        // The graph of the targets: from each to those of the `$ref`s the import reads under
        // it. The document is its last node, unless it is a target itself.
        let mut edges = Vec::with_capacity(targets.len() + 1);
        let mut own = Vec::with_capacity(targets.len() + 1);
        let top = root.is_none().then_some(document);
        for node in targets.iter().copied().chain(top) {
            let mut under = Vec::new();
            let mut values = 0_u64;
            let mut pending = vec![node];
            while let Some(node) = pending.pop() {
                values += 1;
                let JsonValue::Object(map) = node else {
                    continue;
                };
                if let Some(Target::Found(target)) = references.get(&address(node))
                    && map.get("$ref").is_some_and(JsonValue::is_string)
                {
                    under.push(*target);
                }
                subschemas(map, &mut pending);
            }
            edges.push(under);
            own.push(values);
        }
        let budget = LEAST_BUDGET.max(index.values.saturating_mul(BUDGET_FACTOR));
        let mut kept = kept_as_definitions(&edges, &own, root.unwrap_or(targets.len()), budget);
        kept.truncate(targets.len());
        Importer {
            documents,
            references,
            names: vec![None; targets.len()],
            targets,
            kept,
            root,
            missing,
            unresolved: IndexMap::new(),
            definitions: Definitions::new(),
            pending: Vec::new(),
        }
    }

    /// The name of the definition that target `target` is, which is read later. `reference`
    /// is the `$ref` that leads to it.
    fn define(&mut self, target: usize, reference: &str) -> String {
        if let Some(name) = &self.names[target] {
            return name.clone();
        }
        // `#/$defs/node` is named `node`, as the export writes it back; any other by its text.
        let mut name = match reference.strip_prefix(DEFINITIONS) {
            Some(name) if !name.contains('/') => name.replace("~1", "/").replace("~0", "~"),
            _ => reference.to_owned(),
        };
        // A name that leads to nothing must stay one.
        while self.definitions.contains_key(&name) || self.missing.contains_key(&name) {
            name.push('\'');
        }
        // Its place among the definitions is that of the first reference to it.
        self.definitions.insert(name.clone(), Shape::default());
        self.pending.push((name.clone(), self.targets[target]));
        self.names[target] = Some(name.clone());
        name
    }

    /// The shape in the place of the `$ref` of schema `node`.
    fn follow(
        &mut self,
        node: &'d JsonValue,
        reference: &'d str,
        depth: usize,
    ) -> Result<Shape, String> {
        let name = match self.references.get(&address(node)).copied() {
            Some(Target::Found(target)) if self.kept[target] => self.define(target, reference),
            Some(Target::Found(target)) => return self.import(self.targets[target], depth + 1),
            // A name that the schema does not define: the place is unknown.
            Some(Target::Missing(missing)) => {
                self.unresolved
                    .insert(address(node), (reference, Some(missing)));
                self.missing
                    .get_index(missing)
                    .map(|m| m.0.clone())
                    .unwrap_or_default()
            }
            // A schema where none is expected, under `examples` say, that a `$ref` led to: its
            // base is not known.
            None => {
                self.unresolved.insert(address(node), (reference, None));
                return Ok(Shape::default());
            }
        };
        Ok(Shape {
            references: vec![name],
            ..Shape::default()
        })
    }

    /// What the import did not follow, with the place of each `$ref`.
    fn report(&self) -> LoadReport {
        let mut report = LoadReport::default();
        if self.unresolved.is_empty() {
            return report;
        }
        // The places are looked for only now, as most loads have nothing to report.
        let mut places: HashMap<usize, String> = HashMap::new();
        'documents: for (uri, document) in &self.documents {
            let mut pending = vec![(*document, String::new())];
            while let Some((node, pointer)) = pending.pop() {
                let children: Vec<(&JsonValue, String)> = match node {
                    JsonValue::Object(map) => map
                        .iter()
                        .map(|(key, value)| {
                            let key = key.replace('~', "~0").replace('/', "~1");
                            (value, format!("{pointer}/{key}"))
                        })
                        .collect(),
                    JsonValue::Array(items) => items
                        .iter()
                        .enumerate()
                        .map(|(i, value)| (value, format!("{pointer}/{i}")))
                        .collect(),
                    _ => continue,
                };
                if self.unresolved.contains_key(&address(node)) {
                    places
                        .entry(address(node))
                        .or_insert_with(|| format!("{uri}#{pointer}"));
                    if places.len() == self.unresolved.len() {
                        break 'documents;
                    }
                }
                pending.extend(children);
            }
        }
        for (node, (reference, missing)) in &self.unresolved {
            let (uri, document) = match missing.and_then(|m| self.missing.get_index(m)) {
                Some((uri, document)) => (uri.clone(), document.clone()),
                None => ((*reference).to_owned(), None),
            };
            report.unresolved.push(UnresolvedRef {
                place: places.remove(node).unwrap_or_default(),
                reference: (*reference).to_owned(),
                uri,
                document,
            });
        }
        report
    }

    fn import(&mut self, node: &'d JsonValue, depth: usize) -> Result<Shape, String> {
        if depth > 64 {
            return Err("the schema is nested too deep".into());
        }
        let JsonValue::Object(map) = node else {
            return Ok(Shape::default()); // `true` / `false`: nothing known
        };
        let mut shape = match map.get("$ref").and_then(JsonValue::as_str) {
            Some(reference) => self.follow(node, reference, depth)?,
            None => Shape::default(),
        };
        // The keywords that hold schemas are those of `subschemas`.
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
