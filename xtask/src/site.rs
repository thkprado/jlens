//! `cargo xtask site`: the book and the web editor, assembled as one static site.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::{copy_dir, root, wasm};

/// The import map entry of `examples/web-editor/index.html` that points to the package in the
/// repository, and its replacement on the site.
const PKG_IN_REPOSITORY: &str = r#""jlens-wasm": "../../crates/jlens-wasm/pkg/jlens.js""#;
const PKG_ON_SITE: &str = r#""jlens-wasm": "./pkg/jlens.js""#;

/// The files of the package the editor loads: the module, the binary, and the JavaScript
/// helpers that `wasm-bindgen` writes in `snippets/`.
const PKG_FILES: &[&str] = &["jlens.js", "jlens_bg.wasm", "snippets"];

pub fn dir() -> PathBuf {
    root().join("target/site")
}

/// Builds the WebAssembly package and the book, and assembles them in `target/site`: the book
/// at the root, the editor in `editor/`, the package in `editor/pkg/`.
pub fn build() -> Result<()> {
    wasm::build(false)?;
    let site = dir();
    if site.exists() {
        fs::remove_dir_all(&site)?;
    }
    let status = Command::new("mdbook")
        .arg("build")
        .arg(root().join("docs"))
        .arg("--dest-dir")
        .arg(&site)
        .status()
        .context("running mdbook (install it with `cargo install mdbook`)")?;
    if !status.success() {
        bail!("mdbook failed: {status}");
    }
    assemble_editor(&site.join("editor"))?;
    println!(
        "wrote {}; preview it with `cargo xtask serve --site`",
        site.display()
    );
    Ok(())
}

/// Copies the web editor and the built package into `dest`, with the import map pointing to
/// the copy of the package. The package must already be built.
pub fn assemble_editor(dest: &Path) -> Result<()> {
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    copy_dir(&root().join("examples/web-editor"), dest)?;
    // The README links files of the repository.
    fs::remove_file(dest.join("README.md"))?;

    let index = dest.join("index.html");
    let html = fs::read_to_string(&index)?;
    if !html.contains(PKG_IN_REPOSITORY) {
        bail!("the import map of examples/web-editor/index.html has no {PKG_IN_REPOSITORY} entry");
    }
    fs::write(&index, html.replacen(PKG_IN_REPOSITORY, PKG_ON_SITE, 1))?;

    let pkg = dest.join("pkg");
    fs::create_dir_all(&pkg)?;
    for file in PKG_FILES {
        let from = wasm::pkg_dir().join(file);
        if !from.exists() {
            bail!("{} is missing; run `cargo xtask wasm`", from.display());
        }
        if from.is_dir() {
            copy_dir(&from, &pkg.join(file))?;
        } else {
            fs::copy(&from, pkg.join(file))?;
        }
    }
    Ok(())
}
