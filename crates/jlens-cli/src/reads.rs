//! `jlens reads`: the fields an expression reads of its input and bindings (ADR 0016).

use std::fs;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Args;
use jlens_core::reads::Reads;
use jlens_core::{Diagnostic, Expression};
use serde_json::{Map, Value};

use crate::{print_json, report, schema};

#[derive(Args)]
pub struct ReadsArgs {
    /// The JSONata expression (omit when using --file).
    #[arg(required_unless_present = "files")]
    expr: Option<String>,
    /// Read expressions from files: several, or glob patterns such as `rules/*.jsonata`. The
    /// result is an object keyed by file.
    #[arg(short, long = "file", value_name = "FILE", num_args = 1.., conflicts_with = "expr")]
    files: Vec<String>,
    /// Print the union of the files' results: what an input must keep for all of them.
    #[arg(long, requires = "files")]
    union: bool,
}

pub fn run(args: ReadsArgs) -> Result<ExitCode> {
    // Read every expression first, so that an unreadable file prints nothing on stdout.
    let sources = match &args.expr {
        Some(expr) => vec![(expr.clone(), "<expr>".to_owned())],
        None => schema::expand(&args.files)?
            .into_iter()
            .map(|file| {
                let source = fs::read_to_string(&file)
                    .with_context(|| format!("reading {}", file.display()))?;
                Ok((source, file.display().to_string()))
            })
            .collect::<Result<Vec<_>>>()?,
    };
    // Compile them all, so that every syntax error is reported before anything is printed.
    let mut results = Vec::new();
    let mut failed = false;
    for (source, name) in &sources {
        match Expression::compile(source) {
            Ok(expr) => results.push((name, expr.reads())),
            Err(err) => {
                eprint!("{}", report::render(source, name, &Diagnostic::from(&err)));
                failed = true;
            }
        }
    }
    if failed {
        return Ok(ExitCode::from(1));
    }
    if args.expr.is_some() {
        print_json(&results[0].1)?;
    } else if args.union {
        let union = results
            .iter()
            .fold(Reads::default(), |union, (_, reads)| union.union(reads));
        print_json(&union)?;
    } else {
        let mut object = Map::new();
        for (name, reads) in results {
            object.insert(name.clone(), serde_json::to_value(reads)?);
        }
        print_json(&Value::Object(object))?;
    }
    Ok(ExitCode::SUCCESS)
}
