//! `cargo xtask release-notes`: the GitHub release notes of a version, from `CHANGELOG.md`.

use std::fs;

use anyhow::{Context, Result, bail};

use crate::root;

/// The repository, which the relative links of the changelog point into.
const REPOSITORY: &str = "https://github.com/thkprado/jlens";

/// Appended to the notes of every release.
const INSTALL: &str = "\
### Install

```sh
cargo install jlens-cli                 # the jlens command, including jlens lsp
brew install thkprado/jlens/jlens       # the same, with Homebrew (macOS, Linux)
scoop bucket add jlens https://github.com/thkprado/scoop-jlens
scoop install jlens/jlens               # the same, with Scoop (Windows)
docker run --rm -i ghcr.io/thkprado/jlens --version   # the same, as a container
npx jlens-cli --version                 # the same, from npm
code --install-extension thkprado.jlens # the VS Code extension, with the same binary
cargo add jlens-core jlens-schema       # the Rust crates
npm install jlens-wasm                  # the browser ES module
pip install jlens                       # the Python package
```

- crates.io: [jlens-core](https://crates.io/crates/jlens-core), [jlens-schema](https://crates.io/crates/jlens-schema), [jlens-lsp](https://crates.io/crates/jlens-lsp), [jlens-cli](https://crates.io/crates/jlens-cli)
- npm: [jlens-wasm](https://www.npmjs.com/package/jlens-wasm), [jlens-cli](https://www.npmjs.com/package/jlens-cli)
- PyPI: [jlens](https://pypi.org/project/jlens/)
- Container: [ghcr.io/thkprado/jlens](https://github.com/thkprado/jlens/pkgs/container/jlens)
- VS Code: [Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=thkprado.jlens), [Open VSX](https://open-vsx.org/extension/thkprado/jlens)
- Every way to install: [installation](https://thkprado.github.io/jlens/getting-started/installation.html)
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

/// The section of `version` in the changelog, unwrapped and with its relative links made absolute
/// at the tag, followed by the install instructions and the comparison link of the version.
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
    let body = absolute_links(&body, &format!("{REPOSITORY}/blob/v{version}/"));
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

/// Prefixes the relative targets of inline links (`](docs/...)`) with `base`: the release page
/// resolves them against itself, where they break. Code blocks and code spans are kept as they
/// are. Runs on unwrapped text, so that a link split over two lines is whole.
fn absolute_links(body: &str, base: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut fenced = false;
    for line in body.split('\n') {
        let fence = line.trim_start().starts_with("```");
        fenced ^= fence;
        if fenced || fence {
            out.push(line.to_string());
            continue;
        }
        let mut text = String::with_capacity(line.len());
        let mut code = false;
        let mut rest = line;
        while let Some(at) = rest.find(['`', ']']) {
            let (before, mark, after) = (&rest[..at], &rest[at..=at], &rest[at + 1..]);
            text.push_str(before);
            text.push_str(mark);
            rest = after;
            if mark == "`" {
                code = !code;
            } else if let Some(target) = after.strip_prefix('(')
                && !code
                && is_relative(target)
            {
                text.push('(');
                text.push_str(base);
                rest = target.trim_start_matches("./");
            }
        }
        text.push_str(rest);
        out.push(text);
    }
    out.join("\n")
}

/// Whether a link target (the text after `](`) is a path in the repository, rather than a URL or
/// an anchor of the page.
fn is_relative(target: &str) -> bool {
    let end = target.find([')', ' ']).unwrap_or(target.len());
    let target = &target[..end];
    !(target.is_empty()
        || target.contains("://")
        || target.starts_with(['#', '/'])
        || target.starts_with("mailto:"))
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
    fn makes_relative_links_absolute_at_the_tag() {
        let changelog = "\
## [0.3.0]

- A new code ([ADR
  0010](docs/src/adr/0010-lazy-input.md)), see [the guide](./docs/src/guide.md#top).
- Kept: [site](https://example.com/a), [anchor](#install), `[code](docs/x.md)`.

```md
[block](docs/x.md)
```
";
        let notes = notes(changelog, "0.3.0").unwrap();
        let (body, _) = notes.split_once("\n\n### Install").unwrap();
        assert_eq!(
            body,
            "- A new code ([ADR 0010](https://github.com/thkprado/jlens/blob/v0.3.0/\
             docs/src/adr/0010-lazy-input.md)), see [the guide](https://github.com/thkprado/\
             jlens/blob/v0.3.0/docs/src/guide.md#top).\n\
             - Kept: [site](https://example.com/a), [anchor](#install), `[code](docs/x.md)`.\n\n\
             ```md\n[block](docs/x.md)\n```"
        );
    }

    #[test]
    fn rejects_a_missing_version() {
        assert!(notes(CHANGELOG, "0.9.0").is_err());
    }
}
