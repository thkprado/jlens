//! Criterion benchmarks: `cargo bench -p jlens-core`.
//!
//! The workload (`benches/cases.json` at the repository root) is shared with the comparison
//! against jsonata-js (`benches/compare`), so the numbers can be put side by side.
//!
//! Groups:
//! - `compile`: parsing and post-processing;
//! - `evaluate`: evaluating each case against a 200-item document;
//! - `document`: converting that document (with and without dropping it), and
//!   `evaluate_document`: the cases against it converted once;
//! - `audit`: the shipping rule without a tracer, and with audits at each level.

use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use jlens_core::audit::AuditOptions;
use jlens_core::{Bindings, Document, Expression, Options};
use serde_json::Value;

fn benches_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benches")
}

/// The cases of `benches/cases.json`, as `(name, source)`, and the input document.
fn workload() -> (Vec<(String, String)>, Value) {
    let dir = benches_dir();
    let spec: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("cases.json")).unwrap()).unwrap();
    let input = serde_json::from_str(
        &fs::read_to_string(dir.join(spec["input"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    let cases = spec["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            let source = match (case.get("expr"), case.get("file")) {
                (Some(expr), _) => expr.as_str().unwrap().to_owned(),
                (None, Some(file)) => fs::read_to_string(dir.join(file.as_str().unwrap())).unwrap(),
                _ => panic!("a case needs `expr` or `file`"),
            };
            (case["name"].as_str().unwrap().to_owned(), source)
        })
        .collect();
    (cases, input)
}

fn compile(c: &mut Criterion) {
    let (cases, _) = workload();
    let mut group = c.benchmark_group("compile");
    for (name, source) in cases
        .iter()
        .filter(|(n, _)| ["path", "group-by", "shipping rule"].contains(&n.as_str()))
    {
        group.bench_function(name.as_str(), |b| {
            b.iter(|| Expression::compile(black_box(source)).unwrap())
        });
    }
    group.finish();
}

fn evaluate(c: &mut Criterion) {
    let (cases, input) = workload();
    let bindings = Bindings::new();
    let mut group = c.benchmark_group("evaluate");
    for (name, source) in &cases {
        let expr = Expression::compile(source).unwrap();
        // Every case must succeed, or the benchmark measures an error path.
        expr.evaluate(Some(&input), &bindings).unwrap();
        group.bench_function(name.as_str(), |b| {
            b.iter(|| expr.evaluate(Some(black_box(&input)), &bindings).unwrap())
        });
    }
    group.finish();
}

fn document(c: &mut Criterion) {
    let (cases, input) = workload();
    let text = serde_json::to_string(&input).unwrap();
    let mut group = c.benchmark_group("document");
    group.bench_function("Document::parse", |b| {
        b.iter(|| Document::parse(black_box(&text)).unwrap())
    });
    group.bench_function("Document::new", |b| {
        b.iter(|| Document::new(black_box(&input)))
    });
    // The two steps of `Document::new`: converting, then dropping the converted document.
    group.bench_function("Document::new without drop", |b| {
        b.iter_with_large_drop(|| Document::new(black_box(&input)))
    });
    group.bench_function("serde_json::from_str", |b| {
        b.iter(|| serde_json::from_str::<Value>(black_box(&text)).unwrap())
    });
    group.finish();

    // The cases again, against a document converted once.
    let doc = Document::new(&input);
    let bindings = Bindings::new();
    let options = Options::default();
    let mut group = c.benchmark_group("evaluate_document");
    for (name, source) in &cases {
        let expr = Expression::compile(source).unwrap();
        group.bench_function(name.as_str(), |b| {
            b.iter(|| {
                expr.evaluate_document(black_box(&doc), &bindings, &options)
                    .unwrap()
            })
        });
    }
    group.finish();
}

fn audit(c: &mut Criterion) {
    let (cases, input) = workload();
    let (_, source) = cases.iter().find(|(n, _)| n == "shipping rule").unwrap();
    let expr = Expression::compile(source).unwrap();
    let bindings = Bindings::new();
    let options = Options::default();
    let mut group = c.benchmark_group("audit");
    group.bench_function("no tracer", |b| {
        b.iter(|| expr.evaluate(Some(black_box(&input)), &bindings).unwrap())
    });
    for (level, audit) in [
        ("decisions", AuditOptions::decisions()),
        ("full", AuditOptions::full()),
    ] {
        group.bench_function(level, |b| {
            b.iter(|| expr.evaluate_audited(Some(black_box(&input)), &bindings, &options, &audit))
        });
    }
    group.finish();
}

fn config() -> Criterion {
    Criterion::default()
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(2))
}

criterion_group! {
    name = benches;
    config = config();
    targets = compile, evaluate, document, audit
}
criterion_main!(benches);
