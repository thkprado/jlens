//! The `jlens.json` configuration: which schema, variables and host functions apply to which
//! expression files. `jlens check` reads it too, so that the command line and the editors check
//! files alike.

use std::path::{Path, PathBuf};

use glob::{MatchOptions, Pattern};
use jlens_schema::{FunctionDecl, JsonType, LanguageService, LoadReport, Schema, SchemaLoader};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::uri::uri_of;

/// The name of the configuration file, at the root of a workspace folder.
pub const FILE_NAME: &str = "jlens.json";

/// What applies to the files that match one pattern.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// A JSON Schema of the input documents, relative to the configuration's folder.
    pub schema: Option<PathBuf>,
    /// The variables the expressions are evaluated with: a JSON Schema of each value, or `null`.
    pub variables: serde_json::Map<String, Value>,
    /// The host functions the expressions are evaluated with.
    pub functions: Vec<FunctionDecl>,
    /// The JSON types the result of an expression is expected to have, as the conditions of a
    /// rules engine must give a boolean (`LanguageService::with_result_type`): one type's name,
    /// or an array of them. Empty when none is expected.
    #[serde(deserialize_with = "one_or_more")]
    pub result: Vec<JsonType>,
    /// The path whose items the expressions are evaluated against, as the body of its steps
    /// (`items[]`, `LanguageService::with_context`); none for the document.
    pub context: Option<String>,
}

/// Reads one JSON type's name, or a non-empty array of them.
fn one_or_more<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<JsonType>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    JsonType::one_or_more(&value).map_err(|e| serde::de::Error::custom(format!("result {e}")))
}

/// One pattern of a configuration, with its settings.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The folder that the pattern and the schema path are relative to.
    pub root: PathBuf,
    /// The pattern of the paths, relative to `root`, that the settings apply to.
    pub pattern: Pattern,
    /// What applies to the matching files.
    pub settings: Settings,
}

impl Entry {
    fn matches(&self, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(&self.root) else {
            return false;
        };
        let relative: Vec<String> = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let options = MatchOptions {
            case_sensitive: !cfg!(windows),
            require_literal_separator: true,
            require_literal_leading_dot: false,
        };
        self.pattern.matches_with(&relative.join("/"), options)
    }

    /// The language service of these settings. Problems (a missing schema, an invalid variable
    /// schema) are returned alongside, and leave the corresponding part out.
    pub fn service(&self) -> Made {
        self.settings.service(&self.root)
    }

    /// Declares the variables, host functions, expected result type and context of these
    /// settings to `service`, whatever its schema.
    pub fn declare(&self, service: LanguageService) -> Made {
        self.settings.declare(service, &self.root)
    }
}

/// A language service made of some settings, with what was found on the way.
#[derive(Debug)]
pub struct Made {
    /// The service.
    pub service: LanguageService,
    /// What could not be read, which left a part of the settings out: a missing schema, an
    /// invalid variable schema.
    pub problems: Vec<String>,
    /// What the schemas did not follow ([`notes`]): the service leaves those places unknown.
    pub notes: Vec<String>,
}

/// Reads the JSON Schema in the file `path`. A `$ref` to another document is followed into the
/// file at the same relative path from this one as between the two URIs (`common.json`,
/// `../shared/common.json`), when there is one, and never anywhere else: nothing is fetched.
/// The report names the `$ref`s that were not followed.
pub fn load_schema(path: &Path) -> Result<(Schema, LoadReport), String> {
    let document = read(path)?;
    let uri = uri_of(path).unwrap_or_default();
    let folder = path.parent().unwrap_or(Path::new(""));
    SchemaLoader::new()
        .load_asking(&uri, &document, beside(folder))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Reads the JSON Schema `document`, written in a file of `folder`, as [`load_schema`] reads
/// one: its `$ref`s to files are relative to `folder`.
pub fn load_schema_in(folder: &Path, document: &Value) -> Result<(Schema, LoadReport), String> {
    // A folder's URI ends with a slash, so that a reference is resolved inside it.
    let uri = uri_of(folder).map_or_else(String::new, |uri| format!("{uri}/"));
    SchemaLoader::new().load_asking(&uri, document, beside(folder))
}

/// Gives a load the documents that are files at their relative paths from `folder`.
fn beside(folder: &Path) -> impl FnMut(&str, Option<&str>) -> Result<Option<Value>, String> + '_ {
    move |_, relative| {
        let Some(relative) = relative else {
            return Ok(None);
        };
        let path = folder.join(relative);
        if !path.is_file() {
            return Ok(None);
        }
        read(&path).map(Some)
    }
}

fn read(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// What a load did not follow or read, for a person: a line for each document that was not
/// found, one for each `$ref` to a place its document does not have, and one for each keyword
/// that was not read. `what` names the schema.
pub fn notes(what: &str, report: &LoadReport) -> Vec<String> {
    let mut notes = Vec::new();
    for document in report.documents() {
        let references = report
            .unresolved
            .iter()
            .filter(|u| u.document.as_deref() == Some(document))
            .count();
        notes.push(format!(
            "{what}: {references} $ref(s) to {document} not followed, as the document was not \
             found: nothing is checked under them"
        ));
    }
    for unresolved in report.unresolved.iter().filter(|u| u.document.is_none()) {
        notes.push(format!(
            "{what}: $ref {:?} at {} not followed, as {} is not in its document: nothing is \
             checked under it",
            unresolved.reference, unresolved.place, unresolved.uri
        ));
    }
    for unread in &report.unread {
        let why = match unread.keyword.as_str() {
            "type" => "names what is not a JSON type",
            _ => "is not read",
        };
        notes.push(format!(
            "{what}: {} at {} {why}: nothing is checked under it",
            unread.keyword, unread.place
        ));
    }
    notes
}

impl Settings {
    /// The language service of these settings, whose schema path is relative to `root`.
    /// Problems (a missing schema, an invalid variable schema) are returned alongside, and leave
    /// the corresponding part out. The schema is read as [`load_schema`] reads it.
    pub fn service(&self, root: &Path) -> Made {
        let mut problems = Vec::new();
        let mut notes = Vec::new();
        let schema = self.schema.as_ref().and_then(|path| {
            let path = root.join(path);
            match load_schema(&path) {
                Ok((schema, report)) => {
                    notes = self::notes(&format!("schema {}", path.display()), &report);
                    Some(schema)
                }
                Err(e) => {
                    problems.push(format!("schema {e}"));
                    None
                }
            }
        });
        let mut made = self.declare(LanguageService::new(schema), root);
        problems.append(&mut made.problems);
        notes.append(&mut made.notes);
        Made {
            service: made.service,
            problems,
            notes,
        }
    }

    /// Declares the variables, host functions, expected result type and context of these
    /// settings to `service`, whatever its schema. A `$ref` to a file in a variable's schema is relative to `root`, the
    /// configuration's folder ([`load_schema_in`]). Problems (an invalid variable schema) are
    /// returned alongside.
    pub fn declare(&self, mut service: LanguageService, root: &Path) -> Made {
        let mut problems = Vec::new();
        let mut notes = Vec::new();
        for (name, doc) in &self.variables {
            let name = name.trim_start_matches('$');
            service = match doc {
                Value::Null => service.with_variable(name, None),
                doc => match load_schema_in(root, doc) {
                    Ok((schema, report)) => {
                        notes.extend(self::notes(&format!("variable ${name}"), &report));
                        service.with_variable_schema(name, schema)
                    }
                    Err(e) => {
                        problems.push(format!("variable ${name}: {e}"));
                        service.with_variable(name, None)
                    }
                },
            };
        }
        for decl in &self.functions {
            let mut decl = decl.clone();
            decl.name = decl.name.trim_start_matches('$').to_owned();
            service = service.with_function(decl);
        }
        if !self.result.is_empty() {
            service = service.with_result_type(self.result.iter().copied());
        }
        if let Some(context) = &self.context {
            service = service.with_context(context);
        }
        Made {
            service,
            problems,
            notes,
        }
    }
}

/// Reads a configuration object (`{"<pattern>": {settings}}`) whose paths are relative to `root`.
pub fn parse(value: &Value, root: &Path) -> Result<Vec<Entry>, String> {
    let Value::Object(map) = value else {
        return Err("the configuration must be an object of patterns".into());
    };
    map.iter()
        .map(|(pattern, settings)| {
            Ok(Entry {
                root: root.to_owned(),
                pattern: Pattern::new(pattern).map_err(|e| format!("pattern {pattern:?}: {e}"))?,
                settings: Settings::deserialize(settings)
                    .map_err(|e| format!("pattern {pattern:?}: {e}"))?,
            })
        })
        .collect()
}

/// Reads `jlens.json` in `root`. A missing file is an empty configuration.
pub fn load(root: &Path) -> Result<Vec<Entry>, String> {
    let path = root.join(FILE_NAME);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let value: Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&value, root).map_err(|e| format!("{}: {e}", path.display()))
}

/// The first entry whose pattern matches `path`, with its index.
pub fn find<'e>(entries: &'e [Entry], path: &Path) -> Option<(usize, &'e Entry)> {
    entries.iter().enumerate().find(|(_, e)| e.matches(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_result_type_and_the_context_are_declared() {
        let root = Path::new("/work");
        let entries = parse(
            &json!({
                "conditions/*.jsonata": {"result": "boolean"},
                "items/*.jsonata": {"result": ["number", "string"], "context": "items[]"},
                "*.jsonata": {}
            }),
            root,
        )
        .unwrap();
        let schema = Schema::infer(&[json!({"total": 1, "items": [{"price": 2}]})]);
        let codes = |entry: &Entry, text: &str| -> Vec<String> {
            let service = entry
                .declare(LanguageService::new(Some(schema.clone())))
                .service;
            service
                .diagnostics(text)
                .into_iter()
                .map(|d| d.code)
                .collect()
        };
        assert_eq!(entries[0].settings.result, [JsonType::Boolean]);
        assert_eq!(codes(&entries[0], "total"), ["W0212"]);
        assert_eq!(codes(&entries[0], "total > 1"), [""; 0]);
        assert_eq!(codes(&entries[1], "price"), [""; 0]);
        assert_eq!(codes(&entries[1], "total"), ["W0201", "W0214"]);
        assert_eq!(codes(&entries[1], "price > 1"), ["W0212"]);
        assert_eq!(codes(&entries[2], "total"), [""; 0]);
    }

    #[test]
    fn patterns_match_paths_relative_to_the_root() {
        let root = Path::new("/work");
        let entries = parse(
            &json!({
                "rules/*.jsonata": {"variables": {"minimum": {"type": "number"}}},
                "**/*.jsonata": {}
            }),
            root,
        )
        .unwrap();
        let index = |p: &str| find(&entries, &root.join(p)).map(|(i, _)| i);
        assert_eq!(index("rules/a.jsonata"), Some(0));
        assert_eq!(index("rules/deep/a.jsonata"), Some(1));
        assert_eq!(index("a.jsonata"), Some(1));
        assert_eq!(index("a.json"), None);
        assert_eq!(
            find(&entries, Path::new("/elsewhere/a.jsonata")).map(|(i, _)| i),
            None
        );
    }

    #[test]
    fn invalid_configurations_are_reported() {
        let root = Path::new("/work");
        assert!(parse(&json!([]), root).is_err());
        let err = parse(&json!({"*.jsonata": {"schemas": "x"}}), root).unwrap_err();
        assert!(err.contains("schemas"), "{err}");
        for result in [json!("text"), json!([]), json!(["boolean", 1]), json!(true)] {
            let err = parse(&json!({"*.jsonata": {"result": result}}), root).unwrap_err();
            assert!(err.contains("result"), "{err}");
        }
        assert!(parse(&json!({"[": {}}), root).is_err());
    }

    #[test]
    fn services_report_their_problems() {
        let entries = parse(
            &json!({"*": {
                "schema": "missing.json",
                "variables": {"$limit": {"type": "number"}, "bad": 7, "any": null},
                "functions": [{"name": "$rate", "summary": "A rate"}]
            }}),
            Path::new("/work"),
        )
        .unwrap();
        let Made {
            service,
            problems,
            notes,
        } = entries[0].service();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert_eq!(notes, [""; 0]);
        let labels: Vec<String> = service
            .complete("$", 1)
            .items
            .into_iter()
            .map(|i| i.label)
            .collect();
        for expected in ["$limit", "$bad", "$any", "$rate"] {
            assert!(
                labels.iter().any(|l| l == expected),
                "{expected} in {labels:?}"
            );
        }
    }
}
