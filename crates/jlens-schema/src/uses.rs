//! What an expression uses of its schema.
//!
//! [`LanguageService::uses`] joins the fields an expression reads
//! ([`jlens_core::reads`], an upper bound) with what the schema says of each: an application
//! that stores the answer beside an expression can tell, when a new version of the schema
//! arrives, whether anything the expression uses changed.

use std::collections::BTreeMap;

use jlens_core::reads::{Read, Reads};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::service::{LanguageService, field_insert};
use crate::shape::{JsonType, Shape};

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
    fn of(shape: &Shape, required: Option<bool>) -> Use {
        Use {
            unknown: false,
            whole: false,
            types: shape.types.clone(),
            enumeration: shape.enumeration.clone(),
            closed: shape.enumeration.is_some()
                && !shape.enumeration_inferred
                && shape.domain.is_none(),
            domain: shape.domain.clone(),
            format: shape.format.clone(),
            required,
        }
    }
}

/// One value to describe: its shape, if the schema has one, whether it is a required field, its
/// path, and what is read of it; `None` under a value read whole, where everything is.
type Pending<'a> = (Option<&'a Shape>, Option<bool>, String, Option<&'a Read>);

/// The paths `read` reaches in a value of `shape`, each with what the schema says there.
fn collect(shape: Option<&Shape>, read: &Read) -> BTreeMap<String, Use> {
    fn join(path: &str, name: &str) -> String {
        let name = field_insert(name);
        if path.is_empty() {
            name
        } else {
            format!("{path}.{name}")
        }
    }
    let mut out = BTreeMap::new();
    // Iterative, so that a deep schema cannot overflow the stack.
    let mut pending: Vec<Pending> = vec![(shape, None, String::new(), Some(read))];
    while let Some((shape, required, path, read)) = pending.pop() {
        let mut entry = match shape {
            Some(shape) => Use::of(shape, required),
            None => Use {
                unknown: true,
                ..Use::default()
            },
        };
        entry.whole = matches!(read, Some(Read::Whole));
        match (read, shape) {
            (Some(Read::Fields(fields)), Some(shape)) => {
                for (name, read) in fields {
                    match shape.fields.get(name) {
                        Some(field) => pending.push((
                            Some(&field.shape),
                            Some(field.required),
                            join(&path, name),
                            Some(read),
                        )),
                        // A field of an array's items is looked for there.
                        None if shape.items.is_some() => {}
                        None => pending.push((None, None, join(&path, name), Some(read))),
                    }
                }
                // A path step maps over arrays: the same fields are read of the items.
                if let Some(items) = shape.items.as_deref().filter(|_| !fields.is_empty()) {
                    pending.push((Some(items), None, format!("{path}[]"), read));
                }
            }
            (Some(Read::Fields(fields)), None) => {
                for (name, read) in fields {
                    pending.push((None, None, join(&path, name), Some(read)));
                }
            }
            (Some(Read::Whole) | None, Some(shape)) => {
                for (name, field) in &shape.fields {
                    pending.push((
                        Some(&field.shape),
                        Some(field.required),
                        join(&path, name),
                        None,
                    ));
                }
                if let Some(items) = &shape.items {
                    pending.push((Some(items), None, format!("{path}[]"), None));
                }
            }
            (Some(Read::Whole) | None, None) => {}
        }
        let key = if path.is_empty() {
            ROOT.to_owned()
        } else {
            path
        };
        out.entry(key).or_insert(entry);
    }
    out
}

impl LanguageService {
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
                .map(|(_, shape)| shape.as_ref())
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
            for (name, shape) in &self.variables {
                if let Some(shape) = shape {
                    bindings
                        .entry(name.clone())
                        .or_insert_with(|| collect(Some(shape), &Read::Whole));
                }
            }
        }
        Uses {
            input: reads
                .input()
                .map(|read| collect(self.schema.as_ref().map(|s| &s.root), read)),
            bindings,
            every_binding: reads.every_binding(),
        }
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
