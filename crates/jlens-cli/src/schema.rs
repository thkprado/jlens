//! `jlens schema …`: schema inference and language-service queries.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use jlens_core::{Diagnostic, Expression};
use jlens_lsp::config;
use jlens_schema::{
    Change, EnumChange, JsonType, LanguageService, Override, PathChanges, Schema, Use, UseChanges,
    Uses,
};
use serde_json::Value;

use crate::{ExprSource, output, print_json, read_json, report};

#[derive(Subcommand)]
pub enum SchemaCommand {
    /// Infer a JSON Schema from sample documents.
    Infer(InferArgs),
    /// List completions for an expression at a cursor position.
    Complete(CompleteArgs),
    /// Print what an expression uses of its schema, as JSON: what the schema says of each
    /// field the expression reads. Keep it to compare when the schema changes.
    Uses(UsesArgs),
    /// Print what changed between two answers of `schema uses` for one expression: that of
    /// the schema it was written against, and that of a new version.
    Changes(ChangesArgs),
    /// Print what the typing knows of an expression's result, as JSON: the types it may have,
    /// and what may make it undefined or several values; with an expected type, how the result
    /// compares with it.
    Result(ResultArgs),
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

#[derive(Args)]
pub struct UsesArgs {
    #[command(flatten)]
    source: ExprSource,
    /// The schema of the input (JSON Schema, e.g. from `jlens schema infer`). It replaces the
    /// schema that the configuration gives the file.
    #[arg(long, value_name = "FILE")]
    schema: Option<PathBuf>,
    /// The configuration that gives files a schema, variables and host functions, as for
    /// `jlens check` [default: the `jlens.json` of the current directory, if there is one].
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Give the expression of the argument the configuration of this file, as if it were its
    /// contents. The file need not exist.
    #[arg(long, value_name = "FILE", conflicts_with = "file")]
    path: Option<PathBuf>,
}

#[derive(Args)]
pub struct ResultArgs {
    #[command(flatten)]
    source: ExprSource,
    /// The schema of the input (JSON Schema, e.g. from `jlens schema infer`). It replaces the
    /// schema that the configuration gives the file.
    #[arg(long, value_name = "FILE")]
    schema: Option<PathBuf>,
    /// The configuration that gives files a schema, variables, host functions, an expected
    /// result type and a context, as for `jlens check` [default: the `jlens.json` of the
    /// current directory, if there is one].
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Give the expression of the argument the configuration of this file, as if it were its
    /// contents. The file need not exist.
    #[arg(long, value_name = "FILE", conflicts_with = "file")]
    path: Option<PathBuf>,
    #[command(flatten)]
    expect: crate::Expect,
}

#[derive(Args)]
pub struct ChangesArgs {
    /// The answer of `jlens schema uses` with the schema the expression was written against.
    #[arg(value_name = "OLD")]
    old: PathBuf,
    /// The answer of `jlens schema uses` for the same expression with the new schema.
    #[arg(value_name = "NEW")]
    new: PathBuf,
    /// Print the changes as JSON: `{}` when nothing the expression uses changed.
    #[arg(long)]
    json: bool,
}

pub fn run(cmd: SchemaCommand) -> Result<ExitCode> {
    match cmd {
        SchemaCommand::Infer(args) => infer(args),
        SchemaCommand::Complete(args) => complete(args),
        SchemaCommand::Uses(args) => uses(args),
        SchemaCommand::Changes(args) => changes(args),
        SchemaCommand::Result(args) => result(args),
    }
}

/// Reads a schema written as JSON Schema, following its `$ref`s into the files beside it, and
/// prints on stderr those it did not follow.
pub fn load(path: &Path) -> Result<Schema> {
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let (schema, report) =
        config::load_schema(&crate::absolute(&cwd, path)).map_err(anyhow::Error::msg)?;
    warn(&config::notes(
        &format!("schema {}", path.display()),
        &report,
    ));
    Ok(schema)
}

/// Prints on stderr what the schemas of a service did not follow.
pub fn warn(notes: &[String]) {
    for note in notes {
        eprintln!("warning: {note}");
    }
}

/// Expands glob patterns (Windows shells do not do it).
pub fn expand(patterns: &[String]) -> Result<Vec<PathBuf>> {
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
        for missing in schema.missing_override_paths(&overrides) {
            match &missing.suggestion {
                Some(s) => eprintln!(
                    "warning: override path {} is not in the samples; declared (did you mean {s}?)",
                    missing.path
                ),
                None => eprintln!(
                    "warning: override path {} is not in the samples; declared",
                    missing.path
                ),
            }
        }
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
    let schema = args.schema.as_deref().map(load).transpose()?;
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

fn uses(args: UsesArgs) -> Result<ExitCode> {
    let (source, name) = args.source.read()?;
    let file = args.source.file.as_deref().or(args.path.as_deref());
    let service = crate::service_for(file, args.schema.as_ref(), args.config.as_ref())?;
    match Expression::compile(&source) {
        Ok(expression) => {
            print_json(&service.uses_of(&expression))?;
            Ok(ExitCode::SUCCESS)
        }
        Err(err) => {
            eprint!(
                "{}",
                report::render(&source, &name, &Diagnostic::from(&err))
            );
            Ok(ExitCode::from(1))
        }
    }
}

fn result(args: ResultArgs) -> Result<ExitCode> {
    let (source, name) = args.source.read()?;
    let file = args.source.file.as_deref().or(args.path.as_deref());
    let service = crate::service_for(file, args.schema.as_ref(), args.config.as_ref())?;
    let service = args.expect.apply(service);
    let errors: Vec<Diagnostic> = service
        .diagnostics(&source)
        .into_iter()
        .filter(|d| d.severity == jlens_core::Severity::Error)
        .collect();
    if !errors.is_empty() {
        for error in &errors {
            eprint!("{}", report::render(&source, &name, error));
        }
        return Ok(ExitCode::from(1));
    }
    let result = service.result_type(&source);
    let mut json = serde_json::to_value(&result)?;
    if let (Some(expected), Value::Object(object)) = (service.expected_result(), &mut json) {
        object.insert(
            "check".into(),
            serde_json::to_value(result.check(expected))?,
        );
    }
    print_json(&json)?;
    Ok(ExitCode::SUCCESS)
}

fn changes(args: ChangesArgs) -> Result<ExitCode> {
    let read = |path: &PathBuf| {
        Uses::from_json(&read_json(path)?).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
    };
    let changes = read(&args.old)?.changes(&read(&args.new)?);
    if args.json {
        print_json(&changes)?;
        return Ok(ExitCode::SUCCESS);
    }
    if changes.is_empty() {
        output::line("no change")?;
    }
    paths("input", &changes.input)?;
    for (name, changes) in &changes.bindings {
        paths(&format!("${name}"), changes)?;
    }
    if let Some(change) = &changes.every_binding {
        output::line(&format!("everyBinding: {}", sides(change, bool::to_string)))?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Prints the changes of one value: a line for each path added, removed and changed.
fn paths(value: &str, changes: &PathChanges) -> Result<()> {
    if changes.is_empty() {
        return Ok(());
    }
    output::line(value)?;
    for (path, entry) in &changes.added {
        output::line(&format!("  added    {path}: {}", said(entry)))?;
    }
    for (path, entry) in &changes.removed {
        output::line(&format!("  removed  {path}: {}", said(entry)))?;
    }
    for (path, change) in &changes.changed {
        output::line(&format!("  changed  {path}: {}", members(change)))?;
    }
    Ok(())
}

/// What a schema says of a path, on one line.
fn said(entry: &Use) -> String {
    let mut parts = Vec::new();
    if entry.unknown {
        parts.push("unknown".to_owned());
    }
    if !entry.types.is_empty() {
        parts.push(types(&entry.types));
    }
    if let Some(values) = &entry.enumeration {
        parts.push(format!("enum {}", list(values)));
    }
    if let Some(domain) = &entry.domain {
        parts.push(format!("domain {domain}"));
    }
    if let Some(format) = &entry.format {
        parts.push(format!("format {format}"));
    }
    match entry.required {
        Some(true) => parts.push("required".to_owned()),
        Some(false) => parts.push("optional".to_owned()),
        None => {}
    }
    parts.join(", ")
}

/// The members of a path that changed, on one line.
fn members(change: &UseChanges) -> String {
    let flag = |name: &str, change: &Option<Change<bool>>| {
        change
            .as_ref()
            .map(|change| format!("{name} {}", sides(change, bool::to_string)))
    };
    let text = |name: &str, change: &Option<Change<String>>| {
        change
            .as_ref()
            .map(|change| format!("{name} {}", sides(change, String::clone)))
    };
    [
        flag("unknown", &change.unknown),
        flag("whole", &change.whole),
        change
            .types
            .as_ref()
            .map(|change| format!("types {}", sides(change, |t| types(t)))),
        change.enumeration.as_ref().map(enumeration),
        flag("closed", &change.closed),
        text("domain", &change.domain),
        text("format", &change.format),
        flag("required", &change.required),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("; ")
}

/// `old -> new`, a side that said nothing as `(none)`.
fn sides<T>(change: &Change<T>, show: impl Fn(&T) -> String) -> String {
    let side = |side: &Option<T>| side.as_ref().map_or_else(|| "(none)".to_owned(), &show);
    format!("{} -> {}", side(&change.old), side(&change.new))
}

fn types(types: &[JsonType]) -> String {
    let names: Vec<&str> = types.iter().map(|t| t.name()).collect();
    names.join(" | ")
}

fn list(values: &[Value]) -> String {
    values
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// An enumeration that changed: the values added and removed, or the whole of it when the
/// schema started or stopped declaring one.
fn enumeration(change: &EnumChange) -> String {
    match (&change.old, &change.new) {
        (Some(_), Some(_)) => {
            let mut parts = Vec::new();
            if !change.added.is_empty() {
                parts.push(format!("added {}", list(&change.added)));
            }
            if !change.removed.is_empty() {
                parts.push(format!("removed {}", list(&change.removed)));
            }
            format!("enum {}", parts.join(", "))
        }
        (old, new) => {
            let side = |side: &Option<Vec<Value>>| {
                side.as_ref().map_or_else(
                    || "(none)".to_owned(),
                    |values| format!("[{}]", list(values)),
                )
            };
            format!("enum {} -> {}", side(old), side(new))
        }
    }
}
