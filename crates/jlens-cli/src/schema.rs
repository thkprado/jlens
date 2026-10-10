//! `jlens schema …`: schema inference and language-service queries.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use jlens_schema::{LanguageService, Override, Schema};
use serde_json::Value;

use crate::{ExprSource, output, print_json, read_json};

#[derive(Subcommand)]
pub enum SchemaCommand {
    /// Infer a JSON Schema from sample documents.
    Infer(InferArgs),
    /// List completions for an expression at a cursor position.
    Complete(CompleteArgs),
}

#[derive(Args)]
pub struct InferArgs {
    /// Sample documents (JSON files; glob patterns such as `samples/*.json` are expanded).
    #[arg(required = true)]
    samples: Vec<String>,
    /// Treat a top-level array in a sample file as a list of documents.
    #[arg(long)]
    each: bool,
    /// Overrides to apply: a JSON object mapping paths (`items[].type`) to settings.
    #[arg(long, value_name = "FILE")]
    overrides: Option<PathBuf>,
    /// Write the schema to this file instead of stdout.
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,
}

#[derive(Args)]
pub struct CompleteArgs {
    #[command(flatten)]
    source: ExprSource,
    /// The schema (JSON Schema, e.g. from `jlens schema infer`).
    #[arg(long, value_name = "FILE")]
    schema: Option<PathBuf>,
    /// Cursor position, in characters from the start of the expression (defaults to the end).
    #[arg(long)]
    cursor: Option<usize>,
    /// Print the completions as JSON (with kinds, details, documentation and the range).
    #[arg(long)]
    json: bool,
}

pub fn run(cmd: SchemaCommand) -> Result<ExitCode> {
    match cmd {
        SchemaCommand::Infer(args) => infer(args),
        SchemaCommand::Complete(args) => complete(args),
    }
}

/// Reads a schema written as JSON Schema.
pub fn load(path: &PathBuf) -> Result<Schema> {
    let doc = read_json(path)?;
    Schema::from_json_schema(&doc).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
}

/// Expands glob patterns (Windows shells do not do it).
fn expand(patterns: &[String]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for pattern in patterns {
        if pattern.contains(['*', '?', '[']) {
            let matches: Vec<PathBuf> = glob::glob(pattern)
                .with_context(|| format!("invalid pattern {pattern}"))?
                .filter_map(Result::ok)
                .filter(|p| p.is_file())
                .collect();
            if matches.is_empty() {
                bail!("no files match {pattern}");
            }
            files.extend(matches);
        } else {
            files.push(PathBuf::from(pattern));
        }
    }
    Ok(files)
}

fn infer(args: InferArgs) -> Result<ExitCode> {
    let mut schema = Schema::new();
    for file in expand(&args.samples)? {
        match read_json(&file)? {
            Value::Array(docs) if args.each => docs.iter().for_each(|d| schema.observe(d)),
            doc => schema.observe(&doc),
        }
    }
    if let Some(path) = &args.overrides {
        let overrides: BTreeMap<String, Override> = serde_json::from_value(read_json(path)?)
            .with_context(|| format!("reading overrides from {}", path.display()))?;
        for failed in schema.apply_overrides(&overrides) {
            eprintln!("warning: override path {failed} could not be applied");
        }
    }
    let text = serde_json::to_string_pretty(&schema.to_json_schema())?;
    match &args.output {
        Some(path) => {
            fs::write(path, text + "\n").with_context(|| format!("writing {}", path.display()))?;
            eprintln!(
                "inferred from {} sample(s): {}",
                schema.samples,
                path.display()
            );
        }
        None => output::line(&text)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn complete(args: CompleteArgs) -> Result<ExitCode> {
    let (source, _) = args.source.read()?;
    let schema = args.schema.as_ref().map(load).transpose()?;
    let offset = match args.cursor {
        Some(chars) => source
            .char_indices()
            .nth(chars)
            .map_or(source.len(), |(i, _)| i),
        None => source.len(),
    };
    let completions = LanguageService::new(schema).complete(&source, offset);
    if args.json {
        print_json(&completions)?;
    } else {
        for item in &completions.items {
            let kind = serde_json::to_value(item.kind)?;
            let kind = kind.as_str().unwrap_or_default();
            match &item.detail {
                Some(detail) => output::line(&format!("{:<24} {kind:<9} {detail}", item.label))?,
                None => output::line(&format!("{:<24} {kind}", item.label))?,
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}
