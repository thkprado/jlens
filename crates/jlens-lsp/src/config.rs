//! The `jlens.json` configuration: which schema, variables and host functions apply to which
//! expression files.

use std::path::{Path, PathBuf};

use glob::{MatchOptions, Pattern};
use jlens_schema::{FunctionDecl, LanguageService, Schema};
use serde::Deserialize;
use serde_json::Value;

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
}

/// One pattern of a configuration, with its settings.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The folder that the pattern and the schema path are relative to.
    pub root: PathBuf,
    pub pattern: Pattern,
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
    pub fn service(&self) -> (LanguageService, Vec<String>) {
        let mut problems = Vec::new();
        let schema = self.settings.schema.as_ref().and_then(|path| {
            let path = self.root.join(path);
            let loaded = std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|text| serde_json::from_str(&text).map_err(|e| e.to_string()))
                .and_then(|doc| Schema::from_json_schema(&doc).map_err(|e| e.to_string()));
            loaded
                .map_err(|e| problems.push(format!("schema {}: {e}", path.display())))
                .ok()
        });
        let mut service = LanguageService::new(schema);
        for (name, doc) in &self.settings.variables {
            let name = name.trim_start_matches('$');
            let shape = match doc {
                Value::Null => None,
                doc => match Schema::from_json_schema(doc) {
                    Ok(schema) => Some(schema.root),
                    Err(e) => {
                        problems.push(format!("variable ${name}: {e}"));
                        None
                    }
                },
            };
            service = service.with_variable(name, shape);
        }
        for decl in &self.settings.functions {
            let mut decl = decl.clone();
            decl.name = decl.name.trim_start_matches('$').to_owned();
            service = service.with_function(decl);
        }
        (service, problems)
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

/// The first entry whose pattern matches `path`.
pub fn find<'e>(entries: &'e [Entry], path: &Path) -> Option<(usize, &'e Entry)> {
    entries.iter().enumerate().find(|(_, e)| e.matches(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        let (service, problems) = entries[0].service();
        assert_eq!(problems.len(), 2, "{problems:?}");
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
