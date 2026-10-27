//! `cargo xtask bench-table`: measures the native table of the performance page.
//!
//! One run of `cargo bench -p jlens-core --bench evaluate` can be far off: for 0.25.0, one run
//! gave 1.19 ms on `regex` against a `Document`, where the next two gave 179 and 182 µs. So the
//! table takes, for each benchmark, the median of several runs (three by default). Each run
//! saves its results as a criterion baseline, `table-<n>`, and this command reads them back:
//! for each benchmark, the time criterion prints (its slope, or its mean when it has none), in
//! each run, the median and the spread, the slowest run against the fastest. The median stands
//! when most runs are within 2% of it, so that one run far off does not move it; a case where
//! they are not is marked: run again, with more runs, before reading it.
//!
//! `--reuse` reads the baselines `table-1` to `table-<runs>` already saved, without running.
//! The target directory is `CARGO_TARGET_DIR`'s, as Cargo's is, so that a run built with other
//! `RUSTFLAGS`, such as the alignment flags, keeps its own.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use anyhow::{Context, Result, bail};

use crate::differential::flag;
use crate::root;

/// How far from the median a run may be to agree with it.
const AGREE: f64 = 0.02;

pub(crate) fn run(args: &[String]) -> Result<()> {
    let runs = flag(args, "--runs")?.unwrap_or(3);
    if runs == 0 {
        bail!("--runs needs at least one run");
    }
    // Absolute, as Cargo reads a relative one, from where it runs: criterion would read it
    // from the crate's directory.
    let target = match env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => env::current_dir()?.join(PathBuf::from(dir)),
        None => root().join("target"),
    };
    if !args.iter().any(|a| a == "--reuse") {
        for n in 1..=runs {
            eprintln!("bench-table: run {n} of {runs}");
            let status = Command::new(env!("CARGO"))
                .current_dir(root())
                .args(["bench", "-p", "jlens-core", "--bench", "evaluate", "--"])
                .args(["--save-baseline", &format!("table-{n}")])
                .env("CARGO_TARGET_DIR", &target)
                .status()
                .context("running cargo bench")?;
            if !status.success() {
                bail!("cargo bench failed in run {n}");
            }
        }
    }

    let mut rows = Vec::new();
    collect(&target.join("criterion"), runs, &mut rows)?;
    if rows.is_empty() {
        bail!(
            "no baseline table-1 under {}: run without --reuse",
            target.join("criterion").display()
        );
    }
    rows.sort_by_key(|row| (group_rank(&row.group), row.group.clone(), row.name.clone()));
    let width = rows.iter().map(|row| row.id().len()).max().unwrap_or(0);
    for row in &rows {
        let mut times = row.times.clone();
        times.sort_by(f64::total_cmp);
        let median = if times.len() % 2 == 1 {
            times[times.len() / 2]
        } else {
            (times[times.len() / 2 - 1] + times[times.len() / 2]) / 2.0
        };
        let spread = times[times.len() - 1] / times[0] - 1.0;
        let agree = times
            .iter()
            .filter(|&&t| (t / median - 1.0).abs() <= AGREE)
            .count();
        let each: Vec<String> = row.times.iter().map(|&t| format_ns(t)).collect();
        println!(
            "{:width$}  median {:>9}  spread {:>5.1}%{}  [{}]",
            row.id(),
            format_ns(median),
            spread * 100.0,
            if agree * 2 > times.len() { "  " } else { " *" },
            each.join(", "),
        );
    }
    Ok(())
}

/// One benchmark, with its time in each run, in nanoseconds.
struct Row {
    group: String,
    name: String,
    times: Vec<f64>,
}

impl Row {
    fn id(&self) -> String {
        format!("{}/{}", self.group, self.name)
    }
}

/// The groups in the order of the page's tables; any other after them.
fn group_rank(group: &str) -> usize {
    [
        "evaluate",
        "evaluate_document",
        "compile",
        "document",
        "audit",
    ]
    .iter()
    .position(|g| *g == group)
    .unwrap_or(usize::MAX)
}

/// Finds the benchmarks under `dir` that have the baseline `table-1`, and reads their runs.
fn collect(dir: &Path, runs: u64, rows: &mut Vec<Row>) -> Result<()> {
    let first = dir.join("table-1");
    if first.is_dir() {
        let info: serde_json::Value = read_json(&first.join("benchmark.json"))?;
        let text = |key: &str| info[key].as_str().unwrap_or_default().to_owned();
        let mut times = Vec::new();
        for n in 1..=runs {
            let estimates = read_json(&dir.join(format!("table-{n}")).join("estimates.json"))?;
            // Criterion prints the slope when it has one (linear sampling), else the mean.
            let time = estimates["slope"]["point_estimate"]
                .as_f64()
                .or_else(|| estimates["mean"]["point_estimate"].as_f64())
                .with_context(|| format!("no estimate in {}", dir.display()))?;
            times.push(time);
        }
        rows.push(Row {
            group: text("group_id"),
            name: text("function_id"),
            times,
        });
        return Ok(());
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, runs, rows)?;
        }
    }
    Ok(())
}

fn read_json(path: &Path) -> Result<serde_json::Value> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// A time as the page writes it: three significant digits, in ns, µs or ms.
fn format_ns(ns: f64) -> String {
    let (value, unit) = if ns >= 1e6 {
        (ns / 1e6, "ms")
    } else if ns >= 1e3 {
        (ns / 1e3, "µs")
    } else {
        (ns, "ns")
    };
    let decimals = if value >= 100.0 {
        0
    } else if value >= 10.0 {
        1
    } else {
        2
    };
    format!("{value:.decimals$} {unit}")
}
