//! `cargo xtask java`: the Java package (ADR 0021), built by Maven over `jlens-capi`.

use std::env::consts::{ARCH, DLL_PREFIX, DLL_SUFFIX, OS};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::root;

/// The Maven project of the Java package.
fn project() -> PathBuf {
    root().join("crates/jlens-java/java")
}

/// Builds the C API for the host, puts it where the JAR takes it from (`native/<os>-<arch>/`),
/// and runs the Maven Wrapper with `args`, `verify` when none are given.
pub fn test(args: &[String]) -> Result<()> {
    check_version()?;
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(root())
        .args(["build", "--release", "-p", "jlens-capi"])
        .status()
        .context("running cargo build")?;
    if !status.success() {
        bail!("cargo build -p jlens-capi failed");
    }
    let name = format!("{DLL_PREFIX}jlens_capi{DLL_SUFFIX}");
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root().join("target"), PathBuf::from);
    let built = target.join("release").join(&name);
    let native = project().join("native").join(format!("{OS}-{ARCH}"));
    fs::create_dir_all(&native)?;
    fs::copy(&built, native.join(&name)).with_context(|| format!("copying {}", built.display()))?;

    let wrapper = match cfg!(windows) {
        true => project().join("mvnw.cmd"),
        false => project().join("mvnw"),
    };
    let mut maven = Command::new(&wrapper);
    maven.current_dir(project()).arg("-B");
    match args.is_empty() {
        true => maven.arg("verify"),
        false => maven.args(args),
    };
    let status = maven.status().with_context(|| {
        format!(
            "running {} (a JDK 25 or later is required)",
            wrapper.display()
        )
    })?;
    if !status.success() {
        bail!("mvnw failed");
    }
    Ok(())
}

/// Fails unless `pom.xml` has the workspace's version.
fn check_version() -> Result<()> {
    let version = crate::wasm::workspace_version()?;
    let pom = fs::read_to_string(project().join("pom.xml"))?;
    // The project's own version is the first one after its artifactId.
    let ours = pom
        .split_once("<artifactId>jlens</artifactId>")
        .and_then(|(_, rest)| rest.split_once("<version>"))
        .and_then(|(_, rest)| rest.split_once("</version>"))
        .map(|(ours, _)| ours);
    if ours != Some(version.as_str()) {
        bail!("crates/jlens-java/java/pom.xml must have the workspace's version, {version}");
    }
    Ok(())
}
