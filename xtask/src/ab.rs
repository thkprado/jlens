//! `cargo xtask ab`: compares the speed of two revisions on the benchmark workload
//! (`benches/cases.json`).
//!
//! Criterion is bimodal on some machines: two runs of the same build can differ by more than
//! the change being measured. So each revision is built into its own program (`ab_probe.rs`,
//! against that revision's `jlens-core`), and the two programs run in alternation, A then B,
//! then B then A, so that a drift of the machine reaches both. Each run of a program gives
//! each case its fastest time over a few samples. The speed of a whole process can be bimodal
//! too: the same program is now and then 10% faster for its whole run. So the comparison is on
//! the median of the rounds, which a rare fast process does not move; the fastest round is
//! printed beside it.
//!
//! A revision is anything `git rev-parse` accepts, or `.` for the working tree with its
//! uncommitted changes. The program of a commit is built once, in a temporary git worktree,
//! and kept in `target/ab/bin`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use anyhow::{Context, Result, bail};

use crate::differential::flag;
use crate::root;

const PROBE: &str = include_str!("ab_probe.rs");

/// The probe crate, depending on the `jlens-core` at `{core}`. Its own `[workspace]` keeps it
/// out of the repository's workspace; the profile is the repository's release profile.
const MANIFEST: &str = r#"[package]
name = "jlens-ab-probe"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[[bin]]
name = "probe"
path = "main.rs"

[dependencies]
jlens-core = { path = "{core}" }
serde_json = "1"

[profile.release]
lto = true
codegen-units = 1
"#;

pub fn run(args: &[String]) -> Result<()> {
    let mut positional = Vec::new();
    let mut cases = Vec::new();
    let mut mode = "evaluate".to_owned();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--rounds" | "--sample-ms" => i += 1,
            "--mode" => {
                mode = args.get(i + 1).context("--mode needs a value")?.clone();
                i += 1;
            }
            "--case" => {
                cases.push(args.get(i + 1).context("--case needs a value")?.clone());
                i += 1;
            }
            arg => positional.push(arg.to_owned()),
        }
        i += 1;
    }
    let [a, b] = positional.as_slice() else {
        bail!("ab needs two revisions\n\n{}", super::USAGE);
    };
    if mode != "evaluate" && mode != "document" {
        bail!("--mode is `evaluate` or `document`, not {mode:?}");
    }
    let rounds = flag(args, "--rounds")?.unwrap_or(10).max(1);
    let sample_ms = flag(args, "--sample-ms")?.unwrap_or(10).max(1);

    let programs = [build(a)?, build(b)?];
    let benches = root().join("benches");
    let mut times: [BTreeMap<String, Vec<u128>>; 2] = Default::default();
    let mut order = Vec::new();
    for round in 0..rounds {
        eprintln!("round {} of {rounds}", round + 1);
        let sides = if round % 2 == 0 { [0, 1] } else { [1, 0] };
        for side in sides {
            let output = Command::new(&programs[side])
                .arg(&benches)
                .arg(&mode)
                .arg(sample_ms.to_string())
                .args(&cases)
                .output()
                .with_context(|| format!("running {}", programs[side].display()))?;
            if !output.status.success() {
                bail!(
                    "the program of {} failed: {}",
                    [a, b][side],
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            for line in String::from_utf8(output.stdout)?.lines() {
                let (name, nanos) = line
                    .split_once('\t')
                    .with_context(|| format!("unexpected output {line:?}"))?;
                let nanos: u128 = nanos.parse()?;
                if !order.iter().any(|n| n == name) {
                    order.push(name.to_owned());
                }
                times[side].entry(name.to_owned()).or_default().push(nanos);
            }
        }
    }
    if order.is_empty() {
        bail!("no case of benches/cases.json matched");
    }

    let width = order.iter().map(|n| n.len()).max().unwrap_or(0).max(4);
    println!(
        "{mode}, {rounds} rounds: the median of the rounds, and the fastest round\n\n\
         {:width$}  {:>10}  {:>10}  {:>7}  {:>10}  {:>10}  {:>7}",
        "case", "A median", "B median", "B/A", "A fastest", "B fastest", "B/A"
    );
    for name in &order {
        let [a, b] = [0, 1].map(|side| {
            let mut t = times[side][name].clone();
            t.sort_unstable();
            (t[t.len() / 2], t[0])
        });
        println!(
            "{name:width$}  {:>10}  {:>10}  {:>+6.1}%  {:>10}  {:>10}  {:>+6.1}%",
            duration(a.0),
            duration(b.0),
            change(a.0, b.0),
            duration(a.1),
            duration(b.1),
            change(a.1, b.1)
        );
    }
    println!(
        "
A is {a}, B is {b}"
    );
    Ok(())
}

/// Builds the probe against the `jlens-core` of `rev`, and returns the path of the program.
fn build(rev: &str) -> Result<PathBuf> {
    let dir = root().join("target/ab");
    let bin = dir.join("bin");
    fs::create_dir_all(&bin)?;
    if rev == "." {
        let program = bin.join(format!("worktree{}", env::consts::EXE_SUFFIX));
        compile(rev, &root(), &dir.join("probe-worktree"), &program)?;
        return Ok(program);
    }

    let commit = git(&["rev-parse", "--verify", &format!("{rev}^{{commit}}")])
        .with_context(|| format!("{rev:?} is not a revision"))?;
    let short = &commit[..12];
    let program = bin.join(format!("{short}{}", env::consts::EXE_SUFFIX));
    if program.is_file() {
        return Ok(program);
    }
    let source = dir.join(format!("src-{short}"));
    if source.exists() {
        // Left by an interrupted build.
        let _ = git(&["worktree", "remove", "--force", &path(&source)]);
        let _ = fs::remove_dir_all(&source);
    }
    git(&["worktree", "add", "--detach", &path(&source), &commit])?;
    let built = compile(rev, &source, &dir.join(format!("probe-{short}")), &program);
    git(&["worktree", "remove", "--force", &path(&source)])?;
    built?;
    Ok(program)
}

/// Builds the probe crate in `crate_dir` against the `jlens-core` of the checkout at `checkout`,
/// with the versions of that checkout's lock file, and copies the program to `program`.
fn compile(rev: &str, checkout: &Path, crate_dir: &Path, program: &Path) -> Result<()> {
    fs::create_dir_all(crate_dir)?;
    let core = path(&checkout.join("crates/jlens-core"));
    fs::write(
        crate_dir.join("Cargo.toml"),
        MANIFEST.replace("{core}", &core),
    )?;
    fs::write(crate_dir.join("main.rs"), PROBE)?;
    fs::copy(checkout.join("Cargo.lock"), crate_dir.join("Cargo.lock"))?;
    let target = root().join("target/ab/target");
    eprintln!("building the probe of {rev}");
    let status = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(crate_dir)
        .args(["build", "--quiet", "--release"])
        .env("CARGO_TARGET_DIR", &target)
        .status()
        .context("running cargo build")?;
    if !status.success() {
        bail!("building the probe of {rev} failed");
    }
    let built = target.join(format!("release/probe{}", env::consts::EXE_SUFFIX));
    fs::copy(&built, program).with_context(|| format!("copying {}", built.display()))?;
    Ok(())
}

fn git(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(root())
        .args(args)
        .output()
        .context("running git")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

/// A path as text for git and Cargo.toml, with forward slashes, which both accept on Windows.
fn path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// How much slower `b` is than `a`, in percent.
fn change(a: u128, b: u128) -> f64 {
    (b as f64 / a as f64 - 1.0) * 100.0
}

/// A time in nanoseconds, with three significant digits.
fn duration(nanos: u128) -> String {
    let (value, unit) = match nanos {
        n if n < 1_000 => return format!("{n} ns"),
        n if n < 1_000_000 => (n as f64 / 1e3, "µs"),
        n if n < 1_000_000_000 => (n as f64 / 1e6, "ms"),
        n => (n as f64 / 1e9, "s"),
    };
    let decimals = if value < 10.0 {
        2
    } else if value < 100.0 {
        1
    } else {
        0
    };
    format!("{value:.decimals$} {unit}")
}
