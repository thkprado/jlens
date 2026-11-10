//! What changed for an expression between two schemas.
//!
//! [`Uses::changes`] compares two answers of
//! [`LanguageService::uses`](crate::LanguageService::uses), that of the schema an expression was
//! written against and that of a new version: the paths added, removed and changed, and what
//! changed in each.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::shape::JsonType;
use crate::uses::{Use, Uses};

/// What changed in what an expression uses between two schemas, given by [`Uses::changes`].
///
/// Its JSON form is `{"input": PathChanges, "bindings": {name: PathChanges}, "everyBinding":
/// Change}`, each member left out when nothing changed there: `{}` says that nothing the
/// expression uses changed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsesChanges {
    /// What changed in what the expression uses of its input.
    #[serde(default, skip_serializing_if = "PathChanges::is_empty")]
    pub input: PathChanges,
    /// What changed in what it uses of each variable, by name; only the variables with a
    /// change. A variable that only one answer has comes with all its paths added or removed.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bindings: BTreeMap<String, PathChanges>,
    /// [`Uses::every_binding`], when the two answers differ in it, which those of one
    /// expression do not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_binding: Option<Change<bool>>,
}

impl UsesChanges {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.input.is_empty() && self.bindings.is_empty() && self.every_binding.is_none()
    }
}

/// What changed in the paths an expression reads of one value (see [`UsesChanges`]). Each
/// member is left out of the JSON form when it is empty.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathChanges {
    /// The paths that only the new answer has, with what the new schema says there: a field
    /// added under a value the expression reads whole.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub added: BTreeMap<String, Use>,
    /// The paths that only the old answer has, with what the old schema said there.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub removed: BTreeMap<String, Use>,
    /// The paths that both answers have, and of which the schemas do not say the same.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub changed: BTreeMap<String, UseChanges>,
}

impl PathChanges {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    fn between(
        old: Option<&BTreeMap<String, Use>>,
        new: Option<&BTreeMap<String, Use>>,
    ) -> PathChanges {
        let mut changes = PathChanges::default();
        for (path, before) in old.into_iter().flatten() {
            match new.and_then(|new| new.get(path)) {
                Some(after) => {
                    if let Some(change) = UseChanges::between(before, after) {
                        changes.changed.insert(path.clone(), change);
                    }
                }
                None => {
                    changes.removed.insert(path.clone(), before.clone());
                }
            }
        }
        for (path, after) in new.into_iter().flatten() {
            if !old.is_some_and(|old| old.contains_key(path)) {
                changes.added.insert(path.clone(), after.clone());
            }
        }
        changes
    }
}

/// What changed in what the schemas say of one path (see [`PathChanges`]): a member for each
/// one of [`Use`] that changed, under the same name in the JSON form.
///
/// A field that the new schema no longer declares has `unknown` changed, with every member the
/// old schema gave it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UseChanges {
    /// [`Use::unknown`]: the schema started or stopped declaring the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown: Option<Change<bool>>,
    /// [`Use::whole`], which the two answers of one expression do not differ in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub whole: Option<Change<bool>>,
    /// [`Use::types`], compared as sets; a side that gives no type is left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<Change<Vec<JsonType>>>,
    /// [`Use::enumeration`], compared as a set.
    #[serde(rename = "enum", default, skip_serializing_if = "Option::is_none")]
    pub enumeration: Option<EnumChange>,
    /// [`Use::closed`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<Change<bool>>,
    /// [`Use::domain`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<Change<String>>,
    /// [`Use::format`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Change<String>>,
    /// [`Use::required`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<Change<bool>>,
}

impl UseChanges {
    fn between(old: &Use, new: &Use) -> Option<UseChanges> {
        fn flag(old: bool, new: bool) -> Option<Change<bool>> {
            Change::between(Some(old), Some(new))
        }
        let set = |types: &[JsonType]| types.iter().copied().collect::<BTreeSet<_>>();
        let listed = |types: &[JsonType]| (!types.is_empty()).then(|| types.to_vec());
        let changes = UseChanges {
            unknown: flag(old.unknown, new.unknown),
            whole: flag(old.whole, new.whole),
            types: (set(&old.types) != set(&new.types)).then(|| Change {
                old: listed(&old.types),
                new: listed(&new.types),
            }),
            enumeration: EnumChange::between(
                old.enumeration.as_deref(),
                new.enumeration.as_deref(),
            ),
            closed: flag(old.closed, new.closed),
            domain: Change::between(old.domain.clone(), new.domain.clone()),
            format: Change::between(old.format.clone(), new.format.clone()),
            required: Change::between(old.required, new.required),
        };
        (changes != UseChanges::default()).then_some(changes)
    }
}

/// A member that changed: what the old schema said and what the new one says, each left out
/// of the JSON form when that schema said nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change<T> {
    /// What the old schema said.
    #[serde(default = "none", skip_serializing_if = "Option::is_none")]
    pub old: Option<T>,
    /// What the new schema says.
    #[serde(default = "none", skip_serializing_if = "Option::is_none")]
    pub new: Option<T>,
}

fn none<T>() -> Option<T> {
    None
}

impl<T: PartialEq> Change<T> {
    fn between(old: Option<T>, new: Option<T>) -> Option<Change<T>> {
        (old != new).then_some(Change { old, new })
    }
}

/// An enumeration that changed (see [`UseChanges::enumeration`]). The two are compared as
/// sets: the order of the values and a value listed twice change nothing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnumChange {
    /// The values the old schema listed; `None` when it had no enumeration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<Vec<JsonValue>>,
    /// The values the new schema lists; `None` when it has no enumeration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<Vec<JsonValue>>,
    /// The values that only the new schema lists, in its order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<JsonValue>,
    /// The values that only the old schema listed, in its order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<JsonValue>,
}

impl EnumChange {
    fn between(old: Option<&[JsonValue]>, new: Option<&[JsonValue]>) -> Option<EnumChange> {
        if old == new {
            return None;
        }
        let added = missing(new.unwrap_or_default(), old.unwrap_or_default());
        let removed = missing(old.unwrap_or_default(), new.unwrap_or_default());
        if old.is_some() == new.is_some() && added.is_empty() && removed.is_empty() {
            return None;
        }
        Some(EnumChange {
            old: old.map(<[_]>::to_vec),
            new: new.map(<[_]>::to_vec),
            added,
            removed,
        })
    }
}

/// A set of JSON values. Scalars are found by their text, so that two long enumerations are
/// compared in linear time; arrays and objects, which an enumeration seldom has, one by one.
#[derive(Default)]
struct Values<'a> {
    scalars: HashSet<String>,
    others: Vec<&'a JsonValue>,
}

impl<'a> Values<'a> {
    /// Adds a value; false when the set had it.
    fn insert(&mut self, value: &'a JsonValue) -> bool {
        match value {
            JsonValue::Array(_) | JsonValue::Object(_) => {
                let new = !self.others.contains(&value);
                if new {
                    self.others.push(value);
                }
                new
            }
            scalar => self.scalars.insert(scalar.to_string()),
        }
    }

    fn contains(&self, value: &JsonValue) -> bool {
        match value {
            JsonValue::Array(_) | JsonValue::Object(_) => self.others.contains(&value),
            scalar => self.scalars.contains(&scalar.to_string()),
        }
    }
}

/// The values of `list` that `other` does not have, each once, in the order of `list`.
fn missing(list: &[JsonValue], other: &[JsonValue]) -> Vec<JsonValue> {
    let mut theirs = Values::default();
    for value in other {
        theirs.insert(value);
    }
    let mut seen = Values::default();
    list.iter()
        .filter(|value| !theirs.contains(value) && seen.insert(value))
        .cloned()
        .collect()
}

impl Uses {
    /// What changed between this answer, that of the schema an expression was written against,
    /// and `new`, that of [`LanguageService::uses`](crate::LanguageService::uses) for the same
    /// expression with a new version of the schema: the paths added, removed and changed, and
    /// what changed in each.
    ///
    /// Equal answers have no change. The only answers that differ and have none are those that
    /// list the same types or the same values of an enumeration in another order, as both are
    /// compared as sets. A path is compared with the same path: a field that was renamed is one
    /// path that became unknown, or was removed, and another that was added.
    ///
    /// ```
    /// use jlens_core::Expression;
    /// use jlens_schema::{LanguageService, Schema};
    /// use serde_json::json;
    ///
    /// let schema = |active: &str, types: &[&str]| {
    ///     let schema = Schema::from_json_schema(&json!({
    ///         "type": "object",
    ///         "required": ["active"],
    ///         "properties": {
    ///             "active": {"type": active},
    ///             "type": {"type": "string", "enum": types}
    ///         }
    ///     }));
    ///     LanguageService::new(Some(schema.unwrap()))
    /// };
    /// let reads = Expression::compile(r#"active = true and type = "book""#).unwrap().reads();
    /// let old = schema("boolean", &["book", "toy"]).uses(&reads);
    /// let new = schema("string", &["book", "game", "toy"]).uses(&reads);
    ///
    /// let changes = old.changes(&new);
    /// assert!(!changes.is_empty());
    /// assert_eq!(
    ///     serde_json::to_value(&changes).unwrap(),
    ///     json!({
    ///         "input": {
    ///             "changed": {
    ///                 "active": {"types": {"old": ["boolean"], "new": ["string"]}},
    ///                 "type": {"enum": {
    ///                     "old": ["book", "toy"],
    ///                     "new": ["book", "game", "toy"],
    ///                     "added": ["game"]
    ///                 }}
    ///             }
    ///         }
    ///     })
    /// );
    /// assert!(old.changes(&old).is_empty());
    /// ```
    pub fn changes(&self, new: &Uses) -> UsesChanges {
        let mut bindings = BTreeMap::new();
        for name in self.bindings.keys().chain(new.bindings.keys()) {
            if bindings.contains_key(name) {
                continue;
            }
            let changes = PathChanges::between(self.bindings.get(name), new.bindings.get(name));
            if !changes.is_empty() {
                bindings.insert(name.clone(), changes);
            }
        }
        UsesChanges {
            input: PathChanges::between(self.input.as_ref(), new.input.as_ref()),
            bindings,
            every_binding: Change::between(Some(self.every_binding), Some(new.every_binding)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LanguageService, Schema};
    use jlens_core::Expression;
    use serde_json::json;

    fn uses(schema: JsonValue, text: &str) -> Uses {
        let schema = Schema::from_json_schema(&schema).unwrap();
        let reads = Expression::compile(text).unwrap().reads();
        LanguageService::new(Some(schema)).uses(&reads)
    }

    fn order(status: JsonValue, required: &[&str]) -> JsonValue {
        json!({
            "type": "object",
            "required": required,
            "properties": {
                "status": status,
                "customer": {"type": "object", "properties": {"name": {"type": "string"}}}
            }
        })
    }

    /// What changed in `status` for an expression that reads it.
    fn status(old: JsonValue, new: JsonValue) -> JsonValue {
        let text = "status = 'open'";
        let changes = uses(order(old, &[]), text).changes(&uses(order(new, &[]), text));
        let json = serde_json::to_value(&changes).unwrap();
        assert_eq!(
            serde_json::from_value::<UsesChanges>(json.clone()).unwrap(),
            changes
        );
        let input = json["input"].as_object().unwrap();
        assert_eq!(input.keys().collect::<Vec<_>>(), ["changed"]);
        assert_eq!(input["changed"].as_object().unwrap().len(), 1);
        input["changed"]["status"].clone()
    }

    #[test]
    fn a_type_changed() {
        assert_eq!(
            status(json!({"type": "boolean"}), json!({"type": "string"})),
            json!({"types": {"old": ["boolean"], "new": ["string"]}})
        );
        assert_eq!(
            status(
                json!({"type": "string"}),
                json!({"type": ["string", "null"]})
            ),
            json!({"types": {"old": ["string"], "new": ["null", "string"]}})
        );
        // A side that gives no type is left out.
        assert_eq!(
            status(json!({"type": "string"}), json!({})),
            json!({"types": {"old": ["string"]}})
        );
    }

    #[test]
    fn an_enumeration_changed() {
        assert_eq!(
            status(
                json!({"enum": ["open", "closed", 1, [1], {"a": 1}]}),
                json!({"enum": ["closed", "open", "held", "held", 1.5, [2], {"a": 1}]})
            ),
            json!({"enum": {
                "old": ["open", "closed", 1, [1], {"a": 1}],
                "new": ["closed", "open", "held", "held", 1.5, [2], {"a": 1}],
                "added": ["held", 1.5, [2]],
                "removed": [1, [1]]
            }})
        );
        // An enumeration that the schema starts to declare, even an empty one, and one it drops.
        assert_eq!(
            status(
                json!({"type": "string"}),
                json!({"type": "string", "enum": ["open"]})
            ),
            json!({"enum": {"new": ["open"], "added": ["open"]}, "closed": {"old": false, "new": true}})
        );
        assert_eq!(
            status(json!({"enum": []}), json!({})),
            json!({"enum": {"old": []}, "closed": {"old": true, "new": false}})
        );
        // An enumeration that is no longer inferred is closed.
        assert_eq!(
            status(
                json!({"enum": ["open"], "x-jlens-enum-inferred": true}),
                json!({"enum": ["open"]})
            ),
            json!({"closed": {"old": false, "new": true}})
        );
    }

    #[test]
    fn the_order_of_an_enumeration_is_no_change() {
        let text = "status";
        let old = uses(order(json!({"enum": ["a", "b", 1]}), &[]), text);
        let new = uses(order(json!({"enum": [1, "b", "a", "a"]}), &[]), text);
        assert_ne!(old, new);
        assert!(old.changes(&new).is_empty());
        assert_eq!(serde_json::to_value(old.changes(&new)).unwrap(), json!({}));
    }

    #[test]
    fn a_domain_and_a_format_changed() {
        assert_eq!(
            status(
                json!({"type": "string", "x-jlens-domain": "states"}),
                json!({"type": "string", "x-jlens-domain": "phases"})
            ),
            json!({"domain": {"old": "states", "new": "phases"}})
        );
        assert_eq!(
            status(
                json!({"type": "string", "enum": ["open"], "x-jlens-domain": "states"}),
                json!({"type": "string", "enum": ["open"], "format": "date"})
            ),
            json!({
                "domain": {"old": "states"},
                "closed": {"old": false, "new": true},
                "format": {"new": "date"}
            })
        );
    }

    #[test]
    fn required_changed_where_it_was_changed() {
        let text = "customer.name";
        let old = uses(order(json!({}), &[]), text);
        let new = uses(order(json!({}), &["customer"]), text);
        assert_eq!(
            serde_json::to_value(old.changes(&new)).unwrap(),
            json!({"input": {"changed": {
                "customer": {"required": {"old": false, "new": true}}
            }}})
        );
        assert_eq!(
            serde_json::to_value(new.changes(&old)).unwrap(),
            json!({"input": {"changed": {
                "customer": {"required": {"old": true, "new": false}}
            }}})
        );
    }

    #[test]
    fn a_field_the_schema_no_longer_declares() {
        let text = "customer.name & status";
        let old = uses(order(json!({"type": "string"}), &["status"]), text);
        let mut dropped = order(json!({}), &[]);
        dropped["properties"]
            .as_object_mut()
            .unwrap()
            .remove("status");
        assert_eq!(
            serde_json::to_value(old.changes(&uses(dropped, text))).unwrap(),
            json!({"input": {"changed": {"status": {
                "unknown": {"old": false, "new": true},
                "types": {"old": ["string"]},
                "required": {"old": true}
            }}}})
        );
    }

    #[test]
    fn paths_added_and_removed_under_a_value_read_whole() {
        let schema = |field: &str| {
            json!({
                "type": "object",
                "properties": {"customer": {"type": "object", "properties": {
                    "name": {"type": "string"},
                    field: {"type": "integer"}
                }}}
            })
        };
        let text = "customer.*";
        let changes = uses(schema("age"), text).changes(&uses(schema("since"), text));
        assert_eq!(
            serde_json::to_value(&changes).unwrap(),
            json!({"input": {
                "added": {"customer.since": {"types": ["integer"], "required": false}},
                "removed": {"customer.age": {"types": ["integer"], "required": false}}
            }})
        );
        // The same field read by name stays, unknown.
        let text = "customer.age";
        let changes = uses(schema("age"), text).changes(&uses(schema("since"), text));
        assert!(changes.input.added.is_empty() && changes.input.removed.is_empty());
        assert_eq!(
            changes.input.changed["customer.age"].unknown,
            Some(Change {
                old: Some(false),
                new: Some(true)
            })
        );
    }

    /// The example of the schema guide.
    #[test]
    fn the_example_of_the_guide() {
        let schema = |price: JsonValue, required: &[&str], types: &[&str], more: bool| {
            let mut schema = json!({
                "type": "object",
                "required": ["items"],
                "properties": {"items": {"type": "array", "items": {
                    "type": "object",
                    "required": required,
                    "properties": {
                        "type": {"type": "string", "enum": types},
                        "price": {"type": price}
                    }
                }}}
            });
            if more {
                schema["properties"]["items"]["items"]["properties"]["weight"] =
                    json!({"type": "number"});
            }
            schema
        };
        let text = r#"items[type = "book"]"#;
        let old = uses(
            schema(json!("number"), &["type"], &["book", "toy"], false),
            text,
        );
        let new = uses(
            schema(
                json!(["integer", "string"]),
                &[],
                &["book", "game", "toy"],
                true,
            ),
            text,
        );
        assert_eq!(
            serde_json::to_value(old.changes(&new)).unwrap(),
            json!({
                "input": {
                    "added": {
                        "items[].weight": {"types": ["number"], "required": false}
                    },
                    "changed": {
                        "items[].price": {
                            "types": {"old": ["number"], "new": ["integer", "string"]}
                        },
                        "items[].type": {
                            "enum": {
                                "old": ["book", "toy"],
                                "new": ["book", "game", "toy"],
                                "added": ["game"]
                            },
                            "required": {"old": true, "new": false}
                        }
                    }
                }
            })
        );
    }

    #[test]
    fn variables_and_answers_of_other_expressions() {
        let shape = |kind: &str| {
            Schema::from_json_schema(&json!({
                "type": "object",
                "properties": {"max": {"type": kind}}
            }))
            .unwrap()
            .root
        };
        let service = |kind: &str| {
            LanguageService::new(None)
                .with_variable("limits", Some(shape(kind)))
                .with_variable("rates", Some(shape("number")))
        };
        let reads = |text: &str| Expression::compile(text).unwrap().reads();
        let text = "$limits.max + $rates.max";
        let old = service("integer").uses(&reads(text));
        let new = service("string").uses(&reads(text));
        assert_eq!(
            serde_json::to_value(old.changes(&new)).unwrap(),
            json!({"bindings": {"limits": {"changed": {
                "max": {"types": {"old": ["integer"], "new": ["string"]}}
            }}}})
        );

        // Two expressions: the input and a variable that only one reads, and `$eval`.
        let other = service("integer").uses(&reads("$eval('1')"));
        let changes = serde_json::to_value(old.changes(&other)).unwrap();
        assert_eq!(changes["everyBinding"], json!({"old": false, "new": true}));
        assert_eq!(changes["input"]["added"]["$"]["whole"], true);
        assert_eq!(
            changes["bindings"]["limits"]["changed"]["$"],
            json!({"whole": {"old": false, "new": true}})
        );
        let one = service("integer").uses(&reads("$limits.max"));
        let changes = old.changes(&one);
        assert_eq!(changes.bindings.len(), 1);
        assert_eq!(changes.bindings["rates"].removed.len(), 2);
        assert_eq!(one.changes(&old).bindings["rates"].added.len(), 2);
    }
}
