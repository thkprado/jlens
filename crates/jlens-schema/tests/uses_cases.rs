//! The cases of `fixtures/uses/cases.json`, which every package answers with the same JSON
//! (roadmap 0.33 #7): what each expression uses of the first schema, and what changed for it
//! with the second.

use jlens_core::Expression;
use jlens_schema::{FunctionDecl, LanguageService, Schema, Uses, UsesChanges};
use serde_json::Value;

fn service(schema: &Value, variables: &Value, functions: &Value) -> LanguageService {
    let mut service = LanguageService::new(Some(Schema::from_json_schema(schema).unwrap()));
    for (name, schema) in variables.as_object().unwrap() {
        service = match schema {
            Value::Null => service.with_variable(name, None),
            schema => service.with_variable_schema(name, Schema::from_json_schema(schema).unwrap()),
        };
    }
    let functions: Vec<FunctionDecl> = serde_json::from_value(functions.clone()).unwrap();
    functions
        .into_iter()
        .fold(service, LanguageService::with_function)
}

#[test]
fn the_shared_cases() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/uses/cases.json"
    );
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let old = service(
        &fixture["schema"],
        &fixture["variables"],
        &fixture["functions"],
    );
    let new = service(
        &fixture["newSchema"],
        &fixture["newVariables"],
        &fixture["functions"],
    );
    for case in fixture["cases"].as_array().unwrap() {
        let text = case["expression"].as_str().unwrap();
        let reads = Expression::compile(text).unwrap().reads();
        let uses = old.uses(&reads);
        let changes = uses.changes(&new.uses(&reads));
        assert_eq!(serde_json::to_value(&uses).unwrap(), case["uses"], "{text}");
        assert_eq!(
            serde_json::to_value(&changes).unwrap(),
            case["changes"],
            "{text}"
        );
        // The answers kept as JSON are compared with no service.
        let kept: Uses = serde_json::from_value(case["uses"].clone()).unwrap();
        assert_eq!(kept.changes(&new.uses(&reads)), changes, "{text}");
        let read: UsesChanges = serde_json::from_value(case["changes"].clone()).unwrap();
        assert_eq!(read, changes, "{text}");
    }
}
