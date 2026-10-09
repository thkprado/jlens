//! The `jlens` command-line interface.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use jlens_core::{Bindings, Expression};
use serde_json::Value;

/// Evaluate, check and explain JSONata expressions.
#[derive(Parser)]
#[command(name = "jlens", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Evaluate an expression against a JSON input.
    Eval(EvalArgs),
}

#[derive(Args)]
struct EvalArgs {
    /// The JSONata expression (omit when using --file).
    #[arg(required_unless_present = "file")]
    expr: Option<String>,
    /// Read the expression from a file.
    #[arg(short, long, conflicts_with = "expr")]
    file: Option<PathBuf>,
    /// Input JSON document (defaults to stdin; use --no-input for undefined input).
    #[arg(short, long)]
    input: Option<PathBuf>,
    /// Evaluate with no input document.
    #[arg(long, conflicts_with = "input")]
    no_input: bool,
    /// JSON object whose entries are bound as variables.
    #[arg(short, long)]
    bindings: Option<PathBuf>,
    /// Print the result on a single line.
    #[arg(long)]
    compact: bool,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::Eval(args) => eval(args),
    }
}

fn read_json(path: &PathBuf) -> Result<Value> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

fn eval(args: EvalArgs) -> Result<ExitCode> {
    let source = match (&args.expr, &args.file) {
        (Some(expr), _) => expr.clone(),
        (None, Some(file)) => {
            fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?
        }
        (None, None) => unreachable!("clap requires an expression or --file"),
    };
    let input = if args.no_input {
        None
    } else if let Some(path) = &args.input {
        Some(read_json(path)?)
    } else {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("reading stdin")?;
        Some(serde_json::from_str(&text).context("parsing stdin")?)
    };
    let bindings = match &args.bindings {
        Some(path) => match read_json(path)? {
            Value::Object(map) => map.into_iter().collect(),
            _ => anyhow::bail!("{} must contain a JSON object", path.display()),
        },
        None => Bindings::new(),
    };

    let result =
        Expression::compile(&source).and_then(|expr| expr.evaluate(input.as_ref(), &bindings));
    match result {
        Ok(Some(value)) => {
            let text = if args.compact {
                serde_json::to_string(&value)?
            } else {
                serde_json::to_string_pretty(&value)?
            };
            println!("{text}");
            Ok(ExitCode::SUCCESS)
        }
        Ok(None) => Ok(ExitCode::SUCCESS),
        Err(err) => {
            eprintln!("{err}");
            Ok(ExitCode::from(1))
        }
    }
}
