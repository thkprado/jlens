//! `cargo xtask fuzz-seed`: seeds the corpora of the fuzz targets (`fuzz/`) with the cases of
//! the JSONata suite, and with a few regular expressions and pictures. `cargo xtask fuzz-replay`:
//! runs saved inputs through a target, without libFuzzer.

use std::collections::BTreeSet;
use std::process::Command;
use std::{env, fs};

use anyhow::{Context, Result, bail};
use serde_json::Value as JsonValue;

/// Regular expressions, as the `regex` target reads them: flag bits (`i` is 1, `m` 2, `s` 4,
/// `u` 8, `v` 16), then the pattern and the text, separated by a NUL byte.
const REGEXES: &[(u8, &str, &str)] = &[
    (0, r"a(b|c)*d", "xabcbd abd"),
    (1, r"^(?<word>\w+)\s\k<word>$", "Hello hello"),
    (2, r"^\d+$", "12\n34\nx"),
    (4, r"a.b", "a\nb"),
    (8, r"\p{L}+|\u{1D11E}", "héllo 𝄞"),
    (16, r"[\p{L}--[a-z]]", "aBc"),
    (0, r"(?<=\$)\d+(?:\.\d\d)?|(?!x)[^\s]", "$12.50 x"),
    (0, r"^(a+)+$", "aaaaaaaaaaaaaaaaaaaaaaaaa!"),
    (9, r"(?:)", "𝄞é"),
    (0, r"[\b]\B\cJ\x41\0", "\u{8}\nA\0"),
];

/// Pictures, as the `pictures` target reads them: a picture, then a NUL byte and the text to
/// parse or a time zone.
const PICTURES: &[(&str, &str)] = &[
    ("#,##0.00", ""),
    ("#,##0.00;(#,##0.00)", ""),
    ("0.000e0", ""),
    ("00%", ","),
    ("###\u{2030}", ""),
    ("#", "1234"),
    ("w", "one thousand, two hundred and thirty-four"),
    ("I", "MCCXXXIV"),
    ("A", "AB"),
    ("#0;o", "12th"),
    ("[Y0001]-[M01]-[D01]", "2018-10-21"),
    (
        "[Y0001]-[M01]-[D01]T[H01]:[m01]:[s01].[f001][Z01:01t]",
        "+0100",
    ),
    ("[FNn], [D1o] [MNn] [Y] [h]:[m01] [PN] [z]", "-0500"),
    ("[Dwo] [M,3-3] [Y,2] [X0001]-W[W01]-[F1] [x] [E] [C]", ""),
    ("[H01]:[m01]:[s01]", "13:05:00"),
    ("[D1] [MNn] [Y0001]", "21 October 2018"),
    ("[Y]-[d]", "2018-294"),
    ("[[[D]]]", ""),
];

/// Writes the expressions of the suite to `fuzz/corpus/parse` and `fuzz/corpus/parse_recover`;
/// each expression followed by a NUL byte and its input document to `fuzz/corpus/evaluate`; each
/// expression after its length, as the offset, to `fuzz/corpus/language_service` (the input
/// formats of those targets); and the regular expressions and pictures above to theirs.
pub fn seed() -> Result<()> {
    let mut expressions = BTreeSet::new();
    let mut evaluations = BTreeSet::new();
    let mut services = BTreeSet::new();
    for case in suite_cases()? {
        let Some(expr) = case.get("expr").and_then(JsonValue::as_str) else {
            continue;
        };
        expressions.insert(expr.as_bytes().to_vec());
        let mut evaluation = expr.as_bytes().to_vec();
        if let Some(data) = case.get("data") {
            evaluation.push(0);
            evaluation.extend(serde_json::to_vec(data)?);
        }
        evaluations.insert(evaluation);
        // Completion and hover at the end of the expression, where an editor's cursor is.
        let mut service = u16::try_from(expr.len())
            .unwrap_or(0)
            .to_le_bytes()
            .to_vec();
        service.extend(expr.as_bytes());
        services.insert(service);
    }
    write("parse", &expressions)?;
    write("parse_recover", &expressions)?;
    write("evaluate", &evaluations)?;
    write("language_service", &services)?;
    let regexes = REGEXES.iter().map(|(flags, pattern, text)| {
        let mut input = vec![*flags];
        input.extend(pattern.as_bytes());
        input.push(0);
        input.extend(text.as_bytes());
        input
    });
    write("regex", &regexes.collect())?;
    let pictures = PICTURES.iter().map(|(picture, text)| {
        let mut input = picture.as_bytes().to_vec();
        if !text.is_empty() {
            input.push(0);
            input.extend(text.as_bytes());
        }
        input
    });
    write("pictures", &pictures.collect())
}

/// Runs saved inputs through the body of a fuzz target with the `replay` program of `fuzz/`,
/// which does not link libFuzzer. It builds with optimizations and debug assertions, as the
/// `Fuzz` workflow does: an unoptimized build has larger stack frames than the 1 MB thread was
/// sized for.
pub fn replay(args: &[String]) -> Result<()> {
    if args.len() < 2 {
        bail!(
            "fuzz-replay needs a target and the inputs to run\n\n{}",
            super::USAGE
        );
    }
    let status = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["run", "--quiet", "--release", "--manifest-path"])
        .arg(super::root().join("fuzz/Cargo.toml"))
        .args([
            "--no-default-features",
            "--features",
            "replay",
            "--bin",
            "replay",
            "--",
        ])
        .args(args)
        .env("CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS", "true")
        .env("CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS", "true")
        .status()
        .context("running the replay program")?;
    if !status.success() {
        bail!("replaying {} failed", args[0]);
    }
    Ok(())
}

fn suite_cases() -> Result<Vec<JsonValue>> {
    let mut cases = Vec::new();
    let groups = super::root().join("tests/jsonata-suite/groups");
    for group in fs::read_dir(&groups).with_context(|| format!("reading {}", groups.display()))? {
        for file in fs::read_dir(group?.path())? {
            let path = file?.path();
            if path.extension().is_some_and(|e| e == "json") {
                // A few cases hold lone surrogates, which serde_json rejects: skip them.
                match serde_json::from_str(&fs::read_to_string(&path)?) {
                    Ok(JsonValue::Array(items)) => cases.extend(items),
                    Ok(case) => cases.push(case),
                    Err(_) => {}
                }
            }
        }
    }
    Ok(cases)
}

fn write(target: &str, inputs: &BTreeSet<Vec<u8>>) -> Result<()> {
    let dir = super::root().join("fuzz/corpus").join(target);
    fs::create_dir_all(&dir)?;
    for (i, input) in inputs.iter().enumerate() {
        fs::write(dir.join(format!("suite-{i:04}")), input)?;
    }
    println!("{target}: {} inputs in fuzz/corpus/{target}", inputs.len());
    Ok(())
}
