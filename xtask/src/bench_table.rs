//! `cargo xtask bench-table`: measures the native table of the performance page.
//!
//! One run of `jlens-core`'s benchmark `evaluate` can be far off: for 0.25.0, one run gave
//! 1.19 ms on `regex` against a `Document`, where the next two gave 179 and 182 µs. So the table
//! takes, for each benchmark, the median of several runs (three by default). A time also follows
//! the memory the program's file was given, which stays with the file while it is cached, so each
//! run has a new copy of the program, built once by `cargo bench --no-run`, and every copy stays
//! until the last run, as a copy deleted may give its memory to the next. Each run saves its
//! results as a criterion baseline, `table-<n>`, and this command reads them back: for each
//! benchmark, the time criterion prints (its slope, or its mean when it has none), in each run,
//! the median and the spread, the slowest run against the fastest. The median stands when most
//! runs are within 2% of it, so that one run far off does not move it; a case where they are not
//! is marked: run again, with more runs, before reading it. A case whose runs fall in two groups
//! is marked too, though two of three agree: on Windows, the system allocator gives `recursion`
//! against a `Document` about 790 or 890 µs, drawn per process.
//!
//! `--reuse` reads the baselines `table-1` to `table-<runs>` already saved, without running.
//! The target directory is `CARGO_TARGET_DIR`'s, as Cargo's is, so that a run built with other
//! `RUSTFLAGS`, such as the alignment flags, keeps its own.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{env, fs};

use anyhow::{Context, Result, bail};

use crate::differential::flag;
use crate::root;

/// How far from the median a run may be to agree with it.
const AGREE: f64 = 0.02;

/// How far apart two runs next to each other, in order of time, may be and still be one group.
const GROUP: f64 = 0.05;

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
        let program = build(&target)?;
        let copies = target.join("bench-table");
        if copies.exists() {
            fs::remove_dir_all(&copies).context("removing the copies of an earlier run")?;
        }
        fs::create_dir_all(&copies)?;
        for n in 1..=runs {
            eprintln!("bench-table: run {n} of {runs}");
            // The name has one length for every run, as a program's path changes what its process
            // has on the stack.
            let copy = copies.join(format!("run-{n:06}{}", env::consts::EXE_SUFFIX));
            fs::copy(&program, &copy).with_context(|| format!("copying {}", program.display()))?;
            // As `cargo bench` runs it, with the package as its directory.
            let status = Command::new(&copy)
                .current_dir(root().join("crates/jlens-core"))
                .args(["--bench", "--save-baseline", &format!("table-{n}")])
                .env("CARGO_TARGET_DIR", &target)
                .status()
                .with_context(|| format!("running {}", copy.display()))?;
            if !status.success() {
                bail!("the benchmark failed in run {n}");
            }
        }
        fs::remove_dir_all(&copies).context("removing the copies")?;
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
        let median = median(&times);
        let spread = times[times.len() - 1] / times[0] - 1.0;
        let each: Vec<String> = row.times.iter().map(|&t| format_ns(t)).collect();
        println!(
            "{:width$}  median {:>9}  spread {:>5.1}%{}  [{}]",
            row.id(),
            format_ns(median),
            spread * 100.0,
            if doubtful(&times, median) { " *" } else { "  " },
            each.join(", "),
        );
    }
    Ok(())
}

/// Builds the benchmark as `cargo bench` does, and returns the path of its program.
fn build(target: &Path) -> Result<PathBuf> {
    let output = Command::new(env!("CARGO"))
        .current_dir(root())
        .args(["bench", "-p", "jlens-core", "--bench", "evaluate"])
        .args(["--no-run", "--message-format=json"])
        .env("CARGO_TARGET_DIR", target)
        .stderr(Stdio::inherit())
        .output()
        .context("running cargo bench")?;
    if !output.status.success() {
        bail!("cargo bench could not build the benchmark");
    }
    program(&String::from_utf8_lossy(&output.stdout))
        .context("cargo bench named no program for the benchmark")
}

/// The program of the benchmark `evaluate`, in the messages Cargo prints.
fn program(messages: &str) -> Option<PathBuf> {
    messages.lines().find_map(|line| {
        let message: serde_json::Value = serde_json::from_str(line).ok()?;
        let target = &message["target"];
        let is_bench = target["kind"].as_array()?.iter().any(|k| k == "bench");
        (is_bench && target["name"] == "evaluate")
            .then(|| message["executable"].as_str().map(PathBuf::from))
            .flatten()
    })
}

/// The median of `sorted`, which is not empty.
pub(crate) fn median(sorted: &[f64]) -> f64 {
    if sorted.len() % 2 == 1 {
        sorted[sorted.len() / 2]
    } else {
        (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0
    }
}

/// Whether the median of `sorted`, which is not empty, needs a closer look: most runs are not
/// near it, or the runs fall in two groups.
fn doubtful(sorted: &[f64], median: f64) -> bool {
    let agree = sorted
        .iter()
        .filter(|&&t| (t / median - 1.0).abs() <= AGREE)
        .count();
    agree * 2 <= sorted.len() || sorted.windows(2).any(|w| w[1] / w[0] - 1.0 > GROUP)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_median_is_doubtful_when_runs_disagree_or_fall_in_two_groups() {
        let check = |times: &[f64]| doubtful(times, median(times));
        assert!(!check(&[100.0]));
        assert!(!check(&[100.0, 101.0, 102.0]));
        // Most runs far from the median.
        assert!(check(&[96.0, 100.0, 104.0]));
        // Two of three agree, as `recursion` against a `Document` for 0.26.0.
        assert!(check(&[785.0, 882.0, 885.0]));
        // All in the slow group: the runs cannot show it.
        assert!(!check(&[884.0, 890.0, 899.0, 901.0, 907.0]));
    }

    #[test]
    fn the_program_is_the_benchmark_s_own() {
        let messages = [
            r#"{"reason":"compiler-artifact","target":{"name":"jlens_core","kind":["lib"]},"executable":null}"#,
            r#"{"reason":"compiler-artifact","target":{"name":"host_state","kind":["bench"]},"executable":"host"}"#,
            "not a message",
            r#"{"reason":"build-finished","success":true}"#,
            r#"{"reason":"compiler-artifact","target":{"name":"evaluate","kind":["bench"]},"executable":"evaluate-1"}"#,
        ]
        .join("\n");
        assert_eq!(program(&messages), Some(PathBuf::from("evaluate-1")));
        assert_eq!(program(&messages[..messages.rfind('\n').unwrap()]), None);
    }
}
