//! `cargo xtask npm-cli`: the npm packages of the command line (ADR 0012).
//!
//! Builds, from the release archives, one package per platform holding its binary and the
//! package `jlens-cli`, whose launcher runs the binary of the platform package npm installed.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::root;
use crate::wasm::workspace_version;

/// The release targets (binaries.yml) and their `process.platform` and `process.arch`.
const PLATFORMS: [(&str, &str, &str); 5] = [
    ("x86_64-unknown-linux-musl", "linux", "x64"),
    ("aarch64-unknown-linux-musl", "linux", "arm64"),
    ("x86_64-apple-darwin", "darwin", "x64"),
    ("aarch64-apple-darwin", "darwin", "arm64"),
    ("x86_64-pc-windows-msvc", "win32", "x64"),
];

const REPOSITORY: &str = r#"{
    "type": "git",
    "url": "git+https://github.com/thkprado/jlens.git",
    "directory": "crates/jlens-cli"
  }"#;

/// Writes the six package directories into `target/npm-cli`, from the archives found in
/// `archives` (`jlens-<version or commit>-<target>.tar.gz`, `.zip` on Windows), then packs
/// each into a `.tgz` next to them. Every target must have its archive: a `jlens-cli` without
/// one of its platform packages must not be published.
pub fn build(archives: &Path) -> Result<()> {
    let version = workspace_version()?;
    let out = root().join("target/npm-cli");
    if out.exists() {
        fs::remove_dir_all(&out)?;
    }
    let mut optional = Vec::new();
    for (target, os, cpu) in PLATFORMS {
        let name = format!("jlens-cli-{os}-{cpu}");
        let exe = if os == "win32" { "jlens.exe" } else { "jlens" };
        let archive = find_archive(archives, target)?;
        let bytes = fs::read(&archive)?;
        let binary = if os == "win32" {
            zip_entry(&bytes, exe)
        } else {
            tar_gz_entry(&bytes, exe)
        }
        .with_context(|| format!("reading {}", archive.display()))?;

        let dir = out.join(&name);
        fs::create_dir_all(&dir)?;
        write_executable(&dir.join(exe), &binary)?;
        fs::copy(root().join("LICENSE"), dir.join("LICENSE"))?;
        fs::write(
            dir.join("README.md"),
            format!(
                "# {name}\n\nThe `jlens` binary of the jlens {version} release for `{target}`. \
                 Install [`jlens-cli`](https://www.npmjs.com/package/jlens-cli) instead: it \
                 depends on this package on that platform and runs the binary.\n"
            ),
        )?;
        // preferUnplugged: Yarn Plug'n'Play keeps packages zipped, and a zipped binary cannot run.
        fs::write(
            dir.join("package.json"),
            format!(
                r#"{{
  "name": "{name}",
  "version": "{version}",
  "description": "The jlens binary for {os} {cpu}, run by jlens-cli",
  "license": "MIT",
  "repository": {REPOSITORY},
  "homepage": "https://thkprado.github.io/jlens/",
  "os": ["{os}"],
  "cpu": ["{cpu}"],
  "files": ["{exe}"],
  "preferUnplugged": true
}}
"#
            ),
        )?;
        optional.push(format!(r#"    "{name}": "{version}""#));
    }

    let dir = out.join("jlens-cli");
    fs::create_dir_all(dir.join("bin"))?;
    let launcher = fs::read(root().join("crates/jlens-cli/npm/jlens.js"))?;
    write_executable(&dir.join("bin/jlens.js"), &launcher)?;
    fs::copy(
        root().join("crates/jlens-cli/npm/README.md"),
        dir.join("README.md"),
    )?;
    fs::copy(root().join("LICENSE"), dir.join("LICENSE"))?;
    fs::write(
        dir.join("package.json"),
        format!(
            r#"{{
  "name": "jlens-cli",
  "version": "{version}",
  "description": "Command-line interface for jlens: evaluate, check and explain JSONata expressions",
  "license": "MIT",
  "repository": {REPOSITORY},
  "homepage": "https://thkprado.github.io/jlens/",
  "bugs": "https://github.com/thkprado/jlens/issues",
  "bin": {{
    "jlens": "bin/jlens.js"
  }},
  "files": ["bin/"],
  "engines": {{
    "node": ">=18"
  }},
  "optionalDependencies": {{
{optional}
  }},
  "keywords": ["jsonata", "json", "query", "transform", "cli"]
}}
"#,
            optional = optional.join(",\n")
        ),
    )?;

    let platforms = PLATFORMS.map(|(_, os, cpu)| format!("jlens-cli-{os}-{cpu}"));
    for name in platforms.iter().map(String::as_str).chain(["jlens-cli"]) {
        let dir = out.join(name);
        let status = npm()
            .args(["pack", "--pack-destination", ".."])
            .current_dir(&dir)
            .status()
            .context("running npm pack")?;
        if !status.success() {
            bail!("npm pack failed in {}: {status}", dir.display());
        }
    }
    println!("npm packages of jlens {version} in {}", out.display());
    Ok(())
}

pub fn npm() -> Command {
    // npm is a batch file on Windows, which only cmd runs.
    if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "npm"]);
        cmd
    } else {
        Command::new("npm")
    }
}

pub fn find_archive(dir: &Path, target: &str) -> Result<PathBuf> {
    let suffixes = [format!("-{target}.tar.gz"), format!("-{target}.zip")];
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("jlens-") && suffixes.iter().any(|s| name.ends_with(s.as_str())) {
            found.push(path);
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => bail!("no archive for {target} in {}", dir.display()),
        _ => bail!("more than one archive for {target} in {}", dir.display()),
    }
}

pub fn write_executable(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// The content of the regular file `<dir>/<file>` of a gzipped tar archive (ustar or GNU, as
/// `tar czf` writes them; names are short, so no long-name records).
pub fn tar_gz_entry(bytes: &[u8], file: &str) -> Result<Vec<u8>> {
    let mut tar = Vec::new();
    flate2::read::GzDecoder::new(bytes).read_to_end(&mut tar)?;
    let mut at = 0;
    while at + 512 <= tar.len() {
        let header = &tar[at..at + 512];
        if header.iter().all(|&b| b == 0) {
            break;
        }
        let field = |range: std::ops::Range<usize>| {
            let raw = &header[range];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            String::from_utf8_lossy(&raw[..end]).into_owned()
        };
        let size = usize::from_str_radix(field(124..136).trim(), 8)
            .context("invalid size in a tar header")?;
        let name = field(0..100);
        let data = at + 512;
        if matches!(header[156], b'0' | 0) && is_entry(&name, file) {
            return tar
                .get(data..data + size)
                .map(<[u8]>::to_vec)
                .context("truncated tar archive");
        }
        at = data + size.div_ceil(512) * 512;
    }
    bail!("no {file} in the archive")
}

/// The content of `<dir>/<file>` in a zip archive, stored or deflated, found through its
/// central directory.
pub fn zip_entry(bytes: &[u8], file: &str) -> Result<Vec<u8>> {
    let u16_at = |at: usize| -> Result<usize> {
        let b = bytes.get(at..at + 2).context("truncated zip archive")?;
        Ok(u16::from_le_bytes([b[0], b[1]]) as usize)
    };
    let u32_at = |at: usize| -> Result<usize> {
        let b = bytes.get(at..at + 4).context("truncated zip archive")?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let end = (0..bytes.len().saturating_sub(21))
        .rev()
        .find(|&at| bytes[at..].starts_with(&[0x50, 0x4b, 0x05, 0x06]))
        .context("not a zip archive")?;
    let count = u16_at(end + 10)?;
    let mut at = u32_at(end + 16)?;
    for _ in 0..count {
        if u32_at(at)? != 0x0201_4b50 {
            bail!("invalid zip central directory");
        }
        let method = u16_at(at + 10)?;
        let compressed = u32_at(at + 20)?;
        let name_len = u16_at(at + 28)?;
        let skip = name_len + u16_at(at + 30)? + u16_at(at + 32)?;
        let local = u32_at(at + 42)?;
        let name = String::from_utf8_lossy(bytes.get(at + 46..at + 46 + name_len).unwrap_or(&[]))
            .replace('\\', "/");
        if is_entry(&name, file) {
            let start = local + 30 + u16_at(local + 26)? + u16_at(local + 28)?;
            let data = bytes
                .get(start..start + compressed)
                .context("truncated zip archive")?;
            return match method {
                0 => Ok(data.to_vec()),
                8 => {
                    let mut out = Vec::new();
                    flate2::read::DeflateDecoder::new(data).read_to_end(&mut out)?;
                    Ok(out)
                }
                _ => bail!("unsupported zip compression method {method}"),
            };
        }
        at += 46 + skip;
    }
    bail!("no {file} in the archive")
}

/// Whether an archive entry is `file` in the archive's single top directory.
fn is_entry(name: &str, file: &str) -> bool {
    name.split_once('/').is_some_and(|(_, rest)| rest == file)
}
