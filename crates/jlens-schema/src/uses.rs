//! What an expression uses of its schema.
//!
//! [`LanguageService::uses`] joins the fields an expression reads
//! ([`jlens_core::reads`], an upper bound) with what the schema says of each: an application
//! that stores the answer beside an expression can tell, when a new version of the schema
//! arrives, whether anything the expression uses changed.

use std::collections::{BTreeMap, HashSet};

use indexmap::IndexMap;
use jlens_core::reads::{Read, Reads};
use jlens_core::{Bindings, Expression, HostFunction};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::service::{LanguageService, field_insert};
use crate::shape::{Definitions, JsonType, Schema, Sh, Shape, members, members_of, summary};

/// The path of a value itself: the input, or a variable.
const ROOT: &str = "$";

/// What an expression uses of the schema of its input and of each variable, given by
/// [`LanguageService::uses`].
///
/// Each value read is a map from paths to what the schema says there ([`Use`]). A path is
/// written as [`Schema::shape_at`](crate::Schema::shape_at) reads it: `$` is the value itself,
/// `items[].type` the field `type` of the items of `items`, and a name that is not a plain one
/// is written between backquotes. Every value on the way to a field read has its own path, and
/// so do the items of an array.
///
/// Its JSON form is `{"input": {path: Use}, "bindings": {name: {path: Use}}, "everyBinding":
/// true}`, without `input` when the expression does not read it and without `everyBinding`
/// when it is false, as that of [`Reads`]. Paths and names are written in order, so that two
/// equal answers have the same text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Uses {
    /// What the expression uses of its input; `None` when it does not read it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<BTreeMap<String, Use>>,
    /// What it uses of each variable it reads, by name (without the `$`). The host functions
    /// declared to the service are left out.
    #[serde(default)]
    pub bindings: BTreeMap<String, BTreeMap<String, Use>>,
    /// Whether the expression may read any variable whole, as `$eval` can
    /// ([`Reads::every_binding`]): every variable declared with a schema is then in `bindings`,
    /// whole.
    #[serde(default, skip_serializing_if = "is_false")]
    pub every_binding: bool,
}

impl Uses {
    /// The answer of its JSON form, as an application keeps it beside an expression; the error
    /// says what is not in the form. Unlike the derived `Deserialize`, which takes an array for
    /// a struct's fields in order, it takes objects only.
    ///
    /// ```
    /// use jlens_schema::Uses;
    /// use serde_json::json;
    ///
    /// let kept = json!({"input": {"total": {"types": ["number"], "required": true}}});
    /// let uses = Uses::from_json(&kept).unwrap();
    /// assert_eq!(uses.input.unwrap()["total"].required, Some(true));
    /// assert!(Uses::from_json(&json!({"input": {"total": []}})).is_err());
    /// ```
    pub fn from_json(json: &JsonValue) -> Result<Uses, String> {
        fn paths(value: Option<&JsonValue>) -> bool {
            value
                .and_then(JsonValue::as_object)
                .is_none_or(|paths| paths.values().all(JsonValue::is_object))
        }
        let Some(members) = json.as_object() else {
            return Err("an answer must be an object".to_owned());
        };
        let bindings = members.get("bindings").and_then(JsonValue::as_object);
        if !paths(members.get("input"))
            || !bindings.is_none_or(|bindings| bindings.values().all(|value| paths(Some(value))))
        {
            return Err("what the schema says of a path must be an object".to_owned());
        }
        Uses::deserialize(json).map_err(|e| e.to_string())
    }
}

/// What the schema says of one value an expression reads (see [`Uses`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Use {
    /// The schema says nothing of this path: there is no schema for the value, or the field
    /// is not one of those it declares. Nothing else is set then, but `whole`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub unknown: bool,
    /// The expression reads the value whole (`*`, `**`, `$eval`, an operand, the result),
    /// not only some of its fields: every path the schema has under this one is listed too.
    #[serde(default, skip_serializing_if = "is_false")]
    pub whole: bool,
    /// The types the schema gives the value; none when it gives none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<JsonType>,
    /// The values the schema lists for it.
    #[serde(rename = "enum", default, skip_serializing_if = "Option::is_none")]
    pub enumeration: Option<Vec<JsonValue>>,
    /// The enumeration is closed: declared, not inferred from samples, and with no open
    /// domain, so that a value outside it is warned about (`W0206`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub closed: bool,
    /// The open domain of the value ([`Shape::domain`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// The string format of the value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Whether the object that holds the field requires it; `None` for a path that is not a
    /// field: the value itself, the items of an array. A field is always there only when the
    /// fields on its way, which have their own paths, are required too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Use {
    /// What the schema says of a value that may have any of `shapes`: the types of all, the
    /// values any lists, and the first domain and format.
    fn of(shapes: &[Sh], required: Option<bool>) -> Use {
        let mut types: Vec<JsonType> = Vec::new();
        let mut enumeration: Option<Vec<JsonValue>> = None;
        for shape in shapes {
            for t in &shape.types {
                if !types.contains(t) {
                    types.push(*t);
                }
            }
            if let Some(listed) = &shape.enumeration {
                // One shape's values are given as it lists them; another's add the new ones.
                let values = enumeration.get_or_insert_with(Vec::new);
                for value in listed {
                    if shapes.len() == 1 || !values.contains(value) {
                        values.push(value.clone());
                    }
                }
            }
        }
        if shapes.len() > 1 {
            types.sort();
        }
        Use {
            unknown: false,
            whole: false,
            types,
            enumeration,
            closed: !shapes.is_empty()
                && shapes.iter().all(|shape| {
                    shape.enumeration.is_some()
                        && !shape.enumeration_inferred
                        && shape.domain.is_none()
                }),
            domain: shapes.iter().find_map(|shape| shape.domain.clone()),
            format: shapes.iter().find_map(|shape| shape.format.clone()),
            required,
        }
    }
}

/// What the schema has at one place: the shapes as it writes them there, before their
/// references are followed, and the shapes they stand for, which tell what the value may be.
#[derive(Clone)]
struct Place<'a> {
    /// The shapes the value may have; `None` when the schema says nothing of it.
    shapes: Option<Vec<Sh<'a>>>,
    /// The shapes written at the place: their own label, description and extensions come
    /// before those of the definitions they refer to.
    written: Vec<Sh<'a>>,
    /// Whether it is a required field; `None` for a path that is not a field.
    required: Option<bool>,
}

/// One value to describe.
struct Pending<'a> {
    place: Place<'a>,
    path: String,
    /// What is read of it; `None` under a value read whole, where everything is.
    read: Option<&'a Read>,
    /// The arrays whose items led here with the same fields to read: an array that holds
    /// itself leads back to them.
    arrays: Vec<*const Shape>,
}

/// The values under those read whole, still to describe: the nearest to the value read whole
/// first, then in the order of their paths, so that where the listing of a recursive schema
/// stops does not depend on the order the schema is written in.
type Below<'a> = BTreeMap<(usize, usize, String), Place<'a>>;

/// The shapes a value of `shape` may have; `None` when a reference leaves them unknown.
fn shapes_of<'a>(shape: &'a Shape, defs: &'a Definitions) -> Option<Vec<Sh<'a>>> {
    match members_of(shape, defs) {
        (shapes, false) => Some(shapes),
        _ => None,
    }
}

/// The field `name` of a value that may have any of `shapes`, required where every shape that
/// has it requires it, and whether the first shape that lists it hides it
/// ([`Shape::hidden`]). `None` when none has it; no shapes when one is open to it with a shape
/// it does not know ([`Shape::other_field`]).
fn field_of<'a>(shapes: &[Sh<'a>], name: &str) -> Option<(Place<'a>, bool)> {
    let mut found: Option<(Place<'a>, bool)> = None;
    let mut hidden = None;
    for shape in shapes {
        let (field, is_required, unknown) = match shape.shape.fields.get(name) {
            Some(field) => {
                hidden.get_or_insert(field.shape.hidden);
                (&field.shape, field.required, false)
            }
            None => match shape.shape.other_field(name) {
                Some(other) => (other, false, *other == Shape::default()),
                None => continue,
            },
        };
        let (place, _) = found.get_or_insert_with(|| {
            let place = Place {
                shapes: Some(Vec::new()),
                written: Vec::new(),
                required: Some(true),
            };
            (place, false)
        });
        place.required = Some(place.required == Some(true) && is_required);
        place.written.push(Sh {
            shape: field,
            defs: shape.defs,
        });
        if unknown
            || place
                .shapes
                .as_mut()
                .is_none_or(|all| members(field, shape.defs, all))
        {
            place.shapes = None;
        }
    }
    found.map(|(place, _)| (place, hidden == Some(true)))
}

/// The items of a value that may have any of `shapes`; `None` when none is an array of known
/// items.
fn items_of<'a>(shapes: &[Sh<'a>]) -> Option<Place<'a>> {
    let mut found: Option<Place<'a>> = None;
    for shape in shapes {
        let Some(items) = shape.shape.items.as_deref() else {
            continue;
        };
        let place = found.get_or_insert_with(|| Place {
            shapes: Some(Vec::new()),
            written: Vec::new(),
            required: None,
        });
        place.written.push(Sh {
            shape: items,
            defs: shape.defs,
        });
        if place
            .shapes
            .as_mut()
            .is_none_or(|all| members(items, shape.defs, all))
        {
            place.shapes = None;
        }
    }
    found
}

/// The paths `read` reaches in a value described by `schema`, each with what the schema has
/// there and whether it is read whole. `listing` leaves out the hidden fields
/// ([`Shape::hidden`]) and what is under them, as completion does.
fn walk<'a>(
    schema: Option<&'a Schema>,
    read: &'a Read,
    listing: bool,
) -> BTreeMap<String, (Place<'a>, bool)> {
    fn join(path: &str, name: &str) -> String {
        let name = field_insert(name);
        if path.is_empty() {
            name
        } else {
            format!("{path}.{name}")
        }
    }
    let unknown = Place {
        shapes: None,
        written: Vec::new(),
        required: None,
    };
    let mut out = BTreeMap::new();
    let root = Place {
        shapes: schema.and_then(|s| shapes_of(&s.root, &s.definitions)),
        written: (schema.iter())
            .map(|s| Sh {
                shape: &s.root,
                defs: &s.definitions,
            })
            .collect(),
        required: None,
    };
    // Iterative, so that a deep schema cannot overflow the stack.
    let mut pending: Vec<Pending> = vec![Pending {
        place: root,
        path: String::new(),
        read: Some(read),
        arrays: Vec::new(),
    }];
    let mut below: Below = BTreeMap::new();
    // The values read whole so far, and the shapes listed under each: a recursive schema
    // leads back to them, and what is under a shape is listed once.
    let mut wholes = 0;
    let mut listed: HashSet<(usize, *const Shape)> = HashSet::new();
    loop {
        let (next, level) = match pending.pop() {
            Some(next) => (next, None),
            None => match below.pop_first() {
                Some(((whole, depth, path), place)) => (
                    Pending {
                        place,
                        path,
                        read: None,
                        arrays: Vec::new(),
                    },
                    Some((whole, depth)),
                ),
                None => break,
            },
        };
        let Pending {
            place,
            path,
            read,
            arrays,
        } = next;
        match (read, &place.shapes) {
            (Some(Read::Fields(fields)), Some(shapes)) => {
                for (name, read) in fields {
                    match field_of(shapes, name) {
                        Some((place, _)) => pending.push(Pending {
                            place,
                            path: join(&path, name),
                            read: Some(read),
                            arrays: Vec::new(),
                        }),
                        // A field of an array's items is looked for there.
                        None if shapes.iter().any(|s| s.items.is_some()) => {}
                        None => pending.push(Pending {
                            place: unknown.clone(),
                            path: join(&path, name),
                            read: Some(read),
                            arrays: Vec::new(),
                        }),
                    }
                }
                // A path step maps over arrays: the same fields are read of the items.
                let mut arrays = arrays;
                let again = shapes
                    .iter()
                    .any(|s| arrays.contains(&(s.shape as *const Shape)));
                if !again && let Some(items) = items_of(shapes).filter(|_| !fields.is_empty()) {
                    arrays.extend(shapes.iter().map(|s| s.shape as *const Shape));
                    pending.push(Pending {
                        place: items,
                        path: format!("{path}[]"),
                        read,
                        arrays,
                    });
                }
            }
            (Some(Read::Fields(fields)), None) => {
                for (name, read) in fields {
                    pending.push(Pending {
                        place: unknown.clone(),
                        path: join(&path, name),
                        read: Some(read),
                        arrays: Vec::new(),
                    });
                }
            }
            (Some(Read::Whole) | None, Some(shapes)) => {
                let (whole, depth) = level.unwrap_or_else(|| {
                    wholes += 1;
                    (wholes, 0)
                });
                // Where the schema repeats, what is under the value is listed already.
                let shapes: Vec<Sh> = shapes
                    .iter()
                    .copied()
                    .filter(|s| listed.insert((whole, s.shape)))
                    .collect();
                let mut names: Vec<&str> = Vec::new();
                for shape in &shapes {
                    for name in shape.shape.fields.keys() {
                        if !names.contains(&name.as_str()) {
                            names.push(name);
                        }
                    }
                }
                for name in names {
                    match field_of(&shapes, name) {
                        Some((_, true)) if listing => {}
                        Some((place, _)) => {
                            below.insert((whole, depth + 1, join(&path, name)), place);
                        }
                        None => {}
                    }
                }
                if let Some(items) = items_of(&shapes) {
                    below.insert((whole, depth + 1, format!("{path}[]")), items);
                }
            }
            (Some(Read::Whole) | None, None) => {}
        }
        let key = if path.is_empty() {
            ROOT.to_owned()
        } else {
            path
        };
        let whole = matches!(read, Some(Read::Whole));
        out.entry(key).or_insert((place, whole));
    }
    out
}

/// A field of a schema, or another value it describes, as [`Schema::fields`] lists it: what
/// completion and hover tell of it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListedField {
    /// The path, written as [`Uses`] writes it: `$` for the value itself, `items[]` for the
    /// items of `items`, a name that is not a plain one between backquotes.
    pub path: String,
    /// The types, as completion and hover give them (`array<object>`, `string | null`);
    /// `unknown` when the schema gives none.
    pub type_summary: String,
    /// The types the schema gives the value; none when it gives none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<JsonType>,
    /// Whether the object that holds the field requires it; `None` for a path that is not a
    /// field: the value itself, the items of an array.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    /// Its short display name (`title`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// What it means.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The values the schema lists for it.
    #[serde(rename = "enum", default, skip_serializing_if = "Option::is_none")]
    pub enumeration: Option<Vec<JsonValue>>,
    /// The enumeration is closed, as [`Use::closed`] tells.
    #[serde(default, skip_serializing_if = "is_false")]
    pub closed: bool,
    /// Its open domain ([`Shape::domain`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Its string format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Its own `x-` keywords, as written ([`Shape::extensions`]): those of its place first,
    /// then those of the definitions it refers to.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub extensions: IndexMap<String, JsonValue>,
}

impl ListedField {
    fn of(path: String, place: &Place) -> ListedField {
        let shapes = place.shapes.as_deref();
        let all = || place.written.iter().chain(shapes.into_iter().flatten());
        let mut field = match shapes {
            Some(shapes) => {
                let described = Use::of(shapes, place.required);
                ListedField {
                    types: described.types,
                    enumeration: described.enumeration,
                    closed: described.closed,
                    domain: described.domain,
                    format: described.format,
                    ..ListedField::default()
                }
            }
            None => ListedField::default(),
        };
        field.path = path;
        field.required = place.required;
        field.label = all().find_map(|s| s.label.clone());
        field.description = all().find_map(|s| s.description.clone());
        for shape in all() {
            for (keyword, value) in &shape.extensions {
                field
                    .extensions
                    .entry(keyword.clone())
                    .or_insert_with(|| value.clone());
            }
        }
        field.type_summary = match shapes {
            None => "unknown".to_owned(),
            Some(_) => {
                let mut parts: Vec<String> = Vec::new();
                for written in &place.written {
                    for part in alternatives(&summary(written.shape, written.defs)) {
                        if !parts.iter().any(|p| p == part) {
                            parts.push(part.to_owned());
                        }
                    }
                }
                parts.join(" | ")
            }
        };
        field
    }
}

/// The alternatives of a type summary (`array<string | null> | null`): split at each ` | `
/// outside `<>`.
fn alternatives(summary: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0_usize, 0);
    for (i, b) in summary.bytes().enumerate() {
        match b {
            b'<' => depth += 1,
            b'>' => depth = depth.saturating_sub(1),
            b'|' if depth == 0 => {
                out.push(summary[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(summary[start..].trim());
    out
}

impl Schema {
    /// The fields of the schema, and the other values it describes, each with what completion
    /// and hover tell of it, in the order of their paths: what an application needs to build
    /// its own list of attributes, or to match the fields of two versions of a schema, without
    /// walking it. Overrides applied to the schema ([`Schema::apply_overrides`]) are in it; a
    /// hidden field ([`Shape::hidden`]) and what is under it are left out.
    ///
    /// The paths are those of [`LanguageService::uses`] for an expression that reads the whole
    /// value: `$` is the value itself, and a recursive schema is listed down to where it
    /// repeats.
    ///
    /// ```
    /// use jlens_schema::Schema;
    /// use serde_json::json;
    ///
    /// let schema = Schema::from_json_schema(&json!({
    ///     "type": "object",
    ///     "required": ["status"],
    ///     "properties": {
    ///         "status": {
    ///             "title": "Status",
    ///             "type": "string",
    ///             "enum": ["open", "closed"],
    ///             "x-attribute-id": 17
    ///         },
    ///         "lines": {"type": "array", "items": {
    ///             "type": "object",
    ///             "properties": {"sku": {"type": "string", "x-jlens-domain": "products"}}
    ///         }},
    ///         "internal": {"type": "string", "x-jlens-hidden": true}
    ///     }
    /// }))
    /// .unwrap();
    /// let fields = schema.fields();
    /// let paths: Vec<&str> = fields.iter().map(|f| f.path.as_str()).collect();
    /// assert_eq!(paths, ["$", "lines", "lines[]", "lines[].sku", "status"]);
    /// assert_eq!(
    ///     serde_json::to_value(&fields[4]).unwrap(),
    ///     json!({
    ///         "path": "status",
    ///         "typeSummary": "string",
    ///         "types": ["string"],
    ///         "required": true,
    ///         "label": "Status",
    ///         "enum": ["open", "closed"],
    ///         "closed": true,
    ///         "extensions": {"x-attribute-id": 17}
    ///     })
    /// );
    /// assert_eq!(fields[3].domain.as_deref(), Some("products"));
    /// assert_eq!(fields[1].type_summary, "array<object>");
    /// ```
    pub fn fields(&self) -> Vec<ListedField> {
        walk(Some(self), &Read::Whole, true)
            .into_iter()
            .map(|(path, (place, _))| ListedField::of(path, &place))
            .collect()
    }
}

/// The paths `read` reaches in a value described by `schema`, each with what it says there.
fn collect(schema: Option<&Schema>, read: &Read) -> BTreeMap<String, Use> {
    walk(schema, read, false)
        .into_iter()
        .map(|(path, (place, whole))| {
            let mut entry = match &place.shapes {
                Some(shapes) => Use::of(shapes, place.required),
                None => Use::default(),
            };
            entry.unknown = place.shapes.is_none();
            entry.whole = whole;
            (path, entry)
        })
        .collect()
}

impl LanguageService {
    /// The fields of the schema of the service's input, as [`Schema::fields`] lists them;
    /// none without a schema.
    pub fn fields(&self) -> Vec<ListedField> {
        self.schema.as_ref().map(Schema::fields).unwrap_or_default()
    }

    /// What an expression uses of the schema of its input and of the variables declared to the
    /// service, from the fields it reads ([`Expression::reads`](jlens_core::Expression::reads),
    /// or `reads_with` when host functions are marked as not reading their context).
    ///
    /// The answer follows the analysis of the reads, which is an upper bound: the fields the
    /// expression names, those of a predicate included, and the whole value where the analysis
    /// cannot follow it (`*`, `**`, `$eval`, a value given to a host function), with every
    /// path the schema has under it. It does not depend on the rest of the schema: two schemas
    /// that give equal answers differ in nothing the expression uses.
    ///
    /// Under a value read whole, a recursive schema is listed down to where it repeats: what
    /// is under a definition is listed once, at the place nearest to the value read whole, and
    /// the path of another value of that definition is the last on its way.
    ///
    /// ```
    /// use jlens_core::Expression;
    /// use jlens_schema::{LanguageService, Schema};
    /// use serde_json::json;
    ///
    /// let schema = Schema::from_json_schema(&json!({
    ///     "type": "object",
    ///     "required": ["items"],
    ///     "properties": {
    ///         "customer": {"type": "object", "properties": {"name": {"type": "string"}}},
    ///         "items": {"type": "array", "items": {
    ///             "type": "object",
    ///             "required": ["type"],
    ///             "properties": {
    ///                 "type": {"type": "string", "enum": ["book", "toy"]},
    ///                 "price": {"type": "number"}
    ///             }
    ///         }}
    ///     }
    /// }))
    /// .unwrap();
    /// let service = LanguageService::new(Some(schema));
    /// let expression = Expression::compile(r#"$sum(items[type = "book"].price)"#).unwrap();
    /// let uses = service.uses(&expression.reads());
    /// assert_eq!(
    ///     serde_json::to_value(&uses).unwrap(),
    ///     json!({
    ///         "input": {
    ///             "$": {"types": ["object"]},
    ///             "items": {"types": ["array"], "required": true},
    ///             "items[]": {"types": ["object"]},
    ///             "items[].price": {"whole": true, "types": ["number"], "required": false},
    ///             "items[].type": {
    ///                 "whole": true, "types": ["string"], "enum": ["book", "toy"],
    ///                 "closed": true, "required": true
    ///             }
    ///         },
    ///         "bindings": {}
    ///     })
    /// );
    /// ```
    pub fn uses(&self, reads: &Reads) -> Uses {
        let declared = |name: &str| {
            self.variables
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, schema)| schema.as_ref())
        };
        let mut bindings: BTreeMap<String, BTreeMap<String, Use>> = reads
            .bindings()
            .iter()
            // A host function is not a value the schemas describe.
            .filter(|(name, _)| {
                declared(name).is_some() || self.functions.iter().all(|f| &f.name != *name)
            })
            .map(|(name, read)| (name.clone(), collect(declared(name).flatten(), read)))
            .collect();
        if reads.every_binding() {
            for (name, schema) in &self.variables {
                if let Some(schema) = schema {
                    bindings
                        .entry(name.clone())
                        .or_insert_with(|| collect(Some(schema), &Read::Whole));
                }
            }
        }
        Uses {
            input: reads
                .input()
                .map(|read| collect(self.schema.as_ref(), read)),
            bindings,
            every_binding: reads.every_binding(),
        }
    }

    /// What `expression` uses of the schemas of the service, as [`uses`](Self::uses) gives it
    /// for the fields the expression reads, with a call to a host function declared to the
    /// service ([`with_function`](Self::with_function)) taken as reading its arguments alone,
    /// and the context that a `-` of its signature may take.
    ///
    /// That is what the host functions of the WebAssembly, Python and Java packages do, which
    /// receive their arguments and nothing else, and in Rust those marked
    /// [`without_input`](HostFunction::without_input). When a declared function reads the
    /// context of its call ([`HostCall::input`](jlens_core::HostCall::input)), give the
    /// expression's own reads to [`uses`](Self::uses): there the call of a function reads its
    /// context whole, so `$describe(items[0])` uses the whole input, and here `items` alone.
    /// A function declared with a signature that is not one is taken that way too.
    ///
    /// ```
    /// use jlens_core::Expression;
    /// use jlens_schema::{FunctionDecl, LanguageService, Schema};
    /// use serde_json::json;
    ///
    /// let schema = Schema::from_json_schema(&json!({
    ///     "type": "object",
    ///     "properties": {
    ///         "items": {"type": "array", "items": {"type": "string"}},
    ///         "total": {"type": "number"}
    ///     }
    /// }))
    /// .unwrap();
    /// let service = LanguageService::new(Some(schema))
    ///     .with_function(FunctionDecl::new("describe", "Describes a value"));
    /// let expression = Expression::compile("$describe(items[0])").unwrap();
    /// let uses = service.uses_of(&expression);
    /// assert_eq!(
    ///     serde_json::to_value(&uses).unwrap(),
    ///     json!({
    ///         "input": {
    ///             "$": {"types": ["object"]},
    ///             "items": {"whole": true, "types": ["array"], "required": false},
    ///             "items[]": {"types": ["string"]}
    ///         },
    ///         "bindings": {}
    ///     })
    /// );
    /// // The reads alone take the call as reading the input whole.
    /// assert!(service.uses(&expression.reads()).input.unwrap().contains_key("total"));
    /// ```
    pub fn uses_of(&self, expression: &Expression) -> Uses {
        self.uses(&expression.reads_with(&self.host_bindings()))
    }

    /// Bindings with a host function for each function declared to the service, marked as
    /// reading its arguments alone, for the analysis of what an expression reads.
    pub(crate) fn host_bindings(&self) -> Bindings {
        let mut bindings = Bindings::new();
        for function in &self.functions {
            let host = match &function.signature {
                Some(signature) => HostFunction::with_signature(signature, |_, _| Ok(None)),
                None => Ok(HostFunction::new(0, |_, _| Ok(None))),
            };
            // A signature that is not one tells nothing of the context: the call reads it.
            if let Ok(host) = host {
                bindings = bindings.function(function.name.clone(), host.without_input());
            }
        }
        bindings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Schema;
    use jlens_core::Expression;
    use serde_json::json;

    fn schema() -> Schema {
        Schema::from_json_schema(&json!({
            "type": "object",
            "required": ["customer"],
            "properties": {
                "customer": {
                    "type": "object",
                    "required": ["login"],
                    "properties": {
                        "login": {"type": "string", "format": "email"},
                        "country": {"type": "string", "x-jlens-domain": "countries"},
                        "odd.name": {"type": ["string", "null"]}
                    }
                },
                "status": {"enum": ["open", "closed"], "x-jlens-enum-inferred": true},
                "items": {"type": "array", "items": {
                    "type": "object",
                    "properties": {
                        "type": {"type": "string", "enum": ["a", "b"]},
                        "tags": {"type": "array", "items": {"type": "array", "items": {
                            "type": "object",
                            "properties": {"name": {"type": "string"}}
                        }}}
                    }
                }}
            }
        }))
        .unwrap()
    }

    fn uses(service: &LanguageService, text: &str) -> JsonValue {
        let reads = Expression::compile(text).unwrap().reads();
        serde_json::to_value(service.uses(&reads)).unwrap()
    }

    #[test]
    fn a_declared_function_reads_its_arguments_alone() {
        use crate::FunctionDecl;
        let service = LanguageService::new(Some(schema()))
            .with_variable("x", None)
            .with_function(FunctionDecl::new("plain", ""))
            .with_function(FunctionDecl::new("context", "").signature("<s-:s>"))
            .with_function(FunctionDecl::new("broken", "").signature("<n<s>>"));
        let of = |text: &str| {
            let expression = Expression::compile(text).unwrap();
            serde_json::to_value(service.uses_of(&expression)).unwrap()
        };
        let paths = |uses: &JsonValue| -> Vec<String> {
            uses["input"]
                .as_object()
                .map(|input| input.keys().cloned().collect())
                .unwrap_or_default()
        };
        // Nothing of the input, where the reads alone take it whole.
        assert_eq!(of("$plain(1)"), json!({"bindings": {}}));
        assert!(uses(&service, "$plain(1)")["input"]["$"]["whole"] == json!(true));
        assert_eq!(
            paths(&of("$plain(customer.login)")),
            ["$", "customer", "customer.login"]
        );
        assert_eq!(
            of("$plain($x)")["bindings"],
            json!({"x": {"$": {"unknown": true, "whole": true}}})
        );
        // A `-` takes the context when the argument is left out: a string here, with no field.
        assert_eq!(paths(&of("status.$context()")), ["$", "status"]);
        assert_eq!(
            of("$context()")["input"],
            json!({"$": {"types": ["object"]}})
        );
        // A signature that is not one, and a function that is not declared, read the context.
        assert_eq!(of("$broken(1)")["input"]["$"]["whole"], json!(true));
        assert_eq!(of("$other(1)")["input"]["$"]["whole"], json!(true));
    }

    fn input(text: &str) -> JsonValue {
        uses(&LanguageService::new(Some(schema())), text)["input"].clone()
    }

    #[test]
    fn the_fields_named_with_those_of_a_predicate() {
        assert_eq!(
            input("items[type = 'a'].tags.name"),
            json!({
                "$": {"types": ["object"]},
                "items": {"types": ["array"], "required": false},
                "items[]": {"types": ["object"]},
                "items[].type": {
                    "whole": true, "types": ["string"], "enum": ["a", "b"], "closed": true,
                    "required": false
                },
                "items[].tags": {"types": ["array"], "required": false},
                "items[].tags[]": {"types": ["array"]},
                "items[].tags[][]": {"types": ["object"]},
                "items[].tags[][].name": {"whole": true, "types": ["string"], "required": false}
            })
        );
    }

    #[test]
    fn what_the_schema_says_of_a_field() {
        let all = input("[customer.login, customer.country, customer.`odd.name`, status]");
        assert_eq!(
            all["customer.login"],
            json!({"whole": true, "types": ["string"], "format": "email", "required": true})
        );
        assert_eq!(
            all["customer.country"],
            json!({"whole": true, "types": ["string"], "domain": "countries", "required": false})
        );
        assert_eq!(
            all["customer.`odd.name`"],
            json!({"whole": true, "types": ["null", "string"], "required": false})
        );
        // An inferred enumeration is not closed.
        assert_eq!(
            all["status"],
            json!({"whole": true, "enum": ["open", "closed"], "required": false})
        );
        assert_eq!(
            all["customer"],
            json!({"types": ["object"], "required": true})
        );
        // Every path is one the schema reads.
        let schema = schema();
        for path in all.as_object().unwrap().keys() {
            assert!(schema.shape_at(path).is_some(), "{path}");
        }
    }

    #[test]
    fn a_field_the_schema_does_not_have() {
        assert_eq!(
            input("customer.Login.x & items.price & status.a"),
            json!({
                "$": {"types": ["object"]},
                "customer": {"types": ["object"], "required": true},
                "customer.Login": {"unknown": true},
                "customer.Login.x": {"unknown": true, "whole": true},
                "items": {"types": ["array"], "required": false},
                "items[]": {"types": ["object"]},
                "items[].price": {"unknown": true, "whole": true},
                "status": {"enum": ["open", "closed"], "required": false},
                "status.a": {"unknown": true, "whole": true}
            })
        );
    }

    #[test]
    fn a_field_an_open_object_may_have() {
        let schema = Schema::from_json_schema(&json!({
            "type": "object",
            "properties": {
                "meta": {"type": "object"},
                "totals": {"type": "object", "additionalProperties": {"type": "number"}}
            }
        }))
        .unwrap();
        let service = LanguageService::new(Some(schema));
        assert_eq!(
            uses(&service, "meta.a & totals.usd")["input"],
            json!({
                "$": {"types": ["object"]},
                "meta": {"types": ["object"], "required": false},
                "meta.a": {"unknown": true, "whole": true},
                "totals": {"types": ["object"], "required": false},
                "totals.usd": {"types": ["number"], "required": false, "whole": true}
            })
        );
    }

    #[test]
    fn a_value_needed_without_its_fields() {
        assert_eq!(
            input("$count(items)"),
            json!({
                "$": {"types": ["object"]},
                "items": {"types": ["array"], "required": false}
            })
        );
    }

    #[test]
    fn a_value_read_whole_lists_what_is_under_it() {
        let under = input("customer.*");
        assert_eq!(
            under.as_object().unwrap().keys().collect::<Vec<_>>(),
            [
                "$",
                "customer",
                "customer.`odd.name`",
                "customer.country",
                "customer.login"
            ]
        );
        assert_eq!(under["customer"]["whole"], true);
        assert_eq!(under["customer.login"].get("whole"), None);

        // `$eval` reads the input whole, and every variable.
        let service = LanguageService::new(Some(schema()))
            .with_variable(
                "limits",
                Some(schema().root.fields["customer"].shape.clone()),
            )
            .with_variable("free", None);
        let all = uses(&service, "$eval('1')");
        assert_eq!(all["everyBinding"], true);
        assert_eq!(all["input"]["$"]["whole"], true);
        assert_eq!(all["input"].as_object().unwrap().len(), 13);
        assert!(all["input"]["items[].tags[][].name"].is_object());
        assert_eq!(all["bindings"]["limits"]["$"]["whole"], true);
        assert_eq!(all["bindings"]["limits"].as_object().unwrap().len(), 4);
        assert_eq!(all["bindings"].get("free"), None);
    }

    #[test]
    fn variables_and_host_functions() {
        let service = LanguageService::new(None)
            .with_variable("order", Some(schema().root))
            .with_variable("free", None)
            .with_function(crate::FunctionDecl::new("rate", ""));
        assert_eq!(
            uses(
                &service,
                "$order.customer.login & $free.a & $other & $rate(1)"
            ),
            json!({
                // The call of a host function reads its context.
                "input": {"$": {"unknown": true, "whole": true}},
                "bindings": {
                    "order": {
                        "$": {"types": ["object"]},
                        "customer": {"types": ["object"], "required": true},
                        "customer.login": {
                            "whole": true, "types": ["string"], "format": "email",
                            "required": true
                        }
                    },
                    "free": {"$": {"unknown": true}, "a": {"unknown": true, "whole": true}},
                    "other": {"$": {"unknown": true, "whole": true}}
                }
            })
        );
    }

    #[test]
    fn the_root_as_an_array_and_the_json_form() {
        let schema = Schema::from_json_schema(&json!({
            "type": "array",
            "items": {"type": "object", "properties": {"a": {"type": "integer"}}}
        }))
        .unwrap();
        let service = LanguageService::new(Some(schema.clone()));
        let reads = Expression::compile("a > 1").unwrap().reads();
        let uses = service.uses(&reads);
        let json = serde_json::to_value(&uses).unwrap();
        assert_eq!(
            json,
            json!({
                "input": {
                    "$": {"types": ["array"]},
                    "[]": {"types": ["object"]},
                    "[].a": {"whole": true, "types": ["integer"], "required": false}
                },
                "bindings": {}
            })
        );
        assert!(schema.shape_at("[].a").is_some());
        assert_eq!(serde_json::from_value::<Uses>(json).unwrap(), uses);
        assert!(serde_json::from_value::<Uses>(json!({"inputs": {}})).is_err());
    }
}
