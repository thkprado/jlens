//! `cargo xtask regex-corpus [<test262 commit>]`: rebuilds `crates/jlens-regress/tests/corpus/
//! v8.json`, the corpus of V8's answers that `cargo test -p jlens-regress` checks the engine
//! against.
//!
//! It fetches test262's RegExp tests and harness at the commit (by default the one the corpus
//! came from; `main` moves it) into `target/test262`, records the patterns and searches of
//! jlens-regress's own tests (`JLENS_REGRESS_RECORD`, see `tests/common/mod.rs`), and has
//! `tests/corpus/generate.mjs` run both in Node.js.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::root;

const TEST262: &str = "https://github.com/tc39/test262.git";
const SPARSE: &[&str] = &[
    "/harness/",
    "/test/built-ins/RegExp/",
    "/test/annexB/built-ins/RegExp/",
];

pub fn run(args: &[String]) -> Result<()> {
    let regress = root().join("crates/jlens-regress");
    let corpus = regress.join("tests/corpus/v8.json");
    let commit = match args.first() {
        Some(commit) => commit.clone(),
        None => recorded_commit(&corpus)?,
    };

    let checkout = root().join("target/test262");
    if !checkout.join(".git").is_dir() {
        fs::create_dir_all(&checkout)?;
        git(&checkout, &["init", "--quiet"])?;
        git(&checkout, &["remote", "add", "origin", TEST262])?;
    }
    let mut sparse = vec!["sparse-checkout", "set", "--no-cone"];
    sparse.extend(SPARSE);
    git(&checkout, &sparse)?;
    git(
        &checkout,
        &[
            "fetch",
            "--quiet",
            "--depth",
            "1",
            "--filter=blob:none",
            "origin",
            &commit,
        ],
    )?;
    git(
        &checkout,
        &["checkout", "--quiet", "--detach", "FETCH_HEAD"],
    )?;
    let sha = String::from_utf8(
        Command::new("git")
            .current_dir(&checkout)
            .args(["rev-parse", "HEAD"])
            .output()?
            .stdout,
    )?;
    let sha = sha.trim();

    let recorded = root().join("target/regress-cases.jsonl");
    let _ = fs::remove_file(&recorded);
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(root())
        .args([
            "test",
            "--quiet",
            "-p",
            "jlens-regress",
            "--tests",
            "--",
            "--skip",
            "corpus",
        ])
        .env("JLENS_REGRESS_RECORD", &recorded)
        .status()
        .context("running cargo test")?;
    if !status.success() {
        bail!("jlens-regress's tests failed: {status}");
    }

    let status = Command::new("node")
        .arg(regress.join("tests/corpus/generate.mjs"))
        .args([
            checkout.as_os_str(),
            sha.as_ref(),
            recorded.as_os_str(),
            corpus.as_os_str(),
        ])
        .status()
        .context("running node (Node.js 20+ is needed)")?;
    if !status.success() {
        bail!("generate.mjs failed: {status}");
    }
    Ok(())
}

/// The test262 commit the current corpus came from.
fn recorded_commit(corpus: &Path) -> Result<String> {
    let text =
        fs::read_to_string(corpus).with_context(|| format!("reading {}", corpus.display()))?;
    // The header is on the first line; the cases follow.
    let header = text.lines().next().unwrap_or_default();
    let start = header
        .find("\"test262\":\"")
        .context("no test262 commit in the corpus")?
        + 11;
    let len = header[start..]
        .find('"')
        .context("no test262 commit in the corpus")?;
    Ok(header[start..start + len].to_string())
}

fn git(dir: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .context("running git")?;
    if !status.success() {
        bail!("git {} failed: {status}", args.join(" "));
    }
    Ok(())
}
