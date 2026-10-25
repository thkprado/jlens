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
//! `--mode` picks how the cases read their input: `evaluate` from the `serde_json::Value`,
//! `document` from a `Document` converted once, `source` through a `Source` in the probe, one
//! level at a time, as the Python package reads its values.
//!
//! A revision is anything `git rev-parse` accepts, or `.` for the working tree with its
//! uncommitted changes. The program of a commit is built once, in a temporary git worktree,
//! and kept in `target/ab/bin`.
//!
//! Runs may overlap. Each keeps its worktrees and probe crates in its own directory,
//! `target/ab/run-<process id>`, removed when it ends; Cargo's lock on the shared target
//! directory takes their builds one at a time, and a program reaches `target/ab/bin` by a
//! rename, so a run sees either no program or a whole one.
//!
//! `--b-allocator mimalloc` makes `mimalloc` the global allocator of B's program, so that
//! `ab . . --b-allocator mimalloc` measures the allocator alone. `--b-max-steps N` evaluates B's
//! cases with `Options::max_steps(N)`, so that `ab . . --b-max-steps N` measures a step limit
//! against none (ADR 0018); B must be a revision that has it. `--timeout MS` evaluates the cases
//! of both programs with `Options::timeout`, long enough never to expire, so that `ab A B
//! --timeout MS` measures what a timeout costs in each. `--target` builds both programs for
//! another target that runs here, such as `x86_64-unknown-linux-musl` on Linux.

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
name = "{bin}"
path = "main.rs"

[dependencies]
jlens-core = { path = "{core}" }
serde_json = "1"
{extra}

[profile.release]
lto = true
codegen-units = 1
"#;

/// Makes `mimalloc` the global allocator of the probe; appended, as the probe starts with its
/// inner doc comment.
const MIMALLOC: &str = "
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
";

/// The line of the probe that `--b-max-steps` and `--timeout` replace.
const OPTIONS: &str = "let options = Options::default();";

/// The lines of the probe that read an item of an array alone, with `Sources::length` and
/// `Sources::read_item` (0.24); left out for a revision whose `jlens-core` has no `read_item`.
const ITEMS_ALONE: (&str, &str) = ("    // ab: items alone {\n", "    // ab: }\n");

/// How B's program differs from A's.
#[derive(Clone, Copy, Default)]
struct Variant {
    mimalloc: bool,
    max_steps: Option<u64>,
    /// The timeout, in milliseconds, of both programs.
    timeout: Option<u64>,
}

pub fn run(args: &[String]) -> Result<()> {
    let mut positional = Vec::new();
    let mut cases = Vec::new();
    let mut mode = "evaluate".to_owned();
    let mut b_allocator = "system".to_owned();
    let mut b_max_steps = None;
    let mut timeout = None;
    let mut target = None;
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
            "--b-allocator" => {
                b_allocator = args
                    .get(i + 1)
                    .context("--b-allocator needs a value")?
                    .clone();
                i += 1;
            }
            "--b-max-steps" => {
                let steps = args.get(i + 1).context("--b-max-steps needs a value")?;
                b_max_steps = Some(
                    steps
                        .parse::<u64>()
                        .with_context(|| format!("--b-max-steps takes a number, not {steps:?}"))?,
                );
                i += 1;
            }
            "--timeout" => {
                let ms = args.get(i + 1).context("--timeout needs a value")?;
                timeout = Some(
                    ms.parse::<u64>()
                        .with_context(|| format!("--timeout takes milliseconds, not {ms:?}"))?,
                );
                i += 1;
            }
            "--target" => {
                target = Some(args.get(i + 1).context("--target needs a value")?.clone());
                i += 1;
            }
            arg => positional.push(arg.to_owned()),
        }
        i += 1;
    }
    let [a, b] = positional.as_slice() else {
        bail!("ab needs two revisions\n\n{}", super::USAGE);
    };
    if !["evaluate", "document", "source"].contains(&mode.as_str()) {
        bail!("--mode is `evaluate`, `document` or `source`, not {mode:?}");
    }
    if b_allocator != "system" && b_allocator != "mimalloc" {
        bail!("--b-allocator is `system` or `mimalloc`, not {b_allocator:?}");
    }
    let rounds = flag(args, "--rounds")?.unwrap_or(10).max(1);
    let sample_ms = flag(args, "--sample-ms")?.unwrap_or(10).max(1);

    let work = Work::new(target)?;
    let programs = [
        work.build(
            a,
            Variant {
                timeout,
                ..Variant::default()
            },
        )?,
        work.build(
            b,
            Variant {
                mimalloc: b_allocator == "mimalloc",
                max_steps: b_max_steps,
                timeout,
            },
        )?,
    ];
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
    let mut with = Vec::new();
    if b_allocator == "mimalloc" {
        with.push("mimalloc".to_owned());
    }
    if let Some(steps) = b_max_steps {
        with.push(format!("max_steps({steps})"));
    }
    let with = if with.is_empty() {
        String::new()
    } else {
        format!(" with {}", with.join(" and "))
    };
    let both = timeout.map_or(String::new(), |ms| format!("; both with timeout({ms} ms)"));
    println!(
        "
A is {a}, B is {b}{with}{both}"
    );
    Ok(())
}

/// The directory of one run, `target/ab/run-<process id>`, removed when the run ends.
struct Work {
    dir: PathBuf,
    /// The target the programs are built for, when not the host.
    target: Option<String>,
}

impl Work {
    fn new(target: Option<String>) -> Result<Work> {
        let ab = root().join("target/ab");
        fs::create_dir_all(ab.join("bin"))?;
        // Directories of interrupted runs: the one with this process id cannot belong to a live
        // run, and no run lasts a day.
        let mut pruned = false;
        for entry in fs::read_dir(&ab)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(pid) = name.to_str().and_then(|n| n.strip_prefix("run-")) else {
                continue;
            };
            let age = entry.metadata()?.modified()?.elapsed().unwrap_or_default();
            if pid == std::process::id().to_string() || age.as_secs() > 24 * 60 * 60 {
                let _ = fs::remove_dir_all(entry.path());
                pruned = true;
            }
        }
        if pruned {
            git(&["worktree", "prune"])?;
        }
        let dir = ab.join(format!("run-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        Ok(Work { dir, target })
    }

    /// Builds the probe against the `jlens-core` of `rev`, as `with` asks, and returns the path
    /// of the program.
    fn build(&self, rev: &str, with: Variant) -> Result<PathBuf> {
        // Programs that differ in allocator, step limit or target are kept apart.
        let mut variant = String::new();
        if with.mimalloc {
            variant.push_str("-mimalloc");
        }
        if let Some(steps) = with.max_steps {
            variant.push_str(&format!("-steps{steps}"));
        }
        if let Some(ms) = with.timeout {
            variant.push_str(&format!("-timeout{ms}"));
        }
        if let Some(target) = &self.target {
            variant.push_str(&format!("-{target}"));
        }
        if rev == "." {
            let name = format!("worktree{variant}");
            let program = self.dir.join(format!("{name}{}", env::consts::EXE_SUFFIX));
            self.compile(rev, &root(), &name, with, &program)?;
            return Ok(program);
        }

        let commit = git(&["rev-parse", "--verify", &format!("{rev}^{{commit}}")])
            .with_context(|| format!("{rev:?} is not a revision"))?;
        let short = &commit[..12];
        let file = format!("{short}{variant}{}", env::consts::EXE_SUFFIX);
        let program = root().join("target/ab/bin").join(&file);
        if program.is_file() {
            return Ok(program);
        }
        let source = self.dir.join(format!("src-{short}"));
        git(&["worktree", "add", "--detach", &path(&source), &commit])?;
        let name = format!("{short}{variant}");
        let built = self.compile(rev, &source, &name, with, &self.dir.join(&file));
        git(&["worktree", "remove", "--force", &path(&source)])?;
        built?;
        // Another run may have published the same program meanwhile; on Windows the rename then
        // fails while that program runs, and its program serves as well.
        if let Err(error) = fs::rename(self.dir.join(&file), &program)
            && !program.is_file()
        {
            return Err(error).with_context(|| format!("publishing {}", program.display()));
        }
        Ok(program)
    }

    /// Builds the probe crate `probe-{name}` against the `jlens-core` of the checkout at
    /// `checkout`, with the versions of that checkout's lock file, and copies the program to
    /// `program`.
    fn compile(
        &self,
        rev: &str,
        checkout: &Path,
        name: &str,
        with: Variant,
        program: &Path,
    ) -> Result<()> {
        let crate_dir = self.dir.join(format!("probe-{name}"));
        fs::create_dir_all(&crate_dir)?;
        let core = path(&checkout.join("crates/jlens-core"));
        // The program's name is the run's own, so that runs building at once do not overwrite
        // each other's in the shared target directory.
        let bin = format!("probe-{}", std::process::id());
        fs::write(
            crate_dir.join("Cargo.toml"),
            MANIFEST
                .replace("{core}", &core)
                .replace("{bin}", &bin)
                .replace(
                    "{extra}",
                    if with.mimalloc {
                        "mimalloc = \"0.1\""
                    } else {
                        ""
                    },
                ),
        )?;
        let mut source = PROBE.to_owned();
        let mut options = String::new();
        if let Some(steps) = with.max_steps {
            options.push_str(&format!(".max_steps({steps})"));
        }
        if let Some(ms) = with.timeout {
            options.push_str(&format!(".timeout(Duration::from_millis({ms}))"));
        }
        if !options.is_empty() {
            if !source.contains(OPTIONS) {
                bail!(
                    "the probe has no line {OPTIONS:?} for --b-max-steps or --timeout to replace"
                );
            }
            source = source.replace(
                OPTIONS,
                &format!("let options = Options::default(){options};"),
            );
        }
        let value = fs::read_to_string(checkout.join("crates/jlens-core/src/value.rs"))?;
        if !value.contains("fn read_item(") {
            let (begin, end) = ITEMS_ALONE;
            let (Some(start), Some(stop)) = (source.find(begin), source.find(end)) else {
                bail!("the probe has no lines {begin:?} and {end:?} around its items read alone");
            };
            source.replace_range(start..stop + end.len(), "");
        }
        if with.mimalloc {
            source.push_str(MIMALLOC);
        }
        fs::write(crate_dir.join("main.rs"), source)?;
        fs::copy(checkout.join("Cargo.lock"), crate_dir.join("Cargo.lock"))?;
        let target = root().join("target/ab/target");
        eprintln!("building the probe of {rev}");
        let mut cargo = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        cargo
            .current_dir(&crate_dir)
            .args(["build", "--quiet", "--release"])
            .env("CARGO_TARGET_DIR", &target);
        let mut release = target.clone();
        if let Some(triple) = &self.target {
            cargo.args(["--target", triple]);
            release.push(triple);
        }
        let status = cargo.status().context("running cargo build")?;
        if !status.success() {
            bail!("building the probe of {rev} failed");
        }
        let built = release.join(format!("release/{bin}{}", env::consts::EXE_SUFFIX));
        fs::copy(&built, program).with_context(|| format!("copying {}", built.display()))?;
        let _ = fs::remove_file(&built);
        Ok(())
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
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
