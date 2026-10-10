//! `cargo xtask doc-check`: runs the JSONata examples of the documentation and checks its links.
//!
//! Examples are fenced blocks tagged `jsonata-example`:
//!
//! ````text
//! ```jsonata-example
//! input: address.json
//! Phone[type = 'mobile'].number
//! => "077 7700 1234"
//! ```
//! ````
//!
//! - Optional header lines come first. `input:` is followed by a file in `docs/examples/` or by
//!   inline JSON. `bindings:` is followed by a JSON object of variables. Without `input:`,
//!   there is no input document. `differs:` marks an example whose result differs in jsonata-js,
//!   with the reason; `benches/compare/doc-examples.mjs` skips it.
//! - The expression follows, on one or more lines, up to a line starting with `=>`.
//! - The expected result follows `=>`: JSON (which may span several lines), `undefined`,
//!   `error CODE`, or `*` for "any result" (for `$now()` and the like).

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use jlens_core::{Bindings, Expression, Options};
use serde_json::Value;

use crate::root;

pub fn check() -> Result<()> {
    let book = root().join("docs/src");
    let mut pages = Vec::new();
    collect_markdown(&book, &mut pages)?;
    pages.sort();
    // Markdown outside the book whose links are checked too.
    let mut others = vec![
        root().join("README.md"),
        root().join("examples/web-editor/README.md"),
    ];
    for entry in fs::read_dir(root().join("crates"))? {
        let readme = entry?.path().join("README.md");
        if readme.exists() {
            others.push(readme);
        }
    }

    let mut failures = Vec::new();
    let mut examples = 0;
    for page in &pages {
        let text = fs::read_to_string(page)?;
        examples += check_examples(page, &text, &mut failures)?;
    }
    let mut anchors = AnchorCache::default();
    for page in pages.iter().chain(&others) {
        let text = fs::read_to_string(page)?;
        check_links(page, &text, &mut anchors, &mut failures)?;
    }
    check_summary(&book, &pages, &mut failures)?;

    for failure in &failures {
        eprintln!("{failure}\n");
    }
    if !failures.is_empty() {
        bail!("{} documentation problem(s)", failures.len());
    }
    println!(
        "{examples} examples and the links of {} pages are correct",
        pages.len() + others.len()
    );
    Ok(())
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_markdown(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
    Ok(())
}

fn relative(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}

// --- Examples --------------------------------------------------------------------------------

struct Example {
    line: usize,
    input: Option<String>,
    bindings: Option<String>,
    expression: String,
    expected: String,
}

/// Runs the examples of one page; returns how many there were.
fn check_examples(page: &Path, text: &str, failures: &mut Vec<String>) -> Result<usize> {
    let mut count = 0;
    for example in examples(text) {
        count += 1;
        if let Err(problem) = run_example(&example) {
            failures.push(format!(
                "{}:{}: {problem}\n  expression: {}",
                relative(page),
                example.line,
                example.expression.replace('\n', "\n              ")
            ));
        }
    }
    Ok(count)
}

fn examples(text: &str) -> Vec<Example> {
    let mut out = Vec::new();
    let mut lines = text.lines().enumerate();
    while let Some((n, line)) = lines.next() {
        if line.trim_start() != "```jsonata-example" {
            continue;
        }
        // Examples inside list items are indented like their fence.
        let indent = line.len() - line.trim_start().len();
        let mut body = Vec::new();
        for (_, line) in lines.by_ref() {
            if line.trim_start().starts_with("```") {
                break;
            }
            let strip = line.len() - line.trim_start().len();
            body.push(&line[strip.min(indent)..]);
        }
        let mut example = Example {
            line: n + 1,
            input: None,
            bindings: None,
            expression: String::new(),
            expected: String::new(),
        };
        let mut rest = body.as_slice();
        while let Some((first, tail)) = rest.split_first() {
            if let Some(input) = first.strip_prefix("input:") {
                example.input = Some(input.trim().to_owned());
            } else if let Some(bindings) = first.strip_prefix("bindings:") {
                example.bindings = Some(bindings.trim().to_owned());
            } else if first.starts_with("differs:") {
                // A documented difference from jsonata-js (for `benches/compare/doc-examples.mjs`).
            } else {
                break;
            }
            rest = tail;
        }
        let split = rest.iter().position(|l| l.starts_with("=>"));
        let (expression, expected) = match split {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, &[][..]),
        };
        example.expression = expression.join("\n");
        example.expected = expected
            .join("\n")
            .strip_prefix("=>")
            .unwrap_or("")
            .trim()
            .to_owned();
        out.push(example);
    }
    out
}

fn run_example(example: &Example) -> Result<(), String> {
    if example.expected.is_empty() {
        return Err("no expected result (a line starting with `=>`)".into());
    }
    let input = match &example.input {
        None => None,
        Some(spec) if spec.starts_with(['{', '[', '"']) || spec.parse::<f64>().is_ok() => Some(
            serde_json::from_str::<Value>(spec)
                .map_err(|e| format!("invalid inline input: {e}"))?,
        ),
        Some(file) => {
            let path = root().join("docs/examples").join(file);
            let text = fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", relative(&path)))?;
            Some(serde_json::from_str(&text).map_err(|e| format!("invalid {file}: {e}"))?)
        }
    };
    let bindings: Bindings = match &example.bindings {
        None => Bindings::new(),
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => map.into_iter().collect(),
            _ => return Err("bindings must be a JSON object".into()),
        },
    };
    let options = Options {
        timeout: Some(Duration::from_secs(5)),
        max_depth: None,
    };
    let result = Expression::compile(&example.expression)
        .and_then(|expr| expr.evaluate_with(input.as_ref(), &bindings, &options));
    let expected = example.expected.as_str();
    match (result, expected) {
        (Ok(_), "*") => Ok(()),
        (Ok(None), "undefined") => Ok(()),
        (Err(err), code) if code.starts_with("error ") => {
            let want = code.trim_start_matches("error ").trim();
            if err.code() == want {
                Ok(())
            } else {
                Err(format!(
                    "expected error {want}, got {}: {}",
                    err.code(),
                    err.message()
                ))
            }
        }
        (Err(err), _) => Err(format!("failed with {}: {}", err.code(), err.message())),
        (Ok(actual), code) if code.starts_with("error ") => Err(format!(
            "expected {code}, got {}",
            actual.map_or("undefined".into(), |v| compact(&v))
        )),
        (Ok(actual), _) => {
            let want: Value = serde_json::from_str(expected)
                .map_err(|e| format!("the expected result is not JSON ({e}): {expected}"))?;
            match actual {
                Some(actual) if same(&actual, &want) => Ok(()),
                Some(actual) => Err(format!(
                    "expected {}\n  got      {}",
                    compact(&want),
                    compact(&actual)
                )),
                None => Err(format!("expected {}, got undefined", compact(&want))),
            }
        }
    }
}

/// JSON equality, with numbers compared as numbers (`1` equals `1.0`).
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y)
                    .all(|((ka, va), (kb, vb))| ka == kb && same(va, vb))
        }
        _ => a == b,
    }
}

fn compact(v: &Value) -> String {
    jlens_core::format_json(v, false)
}

// --- Links ----------------------------------------------------------------------------------

#[derive(Default)]
struct AnchorCache(HashMap<PathBuf, BTreeSet<String>>);

impl AnchorCache {
    fn anchors(&mut self, page: &Path) -> Result<&BTreeSet<String>> {
        if !self.0.contains_key(page) {
            let text = fs::read_to_string(page)?;
            self.0.insert(page.to_owned(), heading_ids(&text));
        }
        Ok(&self.0[page])
    }
}

/// The ids mdBook gives to the headings of a page.
fn heading_ids(text: &str) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut in_code = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        let trimmed = line.trim_start_matches('#');
        if trimmed.len() == line.len() || !trimmed.starts_with(' ') {
            continue;
        }
        let base: String = strip_links(trimmed.trim())
            .chars()
            .filter_map(|c| {
                if c.is_alphanumeric() || c == '_' || c == '-' {
                    Some(c.to_ascii_lowercase())
                } else if c.is_whitespace() {
                    Some('-')
                } else {
                    None
                }
            })
            .collect();
        let n = seen.entry(base.clone()).or_insert(0);
        let id = if *n == 0 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        *n += 1;
        ids.insert(id);
    }
    ids
}

/// `[text](target)` becomes `text`.
fn strip_links(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match (after.find("]("), after.find(')')) {
            (Some(close), Some(end)) if close < end => {
                out.push_str(&after[..close]);
                rest = &after[end + 1..];
            }
            _ => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn check_links(
    page: &Path,
    text: &str,
    anchors: &mut AnchorCache,
    failures: &mut Vec<String>,
) -> Result<()> {
    let mut in_code = false;
    for (n, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        for target in link_targets(line) {
            if target.contains("://") || target.starts_with("mailto:") {
                continue;
            }
            let (file, anchor) = match target.split_once('#') {
                Some((file, anchor)) => (file, Some(anchor)),
                None => (target.as_str(), None),
            };
            let resolved = if file.is_empty() {
                page.to_owned()
            } else {
                page.parent().unwrap().join(file)
            };
            let problem = if !resolved.exists() {
                Some(format!("broken link `{target}`: no such file"))
            } else if let Some(anchor) = anchor
                && resolved.extension().is_some_and(|e| e == "md")
                && !anchors
                    .anchors(&resolved.canonicalize().context("resolving a link")?)?
                    .contains(anchor)
            {
                Some(format!("broken link `{target}`: no heading `#{anchor}`"))
            } else {
                None
            };
            if let Some(problem) = problem {
                failures.push(format!("{}:{}: {problem}", relative(page), n + 1));
            }
        }
    }
    Ok(())
}

/// The targets of the inline links of a line, outside code spans.
fn link_targets(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut code = false;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'`' => code = !code,
            b']' if !code && bytes.get(i + 1) == Some(&b'(') => {
                if let Some(end) = line[i + 2..].find(')') {
                    let target = line[i + 2..i + 2 + end].trim();
                    // `[text](target "title")`
                    let target = target.split_whitespace().next().unwrap_or("");
                    if !target.is_empty() {
                        out.push(target.to_owned());
                    }
                    i += 2 + end;
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// Every page of the book must be listed in SUMMARY.md.
fn check_summary(book: &Path, pages: &[PathBuf], failures: &mut Vec<String>) -> Result<()> {
    let summary = fs::read_to_string(book.join("SUMMARY.md"))?;
    let listed: BTreeSet<String> = link_targets_all(&summary).into_iter().collect();
    for page in pages {
        let rel = page
            .strip_prefix(book)
            .unwrap()
            .display()
            .to_string()
            .replace('\\', "/");
        if rel != "SUMMARY.md" && !listed.contains(&rel) {
            failures.push(format!("docs/src/{rel}: not listed in SUMMARY.md"));
        }
    }
    Ok(())
}

fn link_targets_all(text: &str) -> Vec<String> {
    text.lines().flat_map(link_targets).collect()
}
