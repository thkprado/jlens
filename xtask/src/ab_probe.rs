//! The program `cargo xtask ab` builds for each revision it compares, against that revision's
//! `jlens-core`. It is also a module of xtask, so that it keeps compiling against the current
//! API.
//!
//! Usage: `probe <benches dir> <evaluate|document> <sample ms> [case]...`. It runs the cases of
//! `cases.json` (those named, or all of them) and prints, for each, its name, a tab and the
//! fastest time of one evaluation in nanoseconds, over five samples of about `sample ms` each.

use std::env;
use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::process::exit;
use std::time::{Duration, Instant};

use jlens_core::{Bindings, Document, Expression, Options};
use serde_json::Value;

const SAMPLES: usize = 5;
const WARM_UP: Duration = Duration::from_millis(20);

pub fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: probe <benches dir> <evaluate|document> <sample ms> [case]...");
        exit(2);
    }
    let dir = Path::new(&args[0]);
    let document = match args[1].as_str() {
        "evaluate" => false,
        "document" => true,
        mode => fail(&format!("unknown mode {mode:?}")),
    };
    let sample = Duration::from_millis(args[2].parse().unwrap_or_else(|_| fail("bad sample")));
    let wanted = &args[3..];

    let spec: Value = read_json(&dir.join("cases.json"));
    let input = read_json(&dir.join(spec["input"].as_str().unwrap_or_default()));
    let doc = Document::new(&input);
    let bindings = Bindings::new();
    // `cargo xtask ab --b-max-steps` and `--timeout` replace this line.
    let options = Options::default();
    for case in spec["cases"].as_array().into_iter().flatten() {
        let name = case["name"].as_str().unwrap_or_default();
        if !wanted.is_empty() && !wanted.iter().any(|w| w == name) {
            continue;
        }
        let source = match (case["expr"].as_str(), case["file"].as_str()) {
            (Some(expr), _) => expr.to_owned(),
            (None, Some(file)) => fs::read_to_string(dir.join(file))
                .unwrap_or_else(|e| fail(&format!("reading {file}: {e}"))),
            _ => fail(&format!("case {name:?} needs `expr` or `file`")),
        };
        let expr = Expression::compile(&source).unwrap_or_else(|e| fail(&format!("{name}: {e}")));
        let run = || {
            let result = if document {
                expr.evaluate_document(black_box(&doc), &bindings, &options)
            } else {
                expr.evaluate_with(Some(black_box(&input)), &bindings, &options)
            };
            // Every case must succeed, or the probe measures an error path.
            if let Err(e) = result {
                fail(&format!("{name}: {e}"));
            }
        };
        println!("{name}\t{}", fastest(run, sample));
    }
}

/// The fastest time of one call of `run`, in nanoseconds, over `SAMPLES` batches that each take
/// about `sample`.
fn fastest(run: impl Fn(), sample: Duration) -> u128 {
    let start = Instant::now();
    let mut calls = 0u128;
    while start.elapsed() < WARM_UP {
        run();
        calls += 1;
    }
    let per_call = start.elapsed().as_nanos() / calls;
    let batch = (sample.as_nanos() / per_call.max(1)).max(1);
    (0..SAMPLES)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..batch {
                run();
            }
            start.elapsed().as_nanos() / batch
        })
        .min()
        .unwrap_or_default()
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|e| fail(&format!("reading {}: {e}", path.display())));
    serde_json::from_str(&text).unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())))
}

fn fail(message: &str) -> ! {
    eprintln!("probe: {message}");
    exit(1)
}
