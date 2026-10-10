//! `jlens eval`.

use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;
use jlens_core::audit::{self, Tracer};
use jlens_core::{Bindings, Diagnostic, Expression, Options};
use serde_json::Value;

use crate::output::{self, Format, truthy};
use crate::{ExprSource, print_json, read_json, report};

#[derive(Args)]
pub struct EvalArgs {
    #[command(flatten)]
    source: ExprSource,
    /// Input JSON document (defaults to stdin).
    #[arg(short, long, value_name = "FILE")]
    input: Option<PathBuf>,
    /// Evaluate with no input document.
    #[arg(long, conflicts_with = "input")]
    no_input: bool,
    /// The input holds one JSON document per line; evaluate the expression for each and print
    /// one result per line.
    #[arg(long)]
    ndjson: bool,
    /// Print the input documents for which the expression is truthy, instead of the results.
    #[arg(long)]
    filter: bool,
    /// JSON object whose fields are bound as variables (`$name`).
    #[arg(short, long, value_name = "FILE")]
    bindings: Option<PathBuf>,
    /// Print each result on a single line.
    #[arg(short, long)]
    compact: bool,
    /// Print strings without quotes.
    #[arg(short, long)]
    raw_output: bool,
    /// Stop an evaluation after this many milliseconds (error D1012).
    #[arg(long, value_name = "MS")]
    timeout: Option<u64>,
    /// Stop an evaluation nested deeper than this (error D1011).
    #[arg(long, value_name = "N")]
    max_depth: Option<usize>,
    /// Print an explanation of how the result was reached instead of the result.
    #[arg(long, conflicts_with_all = ["audit", "ndjson", "filter"])]
    explain: bool,
    /// Print the audit tree as JSON (`json`) or the explanation (`text`) instead of the result.
    #[arg(long, value_enum, conflicts_with_all = ["ndjson", "filter"])]
    audit: Option<AuditFormat>,
    /// What the audit records (with --explain or --audit).
    #[arg(long, value_enum, default_value = "decisions")]
    audit_level: Level,
    /// Write every evaluation event to this file as JSON Lines.
    #[arg(long, value_name = "FILE", conflicts_with = "ndjson")]
    trace_log: Option<PathBuf>,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum AuditFormat {
    /// The audit tree as JSON.
    Json,
    /// The explanation as text (same as --explain).
    Text,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Level {
    /// Comparisons, and/or, conditionals, bindings, filters and annotated nodes.
    Decisions,
    /// Every evaluated node, with its input.
    Full,
}

pub fn run(args: EvalArgs) -> Result<ExitCode> {
    let (source, name) = args.source.read()?;
    let expr = match Expression::compile(&source) {
        Ok(expr) => expr,
        Err(err) => {
            eprint!(
                "{}",
                report::render(&source, &name, &Diagnostic::from(&err))
            );
            return Ok(ExitCode::from(1));
        }
    };
    let bindings = match &args.bindings {
        Some(path) => match read_json(path)? {
            Value::Object(map) => map.into_iter().collect(),
            _ => bail!("{} must contain a JSON object", path.display()),
        },
        None => Bindings::new(),
    };
    let mut options = Options::new();
    options.timeout = args.timeout.map(Duration::from_millis);
    options.max_depth = args.max_depth;
    let format = Format {
        compact: args.compact || args.ndjson,
        raw: args.raw_output,
    };

    if args.ndjson {
        if args.no_input {
            bail!("--ndjson needs input documents");
        }
        return ndjson(&args, &expr, &bindings, &options, format);
    }

    let input = if args.no_input {
        None
    } else {
        Some(match &args.input {
            Some(path) => read_json(path)?,
            None => {
                let mut text = String::new();
                std::io::stdin()
                    .read_to_string(&mut text)
                    .context("reading stdin")?;
                serde_json::from_str(&text).context("parsing stdin")?
            }
        })
    };

    let auditing = args.explain || args.audit.is_some();
    let result = if auditing || args.trace_log.is_some() {
        match traced(&args, &expr, input.as_ref(), &bindings, &options, auditing)? {
            Traced::Printed(code) => return Ok(code),
            Traced::Result(r) => r,
        }
    } else {
        expr.evaluate_with(input.as_ref(), &bindings, &options)
    };
    match result {
        Ok(value) => {
            if args.filter {
                if truthy(value.as_ref())
                    && let Some(doc) = &input
                {
                    output::line(&format.render(doc))?;
                }
            } else if let Some(value) = value {
                output::line(&format.render(&value))?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(err) => {
            eprintln!("{err}");
            Ok(ExitCode::from(1))
        }
    }
}

/// Evaluates the expression for each line of the input.
fn ndjson(
    args: &EvalArgs,
    expr: &Expression,
    bindings: &Bindings,
    options: &Options,
    format: Format,
) -> Result<ExitCode> {
    let reader: Box<dyn BufRead> = match &args.input {
        Some(path) => Box::new(BufReader::new(
            fs::File::open(path).with_context(|| format!("reading {}", path.display()))?,
        )),
        None => Box::new(std::io::stdin().lock()),
    };
    let mut out = BufWriter::new(std::io::stdout().lock());
    let mut failures = 0usize;
    for (index, line) in reader.lines().enumerate() {
        let number = index + 1;
        let line = line.context("reading input")?;
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let doc: Value = match serde_json::from_str(text) {
            Ok(doc) => doc,
            Err(e) => {
                eprintln!("line {number}: invalid JSON: {e}");
                failures += 1;
                continue;
            }
        };
        match expr.evaluate_with(Some(&doc), bindings, options) {
            Ok(value) if args.filter => {
                if truthy(value.as_ref()) {
                    writeln!(out, "{}", format.render(&doc))?;
                }
            }
            Ok(Some(value)) => writeln!(out, "{}", format.render(&value))?,
            Ok(None) => {}
            Err(err) => {
                eprintln!("line {number}: {err}");
                failures += 1;
            }
        }
    }
    out.flush()?;
    Ok(if failures > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

enum Traced {
    /// The audit or explanation was printed; this is the exit code.
    Printed(ExitCode),
    /// Only a trace log was written; the result is printed as usual.
    Result(jlens_core::Result<Option<Value>>),
}

/// Evaluation with --explain, --audit and/or --trace-log.
fn traced(
    args: &EvalArgs,
    expr: &Expression,
    input: Option<&Value>,
    bindings: &Bindings,
    options: &Options,
    auditing: bool,
) -> Result<Traced> {
    let audit_options = audit::AuditOptions {
        level: match args.audit_level {
            Level::Decisions => audit::AuditLevel::Decisions,
            Level::Full => audit::AuditLevel::Full,
        },
        ..Default::default()
    };
    let mut collector = audit::AuditCollector::new(audit_options, input);
    let mut log = match &args.trace_log {
        Some(path) => Some(audit::JsonlTracer::new(
            BufWriter::new(
                fs::File::create(path).with_context(|| format!("creating {}", path.display()))?,
            ),
            audit_options.limits,
        )),
        None => None,
    };
    let result = match (&mut log, auditing) {
        (Some(log), true) => {
            expr.evaluate_traced(input, bindings, options, &mut Tee(&mut collector, log))
        }
        (Some(log), false) => expr.evaluate_traced(input, bindings, options, log),
        (None, _) => expr.evaluate_traced(input, bindings, options, &mut collector),
    };
    if let Some(log) = log {
        if let Some(e) = log.error() {
            bail!("writing the trace log: {e}");
        }
        log.into_inner().flush().context("writing the trace log")?;
    }
    if !auditing {
        return Ok(Traced::Result(result));
    }
    let tree = collector.finish();
    if matches!(args.audit, Some(AuditFormat::Json)) {
        print_json(&tree)?;
    } else {
        let text = audit::explain(&tree).to_string();
        output::line(text.trim_end())?;
    }
    Ok(Traced::Printed(if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }))
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
