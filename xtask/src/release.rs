//! `cargo xtask release-notes`: the GitHub release notes of a version, from `CHANGELOG.md`.

use std::fs;

use anyhow::{Context, Result, bail};

use crate::root;

/// Appended to the notes of every release.
const INSTALL: &str = "\
### Install

```sh
cargo install jlens-cli                 # the jlens command, including jlens lsp
cargo add jlens-core jlens-schema       # the Rust crates
npm install jlens-wasm                  # the browser ES module
```

- crates.io: [jlens-core](https://crates.io/crates/jlens-core), [jlens-schema](https://crates.io/crates/jlens-schema), [jlens-lsp](https://crates.io/crates/jlens-lsp), [jlens-cli](https://crates.io/crates/jlens-cli)
- npm: [jlens-wasm](https://www.npmjs.com/package/jlens-wasm)
- API reference: [docs.rs/jlens-core](https://docs.rs/jlens-core), [docs.rs/jlens-schema](https://docs.rs/jlens-schema), [docs.rs/jlens-lsp](https://docs.rs/jlens-lsp)
";

/// Prints the release notes of `version` (`0.3.0` or `v0.3.0`).
pub fn print(version: &str) -> Result<()> {
    let path = root().join("CHANGELOG.md");
    let changelog = fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?
        .replace("\r\n", "\n");
    print!("{}", notes(&changelog, version.trim_start_matches('v'))?);
    Ok(())
}

/// The section of `version` in the changelog, unwrapped, followed by the install instructions and
/// the comparison link of the version.
fn notes(changelog: &str, version: &str) -> Result<String> {
    let heading = format!("## [{version}]");
    let mut lines = changelog.lines().skip_while(|line| *line != heading);
    if lines.next().is_none() {
        bail!("CHANGELOG.md has no `{heading}` section");
    }
    let section: Vec<&str> = lines
        .take_while(|line| !line.starts_with("## ") && !is_link_definition(line))
        .collect();
    let body = unwrap(&section);
    if body.is_empty() {
        bail!("the `{heading}` section of CHANGELOG.md is empty");
    }
    let mut out = format!("{body}\n\n{INSTALL}");
    let link = format!("[{version}]: ");
    if let Some(url) = changelog.lines().find_map(|line| line.strip_prefix(&link))
        && url.contains("/compare/")
    {
        out.push_str(&format!("- Full changelog: {url}\n"));
    }
    Ok(out)
}

fn is_link_definition(line: &str) -> bool {
    line.starts_with('[') && line.contains("]: ")
}

/// Joins the hard-wrapped lines of paragraphs and list items, which GitHub would show as line
/// breaks. Code blocks are kept as they are.
fn unwrap(lines: &[&str]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut fenced = false;
    let mut joinable = false;
    for line in lines {
        let text = line.trim_start();
        if text.starts_with("```") {
            fenced = !fenced;
            joinable = false;
            out.push(line.to_string());
        } else if fenced || text.is_empty() || text.starts_with('#') {
            joinable = false;
            out.push(line.to_string());
        } else if joinable && !text.starts_with("- ") && !text.starts_with("* ") {
            let last = out.last_mut().expect("a joinable line follows another");
            last.push(' ');
            last.push_str(text);
        } else {
            joinable = true;
            out.push(line.to_string());
        }
    }
    out.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANGELOG: &str = "\
# Changelog

## [Unreleased]

- Something new.

## [0.2.0]

Host functions.

### Added

- Host functions: the application
  provides its own functions.
  - A nested
    item.

```sh
cargo add
  jlens-core
```

## [0.1.0]

The first
release.

[Unreleased]: https://github.com/thkprado/jlens/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/thkprado/jlens/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/thkprado/jlens/releases/tag/v0.1.0
";

    #[test]
    fn unwraps_the_section_and_links_the_comparison() {
        let notes = notes(CHANGELOG, "0.2.0").unwrap();
        let (body, rest) = notes.split_once("### Install").unwrap();
        assert_eq!(
            body,
            "Host functions.\n\n### Added\n\n\
             - Host functions: the application provides its own functions.\n  \
             - A nested item.\n\n```sh\ncargo add\n  jlens-core\n```\n\n"
        );
        assert!(rest.ends_with(
            "- Full changelog: https://github.com/thkprado/jlens/compare/v0.1.0...v0.2.0\n"
        ));
    }

    #[test]
    fn stops_at_the_link_definitions() {
        let notes = notes(CHANGELOG, "0.1.0").unwrap();
        assert!(notes.starts_with("The first release.\n\n### Install"));
        assert!(!notes.contains("Full changelog"));
    }

    #[test]
    fn rejects_a_missing_version() {
        assert!(notes(CHANGELOG, "0.9.0").is_err());
    }
}
