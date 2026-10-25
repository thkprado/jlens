//! The program `cargo xtask ab` builds for each revision it compares, against that revision's
//! `jlens-core`. It is also a module of xtask, so that it keeps compiling against the current
//! API.
//!
//! Usage: `probe <benches dir> <evaluate|document|source> <sample ms> [case]...`. It runs the
//! cases of `cases.json` (those named, or all of them) and prints, for each, its name, a tab and
//! the fastest time of one evaluation in nanoseconds, over five samples of about `sample ms`
//! each.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::process::exit;
use std::rc::Rc;
use std::time::{Duration, Instant};

use jlens_core::input::{Sink, Source, Sources};
use jlens_core::{Bindings, Document, Expression, Options};
use serde_json::Value;

const SAMPLES: usize = 5;
const WARM_UP: Duration = Duration::from_millis(20);

#[derive(Clone, Copy)]
enum Mode {
    /// `evaluate_with` on the `serde_json::Value`.
    Evaluate,
    /// `evaluate_document` on a `Document` converted once.
    Document,
    /// `evaluate_source` on the input read through [`Input`].
    Source,
}

pub fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: probe <benches dir> <evaluate|document|source> <sample ms> [case]...");
        exit(2);
    }
    let dir = Path::new(&args[0]);
    let mode = match args[1].as_str() {
        "evaluate" => Mode::Evaluate,
        "document" => Mode::Document,
        "source" => Mode::Source,
        mode => fail(&format!("unknown mode {mode:?}")),
    };
    let sample = Duration::from_millis(args[2].parse().unwrap_or_else(|_| fail("bad sample")));
    let wanted = &args[3..];

    let spec: Value = read_json(&dir.join("cases.json"));
    // A case reads the workload's input, or its own.
    let default = spec["input"].as_str().unwrap_or_default();
    let mut inputs: BTreeMap<String, (Value, Document)> = BTreeMap::new();
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
        let file = case["input"].as_str().unwrap_or(default);
        let (input, doc) = &*inputs.entry(file.to_owned()).or_insert_with(|| {
            let input = read_json(&dir.join(file));
            let doc = Document::new(&input);
            (input, doc)
        });
        let expr = Expression::compile(&source).unwrap_or_else(|e| fail(&format!("{name}: {e}")));
        let run = || {
            let result = match mode {
                Mode::Evaluate => expr.evaluate_with(Some(black_box(input)), &bindings, &options),
                Mode::Document => expr.evaluate_document(black_box(doc), &bindings, &options),
                Mode::Source => expr.evaluate_source(&Input(black_box(input)), &bindings, &options),
            };
            // Every case must succeed, or the probe measures an error path.
            if let Err(e) = result {
                fail(&format!("{name}: {e}"));
            }
        };
        println!("{name}\t{}", fastest(run, sample));
    }
}

/// The input, read one level at a time as the Python package reads its values: the arrays and
/// objects of a level are members of one [`Level`], made when the first of them is written, and
/// a path step reads one field of an object alone, and an index one item of an array.
struct Input<'j>(&'j Value);

/// The arrays and objects of a level.
#[derive(Default)]
struct Level<'j>(RefCell<Vec<&'j Value>>);

/// Writes `value`; an array or an object becomes a member of `level`.
fn put<'j>(sink: &mut Sink<'_, 'j>, value: &'j Value, level: &mut Option<Rc<Level<'j>>>) {
    match value {
        Value::Null => sink.null(),
        Value::Bool(b) => sink.bool(*b),
        Value::Number(n) => sink.number(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => sink.string(s),
        Value::Array(_) | Value::Object(_) => {
            let level = level.get_or_insert_with(Rc::default);
            let index = {
                let mut members = level.0.borrow_mut();
                members.push(value);
                members.len() as u32 - 1
            };
            if value.is_array() {
                sink.array_of(level, index);
            } else {
                sink.object_of(level, index);
            }
        }
    }
}

impl<'j> Source<'j> for Input<'j> {
    fn read(&self, sink: &mut Sink<'_, 'j>) -> Result<(), String> {
        put(sink, self.0, &mut None);
        Ok(())
    }
}

impl<'j> Sources<'j> for Level<'j> {
    fn read(&self, index: u32, sink: &mut Sink<'_, 'j>) -> Result<(), String> {
        let member = self.0.borrow()[index as usize];
        let mut level = None;
        match member {
            Value::Array(items) => {
                sink.reserve(items.len());
                for item in items {
                    put(sink, item, &mut level);
                }
            }
            Value::Object(fields) => {
                sink.reserve(fields.len());
                for (key, value) in fields {
                    sink.key(key);
                    put(sink, value, &mut level);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn read_field(&self, index: u32, key: &str, sink: &mut Sink<'_, 'j>) -> Result<bool, String> {
        let Value::Object(fields) = self.0.borrow()[index as usize] else {
            return Ok(false);
        };
        if let Some(value) = fields.get(key) {
            put(sink, value, &mut None);
        }
        Ok(true)
    }

    // ab: items alone {
    fn length(&self, index: u32) -> Option<usize> {
        self.0.borrow()[index as usize].as_array().map(Vec::len)
    }

    fn read_item(&self, index: u32, item: usize, sink: &mut Sink<'_, 'j>) -> Result<bool, String> {
        let Value::Array(items) = self.0.borrow()[index as usize] else {
            return Ok(false);
        };
        put(sink, &items[item], &mut None);
        Ok(true)
    }
    // ab: }
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
