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
//! | `x-jlens-min-items-inferred` | The `minItems` was inferred from samples: no array seen was empty |
//! | `x-jlens-domain` | An open domain: an identifier the application fetches the values of |
//! | `x-jlens-samples` | (root) Number of sample documents inferred from |

use std::collections::HashMap;

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{Map, Value as JsonValue, json};

use crate::shape::{Definitions, Field, JsonType, OtherFields, Schema, Shape};
use crate::uri::{percent_decode, relative, resolve, split_fragment};

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

    /// Reads a JSON Schema document. Supports `type`, `properties`, `required`,
    /// `patternProperties`, `additionalProperties` ([`Shape::other_fields`]), `items`, `enum`,
    /// `const`, `format`, `minimum`, `maximum`, `minItems`, `title`, `description`, `examples`,
    /// `$ref`, `$id`, `$anchor`, `allOf` (whose branches are merged), and `anyOf`/`oneOf` (whose
    /// values have the shape of one branch or another). Other keywords are ignored.
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

    /// Reads `document` as [`SchemaLoader::load_at`] does, asking `give` for each document a
    /// `$ref` leads to that the loader was not given, and then for those that these refer to.
    /// jlens still fetches nothing: `give` decides where a document is read from.
    ///
    /// `give` gets the document's URI and, when it differs from the URI of `document` (its
    /// `$id`, or else `uri`) only in its path, the path from the folder of the one to the
    /// other (`common.json`, `../shared/common.json`), which is where a file kept beside the
    /// schema's is. It answers with the document, `None` for one it does not have, which the
    /// report then names, or an error, which fails the load. Each URI is asked for once, and no
    /// more than 256 of them.
    ///
    /// ```
    /// use jlens_schema::SchemaLoader;
    /// use serde_json::json;
    ///
    /// let order = json!({
    ///     "$id": "https://example.com/schemas/order.json",
    ///     "properties": {
    ///         "customer": {"$ref": "common.json#/$defs/person"},
    ///         "carrier": {"$ref": "https://example.org/carrier.json"}
    ///     }
    /// });
    /// let mut asked = Vec::new();
    /// let (schema, report) = SchemaLoader::new()
    ///     .load_asking("", &order, |uri, relative| {
    ///         asked.push((uri.to_owned(), relative.map(str::to_owned)));
    ///         Ok((relative == Some("common.json"))
    ///             .then(|| json!({"$defs": {"person": {"type": "object"}}})))
    ///     })
    ///     .unwrap();
    /// assert_eq!(asked[0].1.as_deref(), Some("common.json"));
    /// assert_eq!(asked[1], ("https://example.org/carrier.json".to_owned(), None));
    /// assert_eq!(report.documents(), ["https://example.org/carrier.json"]);
    /// assert!(schema.shape_at("customer").is_some());
    /// ```
    pub fn load_asking(
        &self,
        uri: &str,
        document: &JsonValue,
        mut give: impl FnMut(&str, Option<&str>) -> Result<Option<JsonValue>, String>,
    ) -> Result<(Schema, LoadReport), String> {
        /// The documents one load asks for, at most.
        const ASKED: usize = 256;
        let base = match document.get("$id").and_then(JsonValue::as_str) {
            Some(id) => resolve(uri, split_fragment(id).0),
            None => uri.to_owned(),
        };
        let mut given: Vec<(String, JsonValue)> = Vec::new();
        let mut asked: Vec<String> = Vec::new();
        loop {
            let (schema, report) = {
                let mut loader: SchemaLoader<'_> = self.clone();
                for (uri, document) in &given {
                    loader = loader.document(uri, document);
                }
                loader.load_at(uri, document)?
            };
            let before = given.len();
            for missing in report.documents() {
                if asked.len() == ASKED || asked.iter().any(|uri| uri == missing) {
                    continue;
                }
                asked.push(missing.to_owned());
                if let Some(document) = give(missing, relative(&base, missing).as_deref())? {
                    given.push((missing.to_owned(), document));
                }
            }
            if given.len() == before {
                return Ok((schema, report));
            }
        }
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
    /// The keywords that were not read where leaving them out could give a false warning, in
    /// the order the load met them: the schema that has one says nothing of its place.
    /// Keywords that only narrow what a place may hold (`not`, `maxLength`) are left out
    /// without a word, as they cannot.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unread: Vec<UnreadKeyword>,
}

impl LoadReport {
    /// Whether everything was read.
    pub fn is_empty(&self) -> bool {
        self.unresolved.is_empty() && self.unread.is_empty()
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

/// A keyword that a load did not read: the place of the schema that has it is unknown.
///
/// ```
/// use jlens_schema::SchemaLoader;
/// use serde_json::json;
///
/// let (_, report) = SchemaLoader::new()
///     .load(&json!({"properties": {"code": {"type": ["string", "int"]}}}))
///     .unwrap();
/// assert_eq!(report.unread[0].place, "#/properties/code");
/// assert_eq!(report.unread[0].keyword, "type");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct UnreadKeyword {
    /// Where the schema that has the keyword is, as [`UnresolvedRef::place`] gives it.
    pub place: String,
    /// The keyword: `$dynamicRef` or `$recursiveRef`, which the scope of a validation
    /// resolves, or `type` with a name that is not a JSON type.
    pub keyword: String,
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
    if let Some(n) = shape.min_items.filter(|n| *n > 0) {
        out.insert("minItems".into(), n.into());
        if shape.min_items_inferred {
            out.insert("x-jlens-min-items-inferred".into(), true.into());
        }
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
    if let Some(other) = &shape.other_fields {
        if !other.patterns.is_empty() {
            let patterns: Map<String, JsonValue> = other
                .patterns
                .iter()
                .map(|(pattern, p)| (pattern.clone(), export(p, definitions)))
                .collect();
            out.insert("patternProperties".into(), patterns.into());
        }
        match other.rest.as_deref() {
            None => {
                out.insert("additionalProperties".into(), false.into());
            }
            // Patterns alone leave the other fields unknown when read back.
            Some(rest) if *rest == Shape::default() && !other.patterns.is_empty() => {}
            Some(rest) if *rest == Shape::default() => {
                out.insert("additionalProperties".into(), true.into());
            }
            Some(rest) => {
                out.insert("additionalProperties".into(), export(rest, definitions));
            }
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
    if unread_keyword(map).is_some() {
        return;
    }
    for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(JsonValue::Array(branches)) = map.get(key) {
            out.extend(branches);
        }
    }
    for key in [
        "properties",
        "patternProperties",
        "dependentSchemas",
        "dependencies",
    ] {
        if let Some(JsonValue::Object(properties)) = map.get(key) {
            out.extend(properties.values().filter(|p| p.is_object()));
        }
    }
    let conditional: &[&str] = if map.contains_key("if") {
        &["then", "else"]
    } else {
        &[]
    };
    let others = [
        "additionalProperties",
        "unevaluatedProperties",
        "additionalItems",
        "unevaluatedItems",
    ];
    for key in others.iter().chain(conditional) {
        if let Some(schema @ JsonValue::Object(_)) = map.get(*key) {
            out.push(schema);
        }
    }
    match map.get("items") {
        Some(items @ JsonValue::Object(_)) => out.push(items),
        Some(JsonValue::Array(tuple)) => out.extend(tuple),
        _ => {}
    }
}

/// The keyword of the schema `map` that the load does not read and that leaves its place
/// unknown: a dynamic reference, which the scope of a validation resolves, or a `type` that
/// names what is not a JSON type.
fn unread_keyword(map: &Map<String, JsonValue>) -> Option<&'static str> {
    for keyword in ["$dynamicRef", "$recursiveRef"] {
        if map.get(keyword).is_some_and(JsonValue::is_string) {
            return Some(keyword);
        }
    }
    let known = |t: &JsonValue| t.as_str().and_then(JsonType::from_name).is_some();
    match map.get("type") {
        None => None,
        Some(JsonValue::Array(types)) if types.iter().all(known) => None,
        Some(t) if known(t) => None,
        Some(_) => Some("type"),
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
                            "properties" | "patternProperties" | "dependentSchemas"
                            | "dependencies" | "$defs" | "definitions" => {
                                pending.push((value, base.clone(), false));
                            }
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
    /// The schemas the import met with a keyword it does not read: their addresses, and the
    /// keyword.
    unread: IndexMap<usize, &'static str>,
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
            unread: IndexMap::new(),
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

    /// What the import did not follow or read, with the place of each schema.
    fn report(&self) -> LoadReport {
        let mut report = LoadReport::default();
        if self.unresolved.is_empty() && self.unread.is_empty() {
            return report;
        }
        // The places are looked for only now, as most loads have nothing to report.
        let wanted =
            |node: usize| self.unresolved.contains_key(&node) || self.unread.contains_key(&node);
        let count = self
            .unread
            .keys()
            .filter(|node| !self.unresolved.contains_key(*node))
            .count()
            + self.unresolved.len();
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
                if wanted(address(node)) {
                    places
                        .entry(address(node))
                        .or_insert_with(|| format!("{uri}#{pointer}"));
                    if places.len() == count {
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
                place: places.get(node).cloned().unwrap_or_default(),
                reference: (*reference).to_owned(),
                uri,
                document,
            });
        }
        for (node, keyword) in &self.unread {
            report.unread.push(UnreadKeyword {
                place: places.get(node).cloned().unwrap_or_default(),
                keyword: (*keyword).to_owned(),
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
        // A schema with a keyword the load does not read says nothing of its place, as the
        // rest of it could forbid what the keyword allows.
        if let Some(keyword) = unread_keyword(map) {
            self.unread.entry(address(node)).or_insert(keyword);
            return Ok(Shape::default());
        }
        let mut shape = match map.get("$ref").and_then(JsonValue::as_str) {
            Some(reference) => self.follow(node, reference, depth)?,
            None => Shape::default(),
        };
        // The keywords that hold schemas are those of `subschemas`. The branches of `allOf`
        // all hold, and are merged with the rest; a value of `anyOf` or `oneOf` has the shape of
        // one branch or another.
        if let Some(JsonValue::Array(branches)) = map.get("allOf") {
            for branch in branches {
                let b = self.import(branch, depth + 1)?;
                merge(&mut shape, b);
            }
        }
        for key in ["anyOf", "oneOf"] {
            if let Some(JsonValue::Array(branches)) = map.get(key) {
                let mut alternatives = Vec::with_capacity(branches.len());
                // A branch `false` holds for no value.
                for branch in branches.iter().filter(|b| **b != JsonValue::Bool(false)) {
                    alternatives.push(self.import(branch, depth + 1)?);
                }
                merge(&mut shape, either(alternatives));
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
        let patterns = map.get("patternProperties").and_then(JsonValue::as_object);
        // `unevaluatedProperties` also sees the fields of the branches, which the shape has
        // merged by now.
        let additional = map
            .get("additionalProperties")
            .or_else(|| map.get("unevaluatedProperties"));
        if patterns.is_some() || additional.is_some() {
            let mut other = OtherFields::default();
            for (pattern, p) in patterns.into_iter().flatten() {
                other
                    .patterns
                    .push((pattern.clone(), self.import(p, depth + 1)?));
            }
            other.rest = match additional {
                Some(JsonValue::Bool(false)) => None,
                Some(rest) => Some(Box::new(self.import(rest, depth + 1)?)),
                // A schema of patterns alone says nothing of the other fields.
                None => (!other.patterns.is_empty()).then(Box::default),
            };
            // Another `other_fields` is that of a `$ref` or a branch: these keywords hold
            // beside it, as `properties` do.
            match shape.other_fields.take() {
                Some(mut before) => {
                    union_others(&mut before, other);
                    shape.other_fields = Some(before);
                }
                None => shape.other_fields = Some(Box::new(other)),
            }
            add_type(&mut shape, "object");
        }
        self.import_items(map, &mut shape, depth)?;
        // What holds for some of the values only: the fields it declares may be there.
        let mut conditions = Vec::new();
        if map.contains_key("if") {
            for key in ["then", "else"] {
                if let Some(condition) = map.get(key) {
                    conditions.push(self.import(condition, depth + 1)?);
                }
            }
        }
        // Until draft-07, `dependencies` held both `dependentSchemas` and the lists of
        // `dependentRequired`.
        for key in ["dependentSchemas", "dependencies"] {
            if let Some(JsonValue::Object(schemas)) = map.get(key) {
                for condition in schemas.values().filter(|s| !s.is_array()) {
                    conditions.push(self.import(condition, depth + 1)?);
                }
            }
        }
        add_conditions(&mut shape, conditions);
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
        if let Some(n) = map.get("minItems").and_then(JsonValue::as_u64) {
            shape.min_items = Some(n);
            shape.min_items_inferred = map
                .get("x-jlens-min-items-inferred")
                .and_then(JsonValue::as_bool)
                == Some(true);
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
        // OpenAPI 3.0: `null` besides the types the schema gives.
        if map.get("nullable") == Some(&JsonValue::Bool(true)) && !shape.types.is_empty() {
            add_type(&mut shape, "null");
            if let Some(values) = &mut shape.enumeration
                && !values.contains(&JsonValue::Null)
            {
                values.push(JsonValue::Null);
            }
        }
        Ok(shape)
    }

    /// Reads the items of the array schema `map` into `shape`: one schema for every item
    /// (`items`), or one for each of the first items (`prefixItems`, or `items` as an array
    /// until draft-07) and one for the rest (`items` after `prefixItems`, `additionalItems`).
    /// The first items and the rest are merged, as a step over the array takes them all.
    fn import_items(
        &mut self,
        map: &'d Map<String, JsonValue>,
        shape: &mut Shape,
        depth: usize,
    ) -> Result<(), String> {
        let (tuple, rest) = match (map.get("prefixItems"), map.get("items")) {
            (Some(JsonValue::Array(tuple)), rest) => (Some(tuple), rest),
            (_, Some(JsonValue::Array(tuple))) => (Some(tuple), map.get("additionalItems")),
            (_, items) => (None, items),
        };
        // `unevaluatedItems` is the rest where nothing else is; an item `contains` matches
        // escapes it.
        let rest = rest.or_else(|| {
            (!map.contains_key("contains"))
                .then(|| map.get("unevaluatedItems"))
                .flatten()
        });
        let Some(tuple) = tuple else {
            if let Some(items @ JsonValue::Object(_)) = map.get("items") {
                shape.items = Some(Box::new(self.import(items, depth + 1)?));
                add_type(shape, "array");
            } else if let Some(rest @ JsonValue::Object(_)) = rest {
                // `unevaluatedItems` alone: the items of the branches, or else every item.
                let rest = self.import(rest, depth + 1)?;
                match &mut shape.items {
                    Some(items) => merge(items, rest),
                    None => shape.items = Some(Box::new(rest)),
                }
                add_type(shape, "array");
            }
            return Ok(());
        };
        let mut merged = Shape::default();
        for item in tuple {
            merge(&mut merged, self.import(item, depth + 1)?);
        }
        let closed = map
            .get("maxItems")
            .and_then(JsonValue::as_u64)
            .is_some_and(|n| n <= tuple.len() as u64);
        match rest {
            Some(rest @ JsonValue::Object(_)) => merge(&mut merged, self.import(rest, depth + 1)?),
            Some(JsonValue::Bool(false)) => {}
            // Any item may follow the first ones: what they are is not known.
            _ if !closed => {
                add_type(shape, "array");
                return Ok(());
            }
            _ => {}
        }
        shape.items = Some(Box::new(merged));
        add_type(shape, "array");
        Ok(())
    }
}

/// Adds to `shape` what schemas that hold for some of its values only (`then`, `else`, those
/// of `dependentSchemas`) say they may have besides: the fields they declare, each optional, at
/// any depth, the other fields they open the object to, and the definitions they refer to.
/// What else they say only narrows the values, as any schema beside `shape` does, and is left
/// out.
fn add_conditions(shape: &mut Shape, conditions: Vec<Shape>) {
    if conditions.is_empty() {
        return;
    }
    // An object whose fields were not known stays one whose fields are not known.
    let open = shape.fields.is_empty() && shape.other_fields.is_none();
    let mut added: IndexMap<String, Shape> = IndexMap::new();
    let mut under: IndexMap<String, Vec<Shape>> = IndexMap::new();
    let mut items = Vec::new();
    for condition in conditions {
        for (name, field) in condition.fields {
            if shape.fields.contains_key(&name) {
                under.entry(name).or_default().push(field.shape);
            } else {
                match added.entry(name) {
                    indexmap::map::Entry::Occupied(mut e) => merge(e.get_mut(), field.shape),
                    indexmap::map::Entry::Vacant(e) => {
                        e.insert(field.shape);
                    }
                }
            }
        }
        if let Some(other) = condition.other_fields {
            match &mut shape.other_fields {
                Some(mine) => union_others(mine, *other),
                None if !open => shape.other_fields = Some(other),
                None => {}
            }
        }
        items.extend(condition.items.map(|i| *i));
        for name in condition.references {
            if !shape.references.contains(&name) {
                shape.references.push(name);
            }
        }
    }
    for (name, conditions) in under {
        if let Some(field) = shape.fields.get_mut(&name) {
            add_conditions(&mut field.shape, conditions);
        }
    }
    if let Some(mine) = &mut shape.items {
        add_conditions(mine, items);
    }
    if open && !added.is_empty() {
        shape.other_fields = Some(Box::new(OtherFields {
            patterns: Vec::new(),
            rest: Some(Box::default()),
        }));
    }
    for (name, field_shape) in added {
        shape.fields.insert(
            name,
            Field {
                shape: field_shape,
                required: false,
                presence: 0.0,
            },
        );
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

/// The shape of a value of one of `branches` (`anyOf`, `oneOf`): it has the types of any
/// branch, a field required only where every branch requires it, the field and the items of
/// each branch read the same way, and the values of the enumerations only where every branch
/// has one. A branch that gives no type leaves the type unknown, but for one that lists its
/// values (`const`, `enum`), whose types are theirs, and one that only refers to a definition.
fn either(mut branches: Vec<Shape>) -> Shape {
    if branches.len() == 1 {
        return branches.pop().unwrap_or_default();
    }
    let count = branches.len();
    let mut unknown = branches.is_empty();
    // The values listed, where every branch lists them, a branch of `null` alone listing one.
    let mut values: Option<Vec<JsonValue>> = Some(Vec::new());
    let mut listed = false;
    let mut values_inferred = false;
    // The shape of each field in the branches that have it, and whether all require it.
    let mut fields: IndexMap<String, (Vec<Shape>, bool)> = IndexMap::new();
    // The items of the branches that are arrays; `None` where one of them does not say.
    let mut items: Option<Vec<Shape>> = Some(Vec::new());
    let mut out = Shape::default();
    for mut branch in branches {
        if branch.types.is_empty() {
            match branch.enumeration.clone() {
                Some(values) => {
                    for value in &values {
                        add_type(&mut branch, value_type(value));
                    }
                }
                None if branch.references.is_empty() => unknown = true,
                None => {}
            }
        }
        match (&mut values, branch.enumeration.take()) {
            (Some(all), Some(values)) => {
                listed = true;
                values_inferred |= branch.enumeration_inferred;
                for value in values {
                    if !all.contains(&value) {
                        all.push(value);
                    }
                }
            }
            (Some(all), None) if branch.types == [JsonType::Null] => {
                if !all.contains(&JsonValue::Null) {
                    all.push(JsonValue::Null);
                }
            }
            _ => values = None,
        }
        for (name, field) in &branch.fields {
            let entry = fields
                .entry(name.clone())
                .or_insert_with(|| (Vec::new(), true));
            entry.0.push(field.shape.clone());
            entry.1 &= field.required;
        }
        if branch.types.contains(&JsonType::Array) {
            match (&mut items, &branch.items) {
                (Some(all), Some(i)) => all.push((**i).clone()),
                _ => items = None,
            }
        }
        merge(&mut out, branch);
    }
    // `merge` read the other fields; the fields and items are read again as alternatives.
    for (name, (shapes, required)) in fields {
        let required = required && shapes.len() == count;
        if let Some(field) = out.fields.get_mut(&name) {
            field.shape = either(shapes);
            if field.required && !required {
                field.presence = 0.0;
            }
            field.required = required;
        }
    }
    out.items = items.filter(|i| !i.is_empty()).map(|i| Box::new(either(i)));
    if let Some(values) = values.filter(|_| listed) {
        out.enumeration = Some(values);
        out.enumeration_inferred = values_inferred;
    }
    if unknown {
        out.types.clear();
    }
    out
}

/// The JSON Schema name of the type of `value`.
fn value_type(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "boolean",
        JsonValue::Number(_) => "number",
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "array",
        JsonValue::Object(_) => "object",
    }
}

/// Merges `other` into `shape` (union of types and fields).
fn merge(shape: &mut Shape, other: Shape) {
    // An object of either shape may have a field either may have.
    let open = |s: &Shape| -> Option<Box<OtherFields>> {
        match &s.other_fields {
            None if s.is_object() && s.fields.is_empty() => Some(Box::new(OtherFields {
                patterns: Vec::new(),
                rest: Some(Box::default()),
            })),
            others => others.clone(),
        }
    };
    let (mine, theirs) = (open(shape), open(&other));
    let empty = shape.fields.is_empty() && other.fields.is_empty();
    shape.other_fields = match (mine, theirs) {
        (Some(mut a), Some(b)) => {
            union_others(&mut a, *b);
            Some(a)
        }
        (a, b) => a.or(b),
    };
    if shape.other_fields.is_none() && empty && (shape.is_object() || other.is_object()) {
        // Both had no field at all: an object that has none, still.
        shape.other_fields = Some(Box::default());
    }
    let arrays = shape.types.contains(&JsonType::Array);
    let other_arrays = other.types.contains(&JsonType::Array);
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
    // An array of either shape: the fewer items, and only if both say.
    if !arrays {
        shape.min_items = other.min_items;
        shape.min_items_inferred = other.min_items_inferred;
    } else if other_arrays {
        shape.min_items = shape.min_items.zip(other.min_items).map(|(a, b)| a.min(b));
        shape.min_items_inferred &= shape.min_items.is_some();
        shape.min_items_inferred |= other.min_items_inferred && shape.min_items.is_some();
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

/// Adds the other fields of `other` to those of `shape`: an object may have a field either
/// may have. A pattern of one takes the other's rest too, where that has one.
fn union_others(shape: &mut OtherFields, other: OtherFields) {
    let mut patterns = std::mem::take(&mut shape.patterns);
    for (_, p) in &mut patterns {
        if let Some(rest) = &other.rest {
            merge(p, (**rest).clone());
        }
    }
    for (pattern, mut p) in other.patterns {
        if let Some(rest) = &shape.rest {
            merge(&mut p, (**rest).clone());
        }
        match patterns.iter_mut().find(|(q, _)| *q == pattern) {
            Some((_, q)) => merge(q, p),
            None => patterns.push((pattern, p)),
        }
    }
    shape.patterns = patterns;
    shape.rest = match (shape.rest.take(), other.rest) {
        (Some(mut a), Some(b)) => {
            merge(&mut a, *b);
            Some(a)
        }
        (a, b) => a.or(b),
    };
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
            min_items: self.min_items,
            min_items_inferred: self.min_items_inferred,
            domain: self.domain.clone(),
            examples: self.examples.clone(),
            description: self.description.clone(),
            label: self.label.clone(),
            hidden: self.hidden,
            references: self.references.clone(),
            other_fields: self.other_fields.as_ref().map(|other| {
                Box::new(OtherFields {
                    patterns: (other.patterns.iter())
                        .map(|(pattern, p)| (pattern.clone(), p.without_state()))
                        .collect(),
                    rest: (other.rest.as_ref()).map(|rest| Box::new(rest.without_state())),
                })
            }),
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
        // A `const` gives the type of its value, and `null` alone is one value.
        let status = schema.shape_at("status").unwrap();
        assert_eq!(status.types, [JsonType::Null, JsonType::String]);
        assert_eq!(
            status.enumeration,
            Some(vec![json!("open"), JsonValue::Null])
        );
        assert!(!status.enumeration_inferred);
    }

    #[test]
    fn any_of_and_one_of_are_one_branch_or_another() {
        let object = |fields: JsonValue, required: JsonValue| json!({"type": "object", "properties": fields, "required": required});
        let a = object(
            json!({"id": {"type": "number"}, "a": {"type": "string"}}),
            json!(["id", "a"]),
        );
        let b = object(
            json!({"id": {"type": "string"}, "b": {"type": "string"}}),
            json!(["id", "b"]),
        );
        for key in ["anyOf", "oneOf"] {
            let schema = Schema::from_json_schema(&json!({key: [a, b]})).unwrap();
            let fields = &schema.root.fields;
            // Required by both branches, of the types of either.
            assert!(fields["id"].required, "{key}");
            assert_eq!(
                fields["id"].shape.types,
                [JsonType::Number, JsonType::String]
            );
            // Required by one branch only: a value of the other lacks it.
            assert!(!fields["a"].required && !fields["b"].required, "{key}");
            assert_eq!(fields["a"].presence, 0.0);
        }
        // `allOf` stays as it was: a field one branch requires is always there.
        let schema = Schema::from_json_schema(&json!({"allOf": [a, b]})).unwrap();
        assert!(schema.root.fields["a"].required && schema.root.fields["b"].required);

        // A branch that gives no type leaves the type unknown, even beside a `$ref` branch.
        let types = |doc: JsonValue| Schema::from_json_schema(&doc).unwrap().root.types;
        assert_eq!(types(json!({"anyOf": [{"type": "string"}, {}]})), []);
        assert_eq!(types(json!({"oneOf": [{"type": "string"}, true]})), []);
        assert_eq!(
            types(json!({"anyOf": [{"type": "string"}, {"minLength": 1}]})),
            []
        );
        // But not the types given beside it, which every value has.
        assert_eq!(
            types(json!({"type": "string", "anyOf": [{"minLength": 1}, {"maxLength": 0}]})),
            [JsonType::String]
        );
        // A branch `false` holds for no value.
        assert_eq!(
            types(json!({"anyOf": [{"type": "string"}, false]})),
            [JsonType::String]
        );

        // Enumerations: the values of every branch, where each lists them.
        let at = |doc: JsonValue| Schema::from_json_schema(&doc).unwrap().root;
        let shape = at(json!({"oneOf": [{"const": "a"}, {"enum": ["b", "c"]}]}));
        assert_eq!(shape.types, [JsonType::String]);
        assert_eq!(
            shape.enumeration,
            Some(vec![json!("a"), json!("b"), json!("c")])
        );
        let shape = at(json!({"anyOf": [{"const": "a"}, {"type": "string"}]}));
        assert_eq!(
            (shape.types, shape.enumeration),
            (vec![JsonType::String], None)
        );

        // The fields and the items the branches share are read the same way.
        let shape = at(json!({"anyOf": [
            {"type": "object", "properties": {"k": {"const": "x"}}},
            {"type": "object", "properties": {"k": {"const": "y"}}}
        ]}));
        assert_eq!(
            shape.fields["k"].shape.enumeration,
            Some(vec![json!("x"), json!("y")])
        );
        let shape = at(json!({"anyOf": [
            {"type": "array", "items": object(json!({"p": {"type": "number"}}), json!(["p"]))},
            {"type": "array", "items": object(json!({"q": {"type": "number"}}), json!(["q"]))}
        ]}));
        let items = shape.items.unwrap();
        assert!(!items.fields["p"].required && !items.fields["q"].required);
        // An array whose items a branch does not give has unknown items.
        let shape = at(json!({"anyOf": [
            {"type": "array", "items": {"type": "number"}},
            {"type": "array"}
        ]}));
        assert!(shape.items.is_none());
    }

    #[test]
    fn min_items_declared_and_inferred() {
        // `tags` was empty in a sample, `codes` never.
        let schema = Schema::infer(&[
            json!({"tags": [], "codes": ["a"]}),
            json!({"tags": ["x"], "codes": ["b", "c"]}),
        ]);
        let doc = schema.to_json_schema();
        assert_eq!(doc["properties"]["tags"].get("minItems"), None);
        assert_eq!(doc["properties"]["codes"]["minItems"], 1);
        assert_eq!(
            doc["properties"]["codes"]["x-jlens-min-items-inferred"],
            true
        );
        let back = Schema::from_json_schema(&doc).unwrap();
        assert_eq!(back.root, schema.root.without_state());

        // Declared, in a branch of `anyOf` too, it is not marked as inferred.
        let doc = json!({
            "type": "object",
            "properties": {
                "a": {"type": "array", "minItems": 2},
                "b": {"anyOf": [{"type": "array", "minItems": 1}, {"type": "array", "minItems": 3}]},
                "c": {"anyOf": [{"type": "array", "minItems": 1}, {"type": "array"}]}
            }
        });
        let schema = Schema::from_json_schema(&doc).unwrap();
        let a = schema.shape_at("a").unwrap();
        assert_eq!((a.min_items, a.min_items_inferred), (Some(2), false));
        assert_eq!(schema.shape_at("b").unwrap().min_items, Some(1));
        assert_eq!(schema.shape_at("c").unwrap().min_items, None);
    }

    #[test]
    fn other_fields_of_an_object() {
        let doc = json!({
            "$schema": DIALECT,
            "type": "object",
            "properties": {
                "meta": {"type": "object"},
                "sealed": {"type": "object", "additionalProperties": false},
                "open": {
                    "type": "object",
                    "properties": {"a": {"type": "number"}},
                    "additionalProperties": true
                },
                "labels": {"type": "object", "patternProperties": {"^x-": {"type": "string"}}},
                "tagged": {
                    "type": "object",
                    "patternProperties": {"^x-": {"type": "string"}},
                    "additionalProperties": false
                },
                "totals": {"type": "object", "additionalProperties": {"type": "number"}}
            }
        });
        let schema = Schema::from_json_schema(&doc).unwrap();
        assert_eq!(schema.to_json_schema(), doc);
        let at = |path: &str| schema.shape_at(path).map(|s| s.types.clone());
        assert_eq!(at("meta.any"), Some(vec![]));
        assert_eq!(at("sealed.any"), None);
        assert_eq!(at("open.any"), Some(vec![]));
        assert_eq!(at("labels.x-a"), Some(vec![JsonType::String]));
        assert_eq!(at("labels.any"), Some(vec![]));
        assert_eq!(at("tagged.x-a"), Some(vec![JsonType::String]));
        assert_eq!(at("tagged.any"), None);
        assert_eq!(at("totals.usd"), Some(vec![JsonType::Number]));

        // A pattern that is not a regular expression may match any name.
        let schema = Schema::from_json_schema(&json!({
            "type": "object",
            "patternProperties": {"(": {"type": "string"}},
            "additionalProperties": false
        }))
        .unwrap();
        assert_eq!(schema.root.other_field("a").unwrap().types, []);

        // An object that is a map of itself is a recursive schema.
        let schema = Schema::from_json_schema(&json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "additionalProperties": {"$ref": "#"}
        }))
        .unwrap();
        assert_eq!(schema.definitions.len(), 1);
        let name = schema.shape_at("a.b.name").unwrap();
        assert_eq!(name.types, [JsonType::String]);

        // Branches: an object of either may have a field either may have.
        let schema = Schema::from_json_schema(&json!({"anyOf": [
            {"type": "object", "properties": {"a": {"type": "number"}}},
            {"type": "object", "additionalProperties": {"type": "string"}}
        ]}))
        .unwrap();
        assert_eq!(
            schema.root.other_field("b").unwrap().types,
            [JsonType::String]
        );
        let schema = Schema::from_json_schema(&json!({"anyOf": [
            {"type": "object", "properties": {"a": {"type": "number"}}},
            {"type": "null"}
        ]}))
        .unwrap();
        assert!(schema.root.other_field("b").is_none());
        let schema = Schema::from_json_schema(&json!({"anyOf": [
            {"type": "object", "additionalProperties": false},
            {"type": "null"}
        ]}))
        .unwrap();
        assert!(schema.root.other_field("b").is_none());
        // An inferred object never seen with a field has none known.
        let schema = Schema::infer(&[json!({"meta": {}})]);
        assert_eq!(schema.shape_at("meta.any").unwrap().types, []);
    }
}
