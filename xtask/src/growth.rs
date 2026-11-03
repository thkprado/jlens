//! `cargo xtask growth`: how the time of each built-in function and operator grows with its input.
//!
//! Each case of `benches/growth.json` runs at the sizes given, each four times the one before,
//! with `evaluate_with` and against a `Document`, and the report gives, for each step between two
//! sizes, the exponent `e` of a time that grows as `n^e`: 1 is linear, 2 quadratic. A case marked
//! `loop` calls something `n` times, and `natural` is quadratic in any implementation (`$reduce`
//! with `$append`). A case with `repeat` builds its expression from a piece written `n` times, and
//! times compiling it as well; `{i}` in the piece is its index. Times are those of the fastest run.
//!
//! The times of `node benches/compare/growth.mjs`, the same cases in jsonata-js, come out in the
//! same format, on the inputs that `--write-inputs` leaves in `target/growth`.

use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use jlens_core::{Bindings, Document, Expression, Options};
use serde_json::{Map, Value, json};

/// The sizes of a run, unless `--sizes` gives others.
const SIZES: [usize; 4] = [1_000, 4_000, 16_000, 64_000];

/// What a size's time is measured for: at least this long, and at least [`MIN_RUNS`] runs.
const MIN_TIME: Duration = Duration::from_millis(40);
const MIN_RUNS: usize = 3;

/// A case that takes longer than this at a size stops there.
const BUDGET: Duration = Duration::from_secs(4);

/// An exponent past this is reported at the end. Linear code reads up to about 1.2 between sizes
/// that leave a cache, and a sort about 1.1.
const MARK: f64 = 1.3;

/// The arguments: `[--case TEXT]... [--sizes N,N] [--write-inputs]`. A debug build is no measure
/// of anything, so that this one builds the release and runs it.
pub fn run(args: &[String]) -> Result<()> {
    if cfg!(debug_assertions) {
        let status = Command::new(env!("CARGO"))
            .args(["run", "--quiet", "--release", "-p", "xtask", "--", "growth"])
            .args(args)
            .status()
            .context("running cargo")?;
        if !status.success() {
            bail!("the release build failed");
        }
        return Ok(());
    }
    let mut sizes = SIZES.to_vec();
    let mut wanted = Vec::new();
    let mut write_inputs = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--case" => wanted.push(args.next().context("--case needs a text")?.clone()),
            "--sizes" => {
                let list = args.next().context("--sizes needs a list")?;
                sizes = list
                    .split(',')
                    .map(|n| n.parse().context("--sizes needs numbers"))
                    .collect::<Result<_>>()?;
            }
            "--write-inputs" => write_inputs = true,
            other => bail!("unknown argument {other:?}"),
        }
    }

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let spec: Value = serde_json::from_str(&fs::read_to_string(root.join("benches/growth.json"))?)?;
    let inputs: Vec<(Value, Document)> = sizes
        .iter()
        .map(|&n| {
            let input = input(n);
            let document = Document::new(&input);
            (input, document)
        })
        .collect();
    if write_inputs {
        let dir = root.join("target/growth");
        fs::create_dir_all(&dir)?;
        for (&n, (input, _)) in sizes.iter().zip(&inputs) {
            fs::write(dir.join(format!("input-{n}.json")), input.to_string())?;
        }
    }

    let header: Vec<String> = sizes.iter().map(usize::to_string).collect();
    println!("case\tmode\t{}\texponents", header.join("\t"));
    let mut marked = Vec::new();
    for case in spec["cases"].as_array().into_iter().flatten() {
        let name = case["name"].as_str().unwrap_or_default();
        if !wanted.is_empty() && !wanted.iter().any(|w| name.contains(w.as_str())) {
            continue;
        }
        for mode in ["evaluate", "document"] {
            let mut times = Vec::new();
            let mut stop = None;
            for (&n, (input, document)) in sizes.iter().zip(&inputs) {
                match measure(case, n, mode, input, document) {
                    Ok(time) => {
                        times.push(time);
                        if time > BUDGET {
                            break;
                        }
                    }
                    Err(code) => {
                        stop = Some(code);
                        break;
                    }
                }
            }
            let exponents = exponents(&sizes, &times);
            let shown: Vec<String> = times.iter().map(|t| micros(*t)).collect();
            let list: Vec<String> = exponents.iter().map(|e| format!("{e:.2}")).collect();
            println!(
                "{name}\t{mode}\t{}\t{}{}",
                pad(shown, sizes.len()),
                list.join(" "),
                stop.map_or(String::new(), |code| format!(" stopped by {code}"))
            );
            if exponents.last().is_some_and(|e| *e > MARK) {
                let mark = case["natural"].as_bool().map_or("", |_| " (natural)");
                marked.push(format!("{name}, {mode}: {}{mark}", list.join(" ")));
            }
        }
    }
    println!("\npast {MARK} at the last step:");
    for line in marked {
        println!("  {line}");
    }
    Ok(())
}

fn pad(mut cells: Vec<String>, len: usize) -> String {
    cells.resize(len, "-".to_owned());
    cells.join("\t")
}

fn micros(time: Duration) -> String {
    format!("{:.1}", time.as_secs_f64() * 1e6)
}

/// The exponent of each step between two sizes measured.
fn exponents(sizes: &[usize], times: &[Duration]) -> Vec<f64> {
    (1..times.len())
        .map(|i| {
            let time = times[i].as_secs_f64() / times[i - 1].as_secs_f64();
            let size = sizes[i] as f64 / sizes[i - 1] as f64;
            time.ln() / size.ln()
        })
        .collect()
}

/// The fastest run of a case, or the code of the error that stopped it.
fn measure(
    case: &Value,
    n: usize,
    mode: &str,
    input: &Value,
    document: &Document,
) -> std::result::Result<Duration, String> {
    let source = source(case, n);
    let bindings = Bindings::new();
    let options = Options::default();
    let built = case["repeat"].is_string();
    let compiled = Expression::compile(&source).map_err(|e| e.code().to_owned());
    let run = |compiled: &Expression| {
        if mode == "evaluate" {
            compiled.evaluate_with(Some(black_box(input)), &bindings, &options)
        } else {
            compiled.evaluate_document(black_box(document), &bindings, &options)
        }
        .map_err(|e| e.code().to_owned())
    };
    let compiled = compiled?;
    let mut fastest = Duration::MAX;
    let mut spent = Duration::ZERO;
    let mut runs = 0;
    while runs < MIN_RUNS || spent < MIN_TIME {
        let start = Instant::now();
        if built {
            let again = Expression::compile(black_box(&source)).map_err(|e| e.code().to_owned())?;
            black_box(run(&again)?);
        } else {
            black_box(run(&compiled)?);
        }
        let time = start.elapsed();
        fastest = fastest.min(time);
        spent += time;
        runs += 1;
        // A run this long says what it can: the next size is what matters.
        if time > BUDGET / 4 {
            break;
        }
    }
    Ok(fastest)
}

/// The expression of a case at a size.
fn source(case: &Value, n: usize) -> String {
    let Some(piece) = case["repeat"].as_str() else {
        return case["expr"].as_str().unwrap_or_default().to_owned();
    };
    let mut text = case["prefix"].as_str().unwrap_or_default().to_owned();
    // A piece nested with each repetition stops at the parser's limit of 1,000 levels, which is
    // what the case is there to show, so that it stays below it.
    let times = if case["nest"].as_bool() == Some(true) {
        n.min(900)
    } else {
        n
    };
    for i in 0..times {
        text.push_str(&piece.replace("{i}", &i.to_string()));
    }
    text.push_str(case["suffix"].as_str().unwrap_or_default());
    text
}

/// A multiplicative walk through `0..n`, so that sorted input does not flatter a sort.
fn scattered(i: usize, n: usize) -> usize {
    i.wrapping_mul(7_919) % n
}

/// The input of size `n`: every array has `n` items, and each text about `10 n` bytes.
fn input(n: usize) -> Value {
    let record = |i: usize| {
        let place = scattered(i, n);
        json!({
            "id": i,
            "key": format!("k{place}"),
            "group": format!("g{}", i % 8),
            "val": place,
            "tags": [i, i + 1],
        })
    };
    let recs: Vec<Value> = (0..n).map(record).collect();
    let mut wide = Map::new();
    for i in 0..n {
        wide.insert(format!("k{i}"), json!(i));
    }
    let repeated = |word: &str| word.repeat(10 * n / word.len());
    json!({
        "n": n,
        "nums": (0..n).map(|i| scattered(i, n)).collect::<Vec<_>>(),
        "rev": (0..n).rev().map(|i| scattered(i, n)).collect::<Vec<_>>(),
        "few": (0..n).map(|i| i % 8).collect::<Vec<_>>(),
        "zeros": vec![0; n],
        "strs": (0..n).map(|i| format!("s{}", scattered(i, n))).collect::<Vec<_>>(),
        "same": (0..n).map(|i| json!({ "a": i })).collect::<Vec<_>>(),
        "singles": (0..n).map(|i| json!({ format!("k{i}"): i })).collect::<Vec<_>>(),
        "pairs": (0..n).map(|i| json!([i, i + 1])).collect::<Vec<_>>(),
        "wraps": (0..n).map(|i| json!({ "rec": { "id": i } })).collect::<Vec<_>>(),
        "recs2": recs.clone(),
        "recs": recs,
        "wide": wide,
        "text": repeated("lorem ipsum dolor "),
        "unicode": repeated("coração ταχύς 日本語 "),
    })
}
