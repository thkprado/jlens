//! The `jlens` command-line interface.
//!
//! Exit codes: `0` success, `1` the expression has errors (syntax or evaluation), `2` usage or
//! I/O problems (unreadable files, invalid JSON input).

mod report;

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use jlens_core::audit::{self, Tracer};
use jlens_core::{Bindings, Diagnostic, Expression, Severity};
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
    /// Check an expression for syntax errors and annotation problems.
    Check(CheckArgs),
    /// Print the tokens or syntax tree of an expression (for debugging expressions and jlens).
    Ast(AstArgs),
}

/// Where the expression comes from.
#[derive(Args)]
struct ExprSource {
    /// The JSONata expression (omit when using --file).
    #[arg(required_unless_present = "file")]
    expr: Option<String>,
    /// Read the expression from a file.
    #[arg(short, long, conflicts_with = "expr")]
    file: Option<PathBuf>,
}

impl ExprSource {
    fn read(&self) -> Result<(String, String)> {
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
struct EvalArgs {
    #[command(flatten)]
    source: ExprSource,
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
    /// Print an explanation of how the result was reached instead of the result.
    #[arg(long, conflicts_with = "audit")]
    explain: bool,
    /// Print the audit tree as JSON instead of the result.
    #[arg(long, value_enum)]
    audit: Option<AuditFormat>,
    /// What the audit records (with --explain or --audit).
    #[arg(long, value_enum, default_value = "decisions")]
    audit_level: Level,
    /// Write every evaluation event to this file as JSON Lines.
    #[arg(long, value_name = "FILE")]
    trace_log: Option<PathBuf>,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum AuditFormat {
    /// The audit tree as JSON.
    Json,
    /// The explanation as text (same as --explain).
    Text,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum Level {
    /// Comparisons, and/or, conditionals, bindings, filters and annotated nodes.
    Decisions,
    /// Every evaluated node, with its input.
    Full,
}

#[derive(Args)]
struct CheckArgs {
    #[command(flatten)]
    source: ExprSource,
    /// Print diagnostics as JSON.
    #[arg(long)]
    json: bool,
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
        Command::Check(args) => check(args),
        Command::Ast(args) => ast(args),
    }
}

fn read_json(path: &PathBuf) -> Result<Value> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn eval(args: EvalArgs) -> Result<ExitCode> {
    let (source, name) = args.source.read()?;
    let expr = match Expression::compile(&source) {
        Ok(expr) => expr,
        Err(err) => {
            let diagnostic = Diagnostic::from(&err);
            eprint!("{}", report::render(&source, &name, &diagnostic));
            return Ok(ExitCode::from(1));
        }
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

    let auditing = args.explain || args.audit.is_some();
    if auditing || args.trace_log.is_some() {
        return eval_traced(&args, &expr, input.as_ref(), &bindings, auditing);
    }

    match expr.evaluate(input.as_ref(), &bindings) {
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

fn check(args: CheckArgs) -> Result<ExitCode> {
    let (source, name) = args.source.read()?;
    let ast = jlens_core::parse_recover(&source);
    if args.json {
        print_json(&ast.diagnostics)?;
    } else {
        for diagnostic in &ast.diagnostics {
            eprint!("{}", report::render(&source, &name, diagnostic));
        }
        let errors = ast
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        let warnings = ast.diagnostics.len() - errors;
        if ast.diagnostics.is_empty() {
            eprintln!("{name}: ok");
        } else {
            eprintln!("{name}: {errors} error(s), {warnings} warning(s)");
        }
    }
    Ok(if ast.has_errors() {
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

/// Forwards every event to two tracers (the audit collector and the event log).
struct Tee<'a>(&'a mut dyn Tracer, &'a mut dyn Tracer);

impl Tracer for Tee<'_> {
    fn begin(&mut self, info: &audit::EvalInfo) {
        self.0.begin(info);
        self.1.begin(info);
    }
    fn enter(&mut self, node: &jlens_core::Node, input: audit::ValueRef<'_>) {
        self.0.enter(node, input);
        self.1.enter(node, input);
    }
    fn exit(&mut self, node: &jlens_core::Node, outcome: audit::Outcome<'_>) {
        self.0.exit(node, outcome);
        self.1.exit(node, outcome);
    }
    fn operands(
        &mut self,
        node: &jlens_core::Node,
        lhs: audit::ValueRef<'_>,
        rhs: audit::ValueRef<'_>,
    ) {
        self.0.operands(node, lhs, rhs);
        self.1.operands(node, lhs, rhs);
    }
    fn short_circuit(&mut self, node: &jlens_core::Node) {
        self.0.short_circuit(node);
        self.1.short_circuit(node);
    }
    fn branch(&mut self, node: &jlens_core::Node, taken: audit::Branch) {
        self.0.branch(node, taken);
        self.1.branch(node, taken);
    }
    fn bind(&mut self, node: &jlens_core::Node, name: &str, value: audit::ValueRef<'_>) {
        self.0.bind(node, name, value);
        self.1.bind(node, name, value);
    }
    fn filter_begin(&mut self, predicate: &jlens_core::Node) {
        self.0.filter_begin(predicate);
        self.1.filter_begin(predicate);
    }
    fn filter_item(
        &mut self,
        predicate: &jlens_core::Node,
        index: usize,
        item: audit::ValueRef<'_>,
        passed: bool,
    ) {
        self.0.filter_item(predicate, index, item, passed);
        self.1.filter_item(predicate, index, item, passed);
    }
    fn filter_end(&mut self, predicate: &jlens_core::Node) {
        self.0.filter_end(predicate);
        self.1.filter_end(predicate);
    }
    fn end(&mut self, outcome: audit::Outcome<'_>) {
        self.0.end(outcome);
        self.1.end(outcome);
    }
}

/// `jlens eval` with --explain, --audit and/or --trace-log.
fn eval_traced(
    args: &EvalArgs,
    expr: &Expression,
    input: Option<&Value>,
    bindings: &Bindings,
    auditing: bool,
) -> Result<ExitCode> {
    let options = audit::AuditOptions {
        level: match args.audit_level {
            Level::Decisions => audit::AuditLevel::Decisions,
            Level::Full => audit::AuditLevel::Full,
        },
        ..Default::default()
    };
    let mut collector = audit::AuditCollector::new(options, input);
    let mut log = match &args.trace_log {
        Some(path) => Some(audit::JsonlTracer::new(
            std::io::BufWriter::new(
                fs::File::create(path).with_context(|| format!("creating {}", path.display()))?,
            ),
            options.limits,
        )),
        None => None,
    };
    let opts = jlens_core::Options::default();
    let result = match (&mut log, auditing) {
        (Some(log), true) => {
            expr.evaluate_traced(input, bindings, &opts, &mut Tee(&mut collector, log))
        }
        (Some(log), false) => expr.evaluate_traced(input, bindings, &opts, log),
        (None, _) => expr.evaluate_traced(input, bindings, &opts, &mut collector),
    };
    if let Some(log) = log {
        if let Some(e) = log.error() {
            anyhow::bail!("writing the trace log: {e}");
        }
        use std::io::Write as _;
        log.into_inner().flush().context("writing the trace log")?;
    }

    if auditing {
        let tree = collector.finish();
        if matches!(args.audit, Some(AuditFormat::Json)) {
            print_json(&tree)?;
        } else {
            print!("{}", audit::explain(&tree));
        }
        return Ok(if result.is_ok() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        });
    }
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
