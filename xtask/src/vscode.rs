//! `cargo xtask vscode`: the VS Code extension's packages (ADR 0013).
//!
//! Builds, from the release archives, one VSIX per VS Code target with the `jlens` binary of
//! that target in `server/`, and one VSIX without a binary, which uses the `jlens` on the
//! `PATH`. The version is the workspace's, whatever `editors/vscode/package.json` says.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::npm::{find_archive, npm, tar_gz_entry, write_executable, zip_entry};
use crate::root;
use crate::wasm::workspace_version;

/// The VS Code targets and the release target (binaries.yml) whose binary they bundle. The
/// static musl binaries serve both glibc (`linux-`) and musl (`alpine-`) systems.
const TARGETS: [(&str, &str); 7] = [
    ("linux-x64", "x86_64-unknown-linux-musl"),
    ("linux-arm64", "aarch64-unknown-linux-musl"),
    ("alpine-x64", "x86_64-unknown-linux-musl"),
    ("alpine-arm64", "aarch64-unknown-linux-musl"),
    ("darwin-x64", "x86_64-apple-darwin"),
    ("darwin-arm64", "aarch64-apple-darwin"),
    ("win32-x64", "x86_64-pc-windows-msvc"),
];

/// The files of `editors/vscode` that go into every package, besides `package.json`.
const FILES: [&str; 5] = [
    "README.md",
    "language-configuration.json",
    "syntaxes/jsonata.tmLanguage.json",
    "schemas/jlens.schema.json",
    "dist/extension.js",
];

/// Bundles the client (`npm run build`, after `npm ci` in `editors/vscode`), then writes the
/// eight VSIX files into `target/vscode`, from the archives found in `archives`
/// (`jlens-<version or commit>-<target>.tar.gz`, `.zip` on Windows). Every target must have
/// its archive.
pub fn build(archives: &Path) -> Result<()> {
    let version = workspace_version()?;
    let source = root().join("editors/vscode");
    let vsce = source.join("node_modules/@vscode/vsce/vsce");
    if !vsce.exists() {
        bail!(
            "vsce is not installed: run `npm ci` in {}",
            source.display()
        );
    }
    let status = npm()
        .args(["run", "build"])
        .current_dir(&source)
        .status()
        .context("running npm run build")?;
    if !status.success() {
        bail!("npm run build failed: {status}");
    }

    let out = root().join("target/vscode");
    if out.exists() {
        fs::remove_dir_all(&out)?;
    }
    let stage = out.join("stage");
    for file in FILES {
        let to = stage.join(file);
        fs::create_dir_all(to.parent().unwrap_or(&stage))?;
        fs::copy(source.join(file), &to).with_context(|| format!("copying {file}"))?;
    }
    fs::copy(root().join("LICENSE"), stage.join("LICENSE"))?;
    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(source.join("package.json"))?)?;
    manifest["version"] = version.clone().into();

    if cfg!(windows) {
        eprintln!(
            "warning: packed on Windows, the Linux and macOS binaries are not executable: \
             pack them on Linux or macOS"
        );
    }
    let server = stage.join("server");
    for (vscode_target, target) in TARGETS {
        let windows = vscode_target.starts_with("win32");
        let exe = if windows { "jlens.exe" } else { "jlens" };
        let archive = find_archive(archives, target)?;
        let bytes = fs::read(&archive)?;
        let binary = if windows {
            zip_entry(&bytes, exe)
        } else {
            tar_gz_entry(&bytes, exe)
        }
        .with_context(|| format!("reading {}", archive.display()))?;
        if server.exists() {
            fs::remove_dir_all(&server)?;
        }
        fs::create_dir_all(&server)?;
        write_executable(&server.join(exe), &binary)?;
        let vsix = out.join(format!("jlens-{vscode_target}-{version}.vsix"));
        pack(&vsce, &stage, &manifest, &vsix, Some(vscode_target))?;
    }
    fs::remove_dir_all(&server)?;
    let vsix = out.join(format!("jlens-{version}.vsix"));
    pack(&vsce, &stage, &manifest, &vsix, None)?;
    fs::remove_dir_all(&stage)?;
    println!("VS Code packages of jlens {version} in {}", out.display());
    Ok(())
}

/// Packs the staged extension with vsce. The client is bundled, so the packages need no
/// `node_modules`. vsce keeps the file mode of the binary, which VS Code restores when it
/// installs the package: the Unix binaries are executable only when packed on Linux or macOS.
fn pack(
    vsce: &Path,
    stage: &Path,
    manifest: &serde_json::Value,
    vsix: &Path,
    target: Option<&str>,
) -> Result<()> {
    // vsce rejects a `files` pattern that matches nothing: only a package with a binary lists
    // `server/`.
    let mut manifest = manifest.clone();
    if target.is_some() {
        manifest["files"]
            .as_array_mut()
            .context("no files in package.json")?
            .push("server/".into());
    }
    fs::write(
        stage.join("package.json"),
        serde_json::to_string_pretty(&manifest)? + "\n",
    )?;
    let mut cmd = Command::new("node");
    cmd.arg(vsce)
        .args(["package", "--no-dependencies", "--out"])
        .arg(vsix)
        .current_dir(stage);
    if let Some(target) = target {
        cmd.args(["--target", target]);
    }
    let status = cmd.status().context("running vsce")?;
    if !status.success() {
        bail!("vsce package failed for {}: {status}", vsix.display());
    }
    Ok(())
}
