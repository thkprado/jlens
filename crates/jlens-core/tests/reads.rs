//! The fields an expression reads (ADR 0016): every case of the official suite gives the same
//! result, or fails with the same error code, on its input and bindings cut by its `Reads`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use jlens_core::reads::Reads;
use jlens_core::{Bindings, Expression, Options};
use serde_json::{Map, Value};

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("listing {dir:?}: {e}"))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    entries
}

struct Case {
    id: String,
    expr: String,
    input: Option<Value>,
    bindings: Map<String, Value>,
    options: Options,
}

fn cases() -> Vec<Case> {
    let suite = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/jsonata-suite");
    let mut datasets = Map::new();
    for path in sorted_entries(&suite.join("datasets")) {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&path).unwrap();
        datasets.insert(name, serde_json::from_str(&text).unwrap());
    }
    let mut cases = Vec::new();
    for group in sorted_entries(&suite.join("groups")) {
        if !group.is_dir() {
            continue;
        }
        for file in sorted_entries(&group) {
            if file.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            // Files serde_json cannot load (lone surrogates) are left out, as by the conformance
            // runner.
            let Ok(spec) = serde_json::from_str::<Value>(&fs::read_to_string(&file).unwrap())
            else {
                continue;
            };
            let id = format!(
                "{}/{}",
                group.file_name().unwrap().to_string_lossy(),
                file.file_stem().unwrap().to_string_lossy()
            );
            let specs = match spec {
                Value::Array(items) => items
                    .into_iter()
                    .enumerate()
                    .map(|(i, item)| (format!("{id}#{i}"), item))
                    .collect(),
                item => vec![(id, item)],
            };
            for (id, spec) in specs {
                let expr = match (&spec["expr"], &spec["expr-file"]) {
                    (Value::String(e), _) => e.clone(),
                    (_, Value::String(f)) => fs::read_to_string(group.join(f)).unwrap(),
                    _ => continue,
                };
                let input = match (spec.get("data"), &spec["dataset"]) {
                    (Some(data), _) => Some(data.clone()),
                    (None, Value::String(name)) => Some(datasets[name].clone()),
                    _ => None,
                };
                let bindings = match &spec["bindings"] {
                    Value::Object(map) => map.clone(),
                    _ => Map::new(),
                };
                let mut options = Options::new().now(1_000_000).seed(1);
                options.timeout = Some(Duration::from_millis(
                    spec["timelimit"].as_u64().unwrap_or(1000),
                ));
                if let Some(depth) = spec["depth"].as_u64() {
                    options.max_depth = Some(depth as usize);
                }
                cases.push(Case {
                    id,
                    expr,
                    input,
                    bindings,
                    options,
                });
            }
        }
    }
    cases
}

fn bindings(values: &Map<String, Value>, reads: Option<&Reads>) -> Bindings {
    let mut bindings = Bindings::new();
    for (name, value) in values {
        match reads {
            None => bindings = bindings.var(name.clone(), value.clone()),
            Some(reads) if reads.every_binding() => {
                bindings = bindings.var(name.clone(), value.clone());
            }
            Some(reads) => {
                if let Some(read) = reads.binding(name) {
                    bindings = bindings.var(name.clone(), read.cut(value));
                }
            }
        }
    }
    bindings
}

/// The outcome of an evaluation as compared: the result, or the error code.
fn outcome(result: jlens_core::Result<Option<Value>>) -> Option<String> {
    match result {
        Ok(value) => Some(format!("{value:?}")),
        // A timeout depends on the machine, not on the input.
        Err(e) if e.code() == "D1012" => None,
        Err(e) => Some(e.code().to_owned()),
    }
}

#[test]
fn every_suite_case_gives_its_result_on_the_fields_it_reads() {
    let cases = cases();
    assert!(cases.len() > 1000, "{} cases", cases.len());
    let mut failures = Vec::new();
    let mut compared = 0;
    // Cases whose input the cut makes smaller: an analysis that read everything would pass too.
    let mut smaller = 0;
    for case in &cases {
        let Ok(expr) = Expression::compile(&case.expr) else {
            continue;
        };
        let reads = expr.reads();
        let whole = expr.evaluate_with(
            case.input.as_ref(),
            &bindings(&case.bindings, None),
            &case.options,
        );
        let cut_input = match (reads.input(), &case.input) {
            (Some(read), Some(input)) => Some(read.cut(input)),
            _ => None,
        };
        let cut = expr.evaluate_with(
            cut_input.as_ref(),
            &bindings(&case.bindings, Some(&reads)),
            &case.options,
        );
        if let (Some(whole), Some(cut)) = (outcome(whole), outcome(cut)) {
            compared += 1;
            if cut_input.is_some() && cut_input != case.input {
                smaller += 1;
            }
            if whole != cut {
                failures.push(format!(
                    "{}: {}\n  reads: {}\n  whole: {whole}\n  cut:   {cut}",
                    case.id,
                    case.expr,
                    serde_json::to_string(&reads).unwrap()
                ));
            }
        }
    }
    assert!(compared > 1000, "{compared} cases compared");
    assert!(smaller > 350, "{smaller} inputs made smaller");
    assert!(
        failures.is_empty(),
        "{} of {compared} cases differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
