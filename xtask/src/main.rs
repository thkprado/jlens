//! Repository automation: `cargo xtask <command>`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use anyhow::{Context, Result, bail};

mod ab;
// Also built on its own by `ab`, against each revision it compares.
#[allow(dead_code)]
mod ab_probe;
mod differential;
mod docs;
mod fuzz;
mod homebrew;
mod npm;
mod release;
mod scoop;
mod site;
mod vscode;
mod wasm;

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  conformance [--update-baseline]   run the whole JSONata suite and write tests/conformance/REPORT.md
  update-suite <jsonata-js checkout> replace tests/jsonata-suite with <checkout>/test/test-suite
  gen-docs [--check]                regenerate the generated pages of the book (errors, functions)
  doc-check                         run the examples of the book and check its links
  differential [--cases N] [--seed S]
                                    run N generated expressions (1000) in jlens and jsonata-js
                                    and report where they differ (needs benches/compare installed)
  fuzz-seed                         seed the corpora of the fuzz targets with the JSONata suite
  fuzz-replay <target> <path>...    run saved inputs (files, or directories of them) through a
                                    fuzz target, without libFuzzer, on any platform
  homebrew <SHA256SUMS> [--url <base>]
                                    print the Homebrew formula of the CLI, for the archives
                                    listed in a release's SHA256SUMS, downloaded from the
                                    GitHub release or from <base>
  npm-cli <archives directory>      build the npm packages of the CLI (jlens-cli and one per
                                    platform) from the release archives, into target/npm-cli,
                                    and pack them
  scoop <SHA256SUMS> [--url <base>]
                                    print the Scoop manifest of the CLI, for the Windows archive
                                    listed in a release's SHA256SUMS, downloaded from the
                                    GitHub release or from <base>
  vscode <archives directory>       build the VS Code extension's packages (one VSIX per target
                                    with its binary, and one that uses the PATH) from the
                                    release archives, into target/vscode
  wasm [--dev]                      build the browser package in crates/jlens-wasm/pkg
  wasm-test [--require-browser] [--example]
                                    build the package and test it in Node.js and a headless browser;
                                    --example also loads the web editor, from the repository and
                                    as the site assembles it
  serve [--site]                    serve the repository over HTTP (for examples/web-editor),
                                    or with --site the site built by `site`
  site                              build the book and the web editor into target/site
  release-notes <version>           print the GitHub release notes of <version>, from CHANGELOG.md
  ab <rev A> <rev B> [--rounds N] [--mode evaluate|document] [--case NAME]... [--sample-ms MS]
     [--b-allocator system|mimalloc] [--target TRIPLE]
                                    compare the speed of two revisions (`.` is the working tree)
                                    on benches/cases.json, by running them in alternation; B can
                                    use mimalloc, and both can be built for another local target
";

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("conformance") => conformance(args.iter().any(|a| a == "--update-baseline")),
        Some("gen-docs") => gen_docs(args.iter().any(|a| a == "--check")),
        Some("doc-check") => docs::check(),
        Some("differential") => differential::run(&args[1..]),
        Some("fuzz-seed") => fuzz::seed(),
        Some("fuzz-replay") => fuzz::replay(&args[1..]),
        Some("homebrew") => match args.get(1) {
            Some(sums) => homebrew::print(
                Path::new(sums),
                args.iter()
                    .position(|a| a == "--url")
                    .and_then(|i| args.get(i + 1))
                    .map(String::as_str),
            ),
            None => bail!(
                "homebrew needs the path of a SHA256SUMS file

{USAGE}"
            ),
        },
        Some("npm-cli") => match args.get(1) {
            Some(dir) => npm::build(Path::new(dir)),
            None => bail!(
                "npm-cli needs the directory of the release archives

{USAGE}"
            ),
        },
        Some("scoop") => match args.get(1) {
            Some(sums) => scoop::print(
                Path::new(sums),
                args.iter()
                    .position(|a| a == "--url")
                    .and_then(|i| args.get(i + 1))
                    .map(String::as_str),
            ),
            None => bail!(
                "scoop needs the path of a SHA256SUMS file

{USAGE}"
            ),
        },
        Some("vscode") => match args.get(1) {
            Some(dir) => vscode::build(Path::new(dir)),
            None => bail!(
                "vscode needs the directory of the release archives

{USAGE}"
            ),
        },
        Some("wasm") => wasm::build(args.iter().any(|a| a == "--dev")),
        Some("wasm-test") => wasm::test(
            args.iter().any(|a| a == "--require-browser"),
            args.iter().any(|a| a == "--example"),
        ),
        Some("ab") => ab::run(&args[1..]),
        Some("serve") => wasm::serve_forever(args.iter().any(|a| a == "--site")),
        Some("site") => site::build(),
        Some("release-notes") => match args.get(1) {
            Some(version) => release::print(version),
            None => bail!("release-notes needs a version\n\n{USAGE}"),
        },
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

/// The only edit made to the vendored suite: a company in the sample data is renamed (text,
/// replacement) in these files. It does not change what the cases test.
const SUITE_EDITS: ((&str, &str), &[&str]) = (
    ("IBM UK", "Acme UK"),
    &[
        "datasets/dataset21.json",
        "groups/transform/case036.json",
        "groups/transform/case037.json",
    ],
);

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
    let ((from, to), files) = SUITE_EDITS;
    for relative in files {
        let path = dest.join(relative);
        let text = fs::read_to_string(&path)?;
        if text.contains(from) {
            fs::write(&path, text.replace(from, to))?;
        } else {
            println!("note: {relative} no longer contains {from:?}; review SUITE_EDITS");
        }
    }
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

/// Generates reference pages from the engine's own metadata, or checks they are up to date.
fn gen_docs(check: bool) -> Result<()> {
    let pages = [
        ("docs/src/language/errors.md", errors_page()),
        ("docs/src/language/functions.md", functions_page()),
    ];
    for (relative, generated) in pages {
        let path = root().join(relative);
        if check {
            let current = fs::read_to_string(&path).unwrap_or_default();
            if current.replace("\r\n", "\n") != generated {
                bail!(
                    "{} is out of date; run `cargo xtask gen-docs`",
                    path.display()
                );
            }
        } else {
            fs::write(&path, generated)?;
            println!("wrote {}", path.display());
        }
    }
    if check {
        println!("generated docs are up to date");
    }
    Ok(())
}

fn errors_page() -> String {
    use jlens_core::error::{Category, catalog};
    let mut out = String::from(
        "# Error reference\n\n\
         <!-- Generated by `cargo xtask gen-docs` from `jlens_core::error::catalog()`. Do not edit. -->\n\n\
         Every error jlens reports carries a stable code. The codes and messages match those of\n\
         jsonata-js, so code that handles JSONata errors works unchanged. In a message template,\n\
         `{{name}}` is replaced with the JSON form of the error detail `name`, and `{{{name}}}`\n\
         with its raw text.\n\n\
         Errors expose:\n\n\
         - `code()`;\n\
         - `message()`;\n\
         - `position()`, a UTF-16 offset just past the offending token, as in jsonata-js;\n\
         - `span()`, the byte range of the offending text;\n\
         - details such as `token()` and `value()`.\n",
    );
    let sections = [
        (
            Category::Static,
            "Static errors (S)",
            "Raised while compiling an expression.",
        ),
        (
            Category::Type,
            "Type errors (T)",
            "Raised while evaluating, when a value has the wrong type.",
        ),
        (
            Category::Dynamic,
            "Dynamic errors (D)",
            "Raised while evaluating, for other run-time problems.",
        ),
        (
            Category::Guardrail,
            "Guardrail errors (U)",
            "Raised when an evaluation exceeds its configured limits, is cancelled, or needs what jlens does not support. \
             `U2001` is raised by `jlens eval --json` for an input document that is not valid JSON.",
        ),
        (
            Category::Warning,
            "Warnings (W)",
            "jlens-specific. Warnings are reported as diagnostics by `jlens check` and `parse_recover`, and never stop evaluation.",
        ),
    ];
    for (category, title, intro) in sections {
        out.push_str(&format!(
            "\n## {title}\n\n{intro}\n\n| Code | Message |\n|---|---|\n"
        ));
        for entry in catalog().iter().filter(|e| e.category() == category) {
            let template = entry.template.replace('|', r"\|");
            out.push_str(&format!("| `{}` | `{}` |\n", entry.code, template));
        }
    }
    out
}

fn functions_page() -> String {
    use jlens_core::Category;
    let mut out = String::from(
        "# Function reference\n\n\
         <!-- Generated by `cargo xtask gen-docs` from `jlens_core::function_catalog()`. Do not edit. -->\n\n\
         jlens implements every built-in function of JSONata 2.2. Each function has a *signature*\n\
         that describes its parameters and result, for example `<s-nn?:s>`:\n\n\
         | Symbol | Meaning |\n|---|---|\n\
         | `s` `n` `b` `l` `o` | string, number, boolean, null, object |\n\
         | `a` | array; a single value is wrapped in an array |\n\
         | `a<n>` | array of numbers (any type can be given in angle brackets) |\n\
         | `f` | function |\n\
         | `j` | any JSON value; `x` any value |\n\
         | `(sf)` | either of the types |\n\
         | `?` | optional parameter |\n\
         | `+` | one or more |\n\
         | `-` | when the argument is missing, the context value is used |\n\
         | `:s` | the result type |\n\n\
         Arguments that do not match the signature raise `T0410` (wrong type), `T0411` (the context\n\
         value has the wrong type) or `T0412` (an array item has the wrong type).\n",
    );
    // (category, title, page with the detailed descriptions)
    let sections = [
        (Category::String, "String functions", "string"),
        (Category::Numeric, "Numeric functions", "numeric"),
        (
            Category::Aggregation,
            "Aggregation functions",
            "aggregation",
        ),
        (Category::Boolean, "Boolean functions", "boolean"),
        (Category::Array, "Array functions", "array"),
        (Category::Object, "Object functions", "object"),
        (
            Category::HigherOrder,
            "Higher-order functions",
            "higher-order",
        ),
        (Category::DateTime, "Date and time functions", "date-time"),
        (Category::Encoding, "Encoding functions", "encoding"),
        (Category::Misc, "Other functions", "misc"),
    ];
    let catalog = jlens_core::function_catalog();
    for (category, title, page) in sections {
        out.push_str(&format!(
            "\n## {title}\n\nDetails and examples: [{title}](functions/{page}.md).\n\n\
             | Function | Signature | Description |\n|---|---|---|\n"
        ));
        for f in catalog.iter().filter(|f| f.category == category) {
            let params: Vec<String> = f.params.iter().map(|p| format!("${p}")).collect();
            // The detailed pages have a `$name` heading per function, whose id is the
            // lowercased name.
            out.push_str(&format!(
                "| [`${}({})`](functions/{page}.md#{}) | `{}` | {} |\n",
                f.name,
                params.join(", "),
                f.name.to_lowercase(),
                f.signature,
                f.summary
            ));
        }
    }
    out
}
