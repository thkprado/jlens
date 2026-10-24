//! jlens-regress: the benchmark of the engine on its own (0.22). The cases are in `engine.json`,
//! which `engine.mjs` runs in V8 for reference. Each case finds every match in its text, the
//! way jlens does: `find_from` from the end of the previous match, or, when
//! `JLENS_REGRESS_INTERRUPT` is set at build time, `find_from_interruptible` with an interrupt
//! like jlens's (a deadline and a cancellation flag).
//!
//! Each variant is a binary of its own. In one binary, the two copies of the backtracker that
//! `find_from` and `find_from_interruptible` instantiate are inlined differently: two copies of
//! the same code, both without a check, differ by up to 16 % there. Code placement still moves
//! a binary's numbers by up to 8 % from one change of the crate to the next (ADR 0009), so
//! functions and blocks are aligned, in a target directory of their own:
//!
//! ```sh
//! export RUSTFLAGS="-C llvm-args=-align-all-functions=6 -C llvm-args=-align-all-nofallthru-blocks=5"
//! export CARGO_TARGET_DIR=target/aligned
//! cargo bench -p jlens-regress --bench engine [case...]
//! JLENS_REGRESS_INTERRUPT=1 cargo bench -p jlens-regress --bench engine [case...]
//! ```
//!
//! Arguments select the cases whose name contains one of them. Each run saves its times in
//! `target/aligned/tmp`, and shows what the check costs against the last run of the other
//! variant.

use regress::Regex;
use serde_json::{Map, Value};
use std::hint::black_box;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Whether this binary matches with `find_from_interruptible`.
const INTERRUPTIBLE: bool = option_env!("JLENS_REGRESS_INTERRUPT").is_some();
/// Samples taken of each case.
const ROUNDS: usize = 31;
/// The least duration of one sample.
const SAMPLE: Duration = Duration::from_millis(5);

fn main() {
    let cases: Value = serde_json::from_str(include_str!("engine.json")).unwrap();
    let filters: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with("--"))
        .collect();
    let selected = |name: &str| filters.is_empty() || filters.iter().any(|f| name.contains(f));
    let str_of = |case: &Value, key: &str| case[key].as_str().unwrap().to_owned();

    let (this, other) = if INTERRUPTIBLE {
        ("interruptible", "find_from")
    } else {
        ("find_from", "interruptible")
    };
    let saved = |variant: &str| {
        std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("engine-{variant}.json"))
    };
    let previous: Map<String, Value> = std::fs::read(saved(other))
        .ok()
        .and_then(|json| serde_json::from_slice(&json).ok())
        .unwrap_or_default();
    let mut times = Map::new();

    println!(
        "{:<32} {:>7} {:>13} {:>13} {:>7}",
        "case", "matches", this, other, "check"
    );
    for case in cases["matches"].as_array().unwrap() {
        let name = str_of(case, "name");
        if !selected(&name) {
            continue;
        }
        let text = &cases["texts"][str_of(case, "text")];
        let text = text["repeat"]
            .as_str()
            .unwrap()
            .repeat(text["times"].as_u64().unwrap() as usize);
        let re =
            Regex::with_flags(&str_of(case, "pattern"), str_of(case, "flags").as_str()).unwrap();
        let matches = find_all(&re, &text);
        let time = measure(|| find_all(&re, &text));
        let other_time = previous.get(&name).and_then(Value::as_f64);
        let (base, checked) = if INTERRUPTIBLE {
            (other_time, Some(time))
        } else {
            (Some(time), other_time)
        };
        let cost = match (base, checked) {
            (Some(base), Some(checked)) => format!("{:+.1}%", (checked / base - 1.0) * 100.0),
            _ => String::new(),
        };
        println!(
            "{name:<32} {matches:>7} {:>13} {:>13} {cost:>7}",
            format_time(time),
            other_time.map(format_time).unwrap_or_default(),
        );
        times.insert(name, time.into());
    }
    for case in cases["compiles"].as_array().unwrap() {
        let name = str_of(case, "name");
        if !selected(&name) {
            continue;
        }
        let (pattern, flags) = (str_of(case, "pattern"), str_of(case, "flags"));
        let time = measure(|| {
            black_box(Regex::with_flags(&pattern, flags.as_str()).unwrap());
            1
        });
        println!("{name:<32} {:>7} {:>13}", "", format_time(time));
    }

    let mut all = std::fs::read(saved(this))
        .ok()
        .and_then(|json| serde_json::from_slice::<Map<String, Value>>(&json).ok())
        .unwrap_or_default();
    all.extend(times);
    std::fs::write(saved(this), serde_json::to_vec_pretty(&all).unwrap()).unwrap();
}

/// Every match of `re` in `text`, found as jlens finds them: with an interrupt under a
/// timeout or a cancellation token, without one otherwise.
fn find_all(re: &Regex, text: &str) -> usize {
    static CANCEL: AtomicBool = AtomicBool::new(false);
    // jlens sets its deadline once per evaluation, which can hold many matches.
    static DEADLINE: OnceLock<Instant> = OnceLock::new();
    let mut start = 0;
    let mut count = 0;
    loop {
        let found = if INTERRUPTIBLE {
            let deadline = *DEADLINE.get_or_init(|| Instant::now() + Duration::from_secs(3600));
            re.find_from_interruptible(text, start, || {
                CANCEL.load(Ordering::Relaxed) || Instant::now() >= deadline
            })
            .unwrap()
        } else {
            re.find_from(text, start).next()
        };
        let Some(m) = found else { break };
        count += 1;
        // Past an empty match by one character.
        start = if m.end() > m.start() {
            m.end()
        } else {
            match text[m.end()..].chars().next() {
                Some(c) => m.end() + c.len_utf8(),
                None => break,
            }
        };
    }
    count
}

/// The median time of one call of `f`, in nanoseconds.
fn measure(f: impl Fn() -> usize) -> f64 {
    let mut iterations = 1u32;
    loop {
        let start = Instant::now();
        for _ in 0..iterations {
            black_box(f());
        }
        if start.elapsed() >= SAMPLE {
            break;
        }
        iterations *= 2;
    }
    let mut samples: Vec<f64> = (0..ROUNDS)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..iterations {
                black_box(f());
            }
            start.elapsed().as_secs_f64() * 1e9 / f64::from(iterations)
        })
        .collect();
    samples.sort_by(f64::total_cmp);
    samples[ROUNDS / 2]
}

fn format_time(ns: f64) -> String {
    if ns >= 1e6 {
        format!("{:.2} ms", ns / 1e6)
    } else if ns >= 1e3 {
        format!("{:.1} µs", ns / 1e3)
    } else {
        format!("{ns:.0} ns")
    }
}
