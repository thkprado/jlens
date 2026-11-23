//! `cargo xtask regress-version`: whether `jlens-regress` must be published, or its version
//! raised.
//!
//! `jlens-regress` has a version of its own, raised only when the crate changes (ADR 0020). The
//! version in its manifest is either not on crates.io yet, and the release publishes it, or on
//! crates.io with the same contents, and the release skips it. A version on crates.io whose
//! contents differ from the crate packaged here is an error: the crate changed and its version
//! was not raised. CI runs this on each push, and the release workflow before it publishes.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::{env, fs};

use anyhow::{Context, Result, bail};

use crate::root;

const CRATE: &str = "jlens-regress";

/// Files of the package whose changes alone do not need a new version: Cargo's records of the
/// commit, of the lockfile (which dependents do not read) and of the manifest as written (the
/// normalized `Cargo.toml` is compared), and the README, which the porting of upstream commits
/// updates with each review and which ships with the next version.
const IGNORED: [&str; 4] = [
    ".cargo_vcs_info.json",
    "Cargo.lock",
    "Cargo.toml.orig",
    "README.md",
];

pub fn run() -> Result<()> {
    let manifest = fs::read_to_string(root().join("crates/jlens-regress/Cargo.toml"))?;
    let version = manifest_version(&manifest)?;
    let workspace = fs::read_to_string(root().join("Cargo.toml"))?;
    let required = dependency_version(&workspace)?;
    if required != version {
        bail!(
            "{CRATE} is {version}, and the `regress` entry of the workspace's dependencies asks \
             for {required}: give it {version}"
        );
    }
    let publish = if published(&version)? {
        let local = files(&fs::read(package(&version)?)?)?;
        let remote = files(&download(&version)?)?;
        let changed = differences(&local, &remote);
        if !changed.is_empty() {
            bail!(
                "{CRATE} {version} is on crates.io with other contents ({}): raise its version in \
                 crates/jlens-regress/Cargo.toml and in the `regress` entry of the workspace's \
                 dependencies (CONTRIBUTING.md)",
                changed.join(", ")
            );
        }
        println!("{CRATE} {version} is on crates.io, unchanged: the release does not publish it");
        false
    } else {
        println!("{CRATE} {version} is not on crates.io: the release publishes it");
        true
    };
    if let Some(path) = env::var_os("GITHUB_OUTPUT") {
        let mut out = fs::OpenOptions::new().append(true).open(path)?;
        writeln!(out, "publish={publish}")?;
    }
    Ok(())
}

/// The version of `[package]` in the crate's manifest.
fn manifest_version(manifest: &str) -> Result<String> {
    manifest
        .lines()
        .find_map(|l| l.strip_prefix("version = \""))
        .map(|v| v.trim_end_matches('"').to_owned())
        .with_context(|| format!("no version of its own in {CRATE}'s Cargo.toml"))
}

/// The version the `regress` entry of the workspace's dependencies asks for.
fn dependency_version(workspace: &str) -> Result<String> {
    let entry = workspace
        .lines()
        .find(|l| l.starts_with("regress = "))
        .context("no `regress` entry in the workspace's dependencies")?;
    let (_, rest) = entry
        .split_once("version = \"")
        .context("no version in the workspace's `regress` entry")?;
    Ok(rest.split('"').next().unwrap_or_default().to_owned())
}

/// Whether crates.io's index lists the version.
fn published(version: &str) -> Result<bool> {
    let index = curl(&format!("https://index.crates.io/jl/en/{CRATE}"))?;
    for line in String::from_utf8(index)?.lines() {
        let record: serde_json::Value = serde_json::from_str(line)?;
        if record["vers"] == version {
            return Ok(true);
        }
    }
    Ok(false)
}

fn download(version: &str) -> Result<Vec<u8>> {
    curl(&format!(
        "https://static.crates.io/crates/{CRATE}/{CRATE}-{version}.crate"
    ))
}

fn curl(url: &str) -> Result<Vec<u8>> {
    let output = Command::new("curl")
        .args(["-sSfL", url])
        .output()
        .context("running curl")?;
    if !output.status.success() {
        bail!(
            "curl {url}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

/// Packages the crate as `cargo publish` would, and returns the path of its `.crate`.
fn package(version: &str) -> Result<PathBuf> {
    let status = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(root())
        .args([
            "package",
            "-p",
            CRATE,
            "--no-verify",
            "--allow-dirty",
            "--quiet",
        ])
        .status()
        .context("running cargo package")?;
    if !status.success() {
        bail!("cargo package -p {CRATE} failed: {status}");
    }
    let target =
        env::var_os("CARGO_TARGET_DIR").map_or_else(|| root().join("target"), PathBuf::from);
    Ok(target.join(format!("package/{CRATE}-{version}.crate")))
}

/// The files of a `.crate`, by their path inside the package, without the ignored ones.
fn files(archive: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    Ok(crate::npm::tar_gz_files(archive)?
        .into_iter()
        .filter_map(|(name, data)| {
            let (_, path) = name.split_once('/')?;
            (!IGNORED.contains(&path)).then(|| (path.to_owned(), data))
        })
        .collect())
}

/// The paths that are in one package only, or differ.
fn differences(a: &BTreeMap<String, Vec<u8>>, b: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    let paths: BTreeSet<&String> = a
        .keys()
        .chain(b.keys())
        .filter(|path| a.get(*path) != b.get(*path))
        .collect();
    paths.into_iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_versions() {
        let manifest = "[package]\nname = \"jlens-regress\"\nversion = \"0.35.1\"\n";
        assert_eq!(manifest_version(manifest).unwrap(), "0.35.1");
        assert!(manifest_version("[package]\nversion.workspace = true\n").is_err());
        let workspace = "[workspace.dependencies]\n\
            regress = { package = \"jlens-regress\", path = \"crates/jlens-regress\", \
            version = \"0.35.1\", default-features = false }\n";
        assert_eq!(dependency_version(workspace).unwrap(), "0.35.1");
    }

    #[test]
    fn lists_what_differs() {
        let map = |files: &[(&str, &str)]| {
            files
                .iter()
                .map(|(path, data)| (path.to_string(), data.as_bytes().to_vec()))
                .collect::<BTreeMap<_, _>>()
        };
        let a = map(&[
            ("Cargo.toml", "1"),
            ("src/lib.rs", "a"),
            ("src/new.rs", "n"),
        ]);
        let b = map(&[
            ("Cargo.toml", "1"),
            ("src/lib.rs", "b"),
            ("src/old.rs", "o"),
        ]);
        assert_eq!(
            differences(&a, &b),
            ["src/lib.rs", "src/new.rs", "src/old.rs"]
        );
        assert!(differences(&a, &a).is_empty());
    }
}
