//! Repository automation: `cargo xtask <command>`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use anyhow::{Context, Result, bail};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  conformance [--update-baseline]   run the whole JSONata suite and write tests/conformance/REPORT.md
  update-suite <jsonata-js checkout> replace tests/jsonata-suite with <checkout>/test/test-suite
";

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("conformance") => conformance(args.iter().any(|a| a == "--update-baseline")),
        Some("update-suite") => match args.get(1) {
            Some(path) => update_suite(Path::new(path)),
            None => bail!("update-suite needs the path of a jsonata-js checkout\n\n{USAGE}"),
        },
        _ => {
            eprint!("{USAGE}");
            Ok(())
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn conformance(update_baseline: bool) -> Result<()> {
    let mut cmd = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(root())
        .args([
            "test",
            "--quiet",
            "-p",
            "jlens-core",
            "--test",
            "conformance",
        ])
        .env("JLENS_CONFORMANCE_REPORT", "1");
    if update_baseline {
        cmd.env("JLENS_UPDATE_BASELINE", "1");
    }
    let status = cmd.status().context("running cargo test")?;
    if !status.success() {
        bail!("conformance run failed: {status}");
    }
    Ok(())
}

fn update_suite(checkout: &Path) -> Result<()> {
    let src = checkout.join("test/test-suite");
    if !src.join("groups").is_dir() {
        bail!(
            "{} does not contain test/test-suite/groups",
            checkout.display()
        );
    }
    let dest = root().join("tests/jsonata-suite");
    for sub in ["groups", "datasets"] {
        let target = dest.join(sub);
        if target.exists() {
            fs::remove_dir_all(&target)?;
        }
        copy_dir(&src.join(sub), &target)?;
    }
    fs::copy(src.join("TESTSUITE.md"), dest.join("TESTSUITE.md"))?;
    fs::copy(checkout.join("LICENSE"), dest.join("LICENSE"))?;
    println!("suite updated; remember to bump the pinned version in tests/jsonata-suite/README.md");
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &target)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}
