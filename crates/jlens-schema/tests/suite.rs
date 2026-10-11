//! No false positives: the expressions of the official suite that evaluate without error get
//! no warning about functions or variables, with a schema inferred from their input and their
//! bindings declared.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use jlens_schema::{LanguageService, Schema};
use serde_json::Value;

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("listing {dir:?}: {e}"))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    entries
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

#[test]
fn suite_expressions_get_no_function_or_variable_warnings() {
    let suite = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/jsonata-suite");
    let mut schemas: BTreeMap<String, Option<Schema>> = BTreeMap::new();
    for path in sorted_entries(&suite.join("datasets")) {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let schema = read_json(&path).map(|data| Schema::infer(&[data]));
        schemas.insert(name, schema);
    }

    let mut checked = 0;
    let mut failures = Vec::new();
    for group in sorted_entries(&suite.join("groups")) {
        for file in sorted_entries(&group) {
            // Files that do not parse (lone surrogates) are not cases here either.
            let Some(spec) = read_json(&file) else {
                continue;
            };
            let cases = match spec {
                Value::Array(items) => items,
                item => vec![item],
            };
            for (i, case) in cases.iter().enumerate() {
                if case.get("code").is_some() || case.get("error").is_some() {
                    continue;
                }
                let expr = match (case.get("expr"), case.get("expr-file")) {
                    (Some(Value::String(e)), _) => e.clone(),
                    (_, Some(Value::String(f))) => fs::read_to_string(group.join(f)).unwrap(),
                    _ => continue,
                };
                let schema = match (case.get("data"), case.get("dataset")) {
                    (Some(data), _) => Some(Schema::infer(std::slice::from_ref(data))),
                    (_, Some(Value::String(name))) => schemas[name].clone(),
                    _ => None,
                };
                let mut service = LanguageService::new(schema);
                if let Some(Value::Object(bindings)) = case.get("bindings") {
                    for (name, value) in bindings {
                        let shape = Schema::infer(std::slice::from_ref(value)).root;
                        service = service.with_variable(name, Some(shape));
                    }
                }
                checked += 1;
                for d in service.diagnostics(&expr) {
                    if ["W0202", "W0203", "W0204", "W0205"].contains(&d.code.as_str()) {
                        failures.push(format!("{}#{i}: {expr}\n  {}", file.display(), d.message));
                    }
                }
            }
        }
    }
    assert!(checked > 1000, "only {checked} expressions found");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
