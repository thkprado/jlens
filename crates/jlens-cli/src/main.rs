//! The `jlens` command-line interface.
//!
//! Exit codes: `0` success, `1` the expression has errors (syntax or evaluation), `2` usage or
//! I/O problems (unreadable files, invalid JSON input).

mod eval;
mod functions;
mod output;
mod report;
mod schema;

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use jlens_core::{Diagnostic, Severity};
use serde_json::Value;

/// Evaluate, check and explain JSONata expressions.
#[derive(Parser)]
#[command(
    name = "jlens",
    version,
    about,
    after_help = "Run `jlens <command> --help` for the options of each command."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Evaluate an expression against JSON input (a document, or one per line with --ndjson).
    Eval(eval::EvalArgs),
    /// Check an expression for syntax errors, annotation problems and unknown fields.
    Check(CheckArgs),
    /// List the built-in functions, or describe one.
    Functions(functions::FunctionsArgs),
    /// Infer schemas from sample documents and query the language service.
    #[command(subcommand)]
    Schema(schema::SchemaCommand),
    /// Print the tokens or syntax tree of an expression (for debugging expressions and jlens).
    Ast(AstArgs),
    /// Run the language server (LSP) over standard input and output, for editors. It is
    /// configured by the `jlens.json` of each workspace folder.
    Lsp,
}

/// Where the expression comes from.
#[derive(Args)]
pub struct ExprSource {
    /// The JSONata expression (omit when using --file).
    #[arg(required_unless_present = "file")]
    expr: Option<String>,
    /// Read the expression from a file.
    #[arg(short, long, conflicts_with = "expr")]
    file: Option<PathBuf>,
}

impl ExprSource {
    /// The expression text and a name for messages (`<expr>` or the file path).
    pub fn read(&self) -> Result<(String, String)> {
        match (&self.expr, &self.file) {
            (Some(expr), _) => Ok((expr.clone(), "<expr>".to_owned())),
            (None, Some(file)) => Ok((
                fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?,
                file.display().to_string(),
            )),
            (None, None) => unreachable!("clap requires an expression or --file"),
        }
    }
}

#[derive(Args)]
struct CheckArgs {
    #[command(flatten)]
    source: ExprSource,
    /// Print diagnostics as JSON.
    #[arg(long)]
    json: bool,
    /// Also report fields that this schema (JSON Schema, e.g. from `jlens schema infer`) lacks.
    #[arg(long, value_name = "FILE")]
    schema: Option<PathBuf>,
}

#[derive(Args)]
struct AstArgs {
    #[command(flatten)]
    source: ExprSource,
    /// Print the token stream instead of the tree.
    #[arg(long, conflicts_with = "raw")]
    tokens: bool,
    /// Print the raw parse tree, before post-processing.
    #[arg(long)]
    raw: bool,
    /// Keep going after syntax errors and print the partial tree.
    #[arg(long)]
    recover: bool,
}

/// Stack size of the thread doing the work. The main thread's stack is small on some platforms
/// (1 MiB on Windows), and walking a deeply nested syntax tree (to print it, for example) is
/// recursive.
const STACK_SIZE: usize = 64 * 1024 * 1024;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let worker = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || run(cli))
        .expect("spawning the worker thread");
    let result = worker
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    match result {
        Ok(code) => code,
        Err(err) if output::is_broken_pipe(&err) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::Eval(args) => eval::run(args),
        Command::Check(args) => check(args),
        Command::Functions(args) => functions::run(args),
        Command::Schema(cmd) => schema::run(cmd),
        Command::Ast(args) => ast(args),
        Command::Lsp => {
            jlens_lsp::run_stdio().map_err(|e| anyhow::anyhow!("language server: {e}"))?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Reads and parses a JSON file.
pub fn read_json(path: &PathBuf) -> Result<Value> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Prints a value as pretty JSON on stdout.
pub fn print_json(value: &impl serde::Serialize) -> Result<()> {
    output::line(&serde_json::to_string_pretty(value)?)
}

fn check(args: CheckArgs) -> Result<ExitCode> {
    let (source, name) = args.source.read()?;
    let schema = args.schema.as_ref().map(schema::load).transpose()?;
    let diagnostics = jlens_schema::LanguageService::new(schema).diagnostics(&source);
    let errors = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    if args.json {
        print_json(&diagnostics)?;
    } else {
        for diagnostic in &diagnostics {
            eprint!("{}", report::render(&source, &name, diagnostic));
        }
        let warnings = diagnostics.len() - errors;
        if diagnostics.is_empty() {
            eprintln!("{name}: ok");
        } else {
            eprintln!("{name}: {errors} error(s), {warnings} warning(s)");
        }
    }
    Ok(if errors > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

fn ast(args: AstArgs) -> Result<ExitCode> {
    let (source, name) = args.source.read()?;
    let failed = |err: &jlens_core::Error| {
        eprint!("{}", report::render(&source, &name, &Diagnostic::from(err)));
        Ok(ExitCode::from(1))
    };
    if args.tokens {
        return match jlens_core::debug::tokens(&source) {
            Ok(tokens) => print_json(&tokens).map(|_| ExitCode::SUCCESS),
            Err(err) => failed(&err),
        };
    }
    if args.raw {
        return match jlens_core::debug::raw_tree(&source, args.recover) {
            Ok((tree, errors)) => {
                print_json(&tree)?;
                for err in &errors {
                    eprint!("{}", report::render(&source, &name, &Diagnostic::from(err)));
                }
                Ok(if errors.is_empty() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                })
            }
            Err(err) => failed(&err),
        };
    }
    let ast = if args.recover {
        jlens_core::parse_recover(&source)
    } else {
        match jlens_core::parse(&source) {
            Ok(ast) => ast,
            Err(err) => return failed(&err),
        }
    };
    print_json(&ast)?;
    for diagnostic in &ast.diagnostics {
        eprint!("{}", report::render(&source, &name, diagnostic));
    }
    Ok(if ast.has_errors() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}
