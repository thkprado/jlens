//! The crate's one exception to the workspace lint (ADR 0021): `unsafe` appears only in
//! `#[unsafe(no_mangle)]`, inside the exports' macro, and `allow(unsafe_code)` only on the
//! exports module. Every Rust file of the crate is scanned, not only `src/`, since a `#[path]`
//! module can live elsewhere. Comments and string literals are skipped.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "target") {
                rust_files(&path, files);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

/// The line with the contents of its string literals removed (one-line literals only, which is
/// all this crate has) and its comment cut off.
fn code(line: &str) -> String {
    let mut code = String::new();
    let (mut in_string, mut escaped) = (false, false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            match (escaped, c) {
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => escaped = false,
            }
        } else if c == '/' && chars.peek() == Some(&'/') {
            break;
        } else {
            in_string = c == '"';
            code.push(c);
        }
    }
    code
}

/// The lines of the crate's Rust files that contain `word`, as `path:line: text`.
fn lines_with(word: &str) -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(root, &mut files);
    files.sort();
    let mut found = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        for (number, line) in text.lines().enumerate() {
            let code = code(line);
            let mut words = code.split(|c: char| !(c.is_alphanumeric() || c == '_'));
            if words.any(|w| w == word) {
                let name = path.strip_prefix(root).unwrap().to_string_lossy();
                found.push(format!(
                    "{}:{}: {}",
                    name.replace('\\', "/"),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }
    found
}

#[test]
fn unsafe_only_in_the_exports_attribute() {
    let found = lines_with("unsafe");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0].starts_with("src/exports.rs:") && found[0].ends_with(": #[unsafe(no_mangle)]"),
        "{found:#?}"
    );
}

#[test]
fn the_lint_is_allowed_on_the_exports_module_only() {
    let found = lines_with("unsafe_code");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0].starts_with("src/lib.rs:") && found[0].ends_with(": #[allow(unsafe_code)]"),
        "{found:#?}"
    );
    let lib: Vec<_> = include_str!("../src/lib.rs")
        .lines()
        .map(str::trim)
        .collect();
    let allow = lib.iter().position(|line| *line == "#[allow(unsafe_code)]");
    assert_eq!(
        allow.and_then(|at| lib.get(at + 1)),
        Some(&"pub mod exports;"),
        "the allow is on the exports module"
    );
}
