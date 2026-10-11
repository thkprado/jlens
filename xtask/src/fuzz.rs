//! `cargo xtask fuzz-seed`: seeds the corpora of the fuzz targets (`fuzz/`) with the cases of
//! the JSONata suite.

use std::collections::BTreeSet;
use std::fs;

use anyhow::{Context, Result};
use serde_json::Value as JsonValue;

/// Writes the expressions of the suite to `fuzz/corpus/parse` and `fuzz/corpus/parse_recover`,
/// and each expression followed by a NUL byte and its input document to `fuzz/corpus/evaluate`
/// (the input format of that target).
pub fn seed() -> Result<()> {
    let mut expressions = BTreeSet::new();
    let mut evaluations = BTreeSet::new();
    for case in suite_cases()? {
        let Some(expr) = case.get("expr").and_then(JsonValue::as_str) else {
            continue;
        };
        expressions.insert(expr.as_bytes().to_vec());
        let mut evaluation = expr.as_bytes().to_vec();
        if let Some(data) = case.get("data") {
            evaluation.push(0);
            evaluation.extend(serde_json::to_vec(data)?);
        }
        evaluations.insert(evaluation);
    }
    write("parse", &expressions)?;
    write("parse_recover", &expressions)?;
    write("evaluate", &evaluations)
}

fn suite_cases() -> Result<Vec<JsonValue>> {
    let mut cases = Vec::new();
    let groups = super::root().join("tests/jsonata-suite/groups");
    for group in fs::read_dir(&groups).with_context(|| format!("reading {}", groups.display()))? {
        for file in fs::read_dir(group?.path())? {
            let path = file?.path();
            if path.extension().is_some_and(|e| e == "json") {
                // A few cases hold lone surrogates, which serde_json rejects: skip them.
                match serde_json::from_str(&fs::read_to_string(&path)?) {
                    Ok(JsonValue::Array(items)) => cases.extend(items),
                    Ok(case) => cases.push(case),
                    Err(_) => {}
                }
            }
        }
    }
    Ok(cases)
}

fn write(target: &str, inputs: &BTreeSet<Vec<u8>>) -> Result<()> {
    let dir = super::root().join("fuzz/corpus").join(target);
    fs::create_dir_all(&dir)?;
    for (i, input) in inputs.iter().enumerate() {
        fs::write(dir.join(format!("suite-{i:04}")), input)?;
    }
    println!("{target}: {} inputs in fuzz/corpus/{target}", inputs.len());
    Ok(())
}
