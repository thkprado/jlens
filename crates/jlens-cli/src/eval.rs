//! `jlens eval`.

use std::fs;
use std::io::{BufRead, BufReader, BufWriter, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;
use jlens_core::audit::{self, Tracer};
use jlens_core::reads;
use jlens_core::{Bindings, Diagnostic, Document, Expression, Options};
use serde_json::Value;

use crate::output::{self, Format, truthy};
use crate::{ExprSource, print_json, read_json, report};

#[derive(Args)]
pub struct EvalArgs {
    #[command(flatten)]
    source: ExprSource,
    /// Input JSON document (defaults to stdin, which must not be a terminal).
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
    /// Print one JSON envelope per evaluation on stdout, errors included:
    /// {"ok":true,"result":…}, {"ok":true,"undefined":true} or {"ok":false,"error":{…}}.
    /// With --ndjson, each envelope also carries the input "line".
    #[arg(long, conflicts_with_all = ["raw_output", "explain", "audit"])]
    json: bool,
    /// Stop an evaluation after this many milliseconds (error D1012).
    #[arg(long, value_name = "MS")]
    timeout: Option<u64>,
    /// Stop an evaluation nested deeper than this (error D1011).
    #[arg(long, value_name = "N")]
    max_depth: Option<usize>,
    #[command(flatten)]
    caps: SizeCaps,
    /// The time seen by $now() and $millis(): milliseconds since the epoch, or an ISO 8601
    /// timestamp such as 2026-01-01T00:00:00Z.
    #[arg(long, value_name = "TIME", value_parser = parse_now)]
    now: Option<i64>,
    /// Seed for $random() and $shuffle(): the same seed gives the same results.
    #[arg(long, value_name = "N")]
    seed: Option<u64>,
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

/// The size caps of an evaluation, taken by `eval` and `test`.
#[derive(Args)]
pub struct SizeCaps {
    /// Stop an evaluation that builds a string longer than this many bytes (error U1001);
    /// the default is 256 MiB.
    #[arg(long, value_name = "BYTES")]
    max_string_length: Option<usize>,
    /// Stop an evaluation that builds an array of more items than this (error U1001); the
    /// default is 16,777,216.
    #[arg(long, value_name = "N")]
    max_array_length: Option<usize>,
    /// Stop an evaluation whose result is larger than this many bytes of compact JSON, counting
    /// a value each time it appears (error U1001); the default is 256 MiB.
    #[arg(long, value_name = "BYTES")]
    max_result_size: Option<usize>,
}

impl SizeCaps {
    /// Sets the caps that were given in `options`.
    pub fn apply(&self, options: &mut Options) {
        options.max_string_length = self.max_string_length;
        options.max_array_length = self.max_array_length;
        options.max_result_size = self.max_result_size;
    }
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
    guard_terminal(
        args.input.is_none() && !args.no_input,
        std::io::stdin().is_terminal(),
    )?;
    let (source, name) = args.source.read()?;
    let expr = match Expression::compile(&source) {
        Ok(expr) => expr,
        Err(err) => {
            if args.json {
                output::line(&output::error_envelope(&err, &source, None))?;
            } else {
                eprint!(
                    "{}",
                    report::render(&source, &name, &Diagnostic::from(&err))
                );
            }
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
    args.caps.apply(&mut options);
    options.now = args.now;
    options.seed = args.seed;
    let format = Format {
        compact: args.compact || args.ndjson,
        raw: args.raw_output,
    };

    if args.ndjson {
        if args.no_input {
            bail!("--ndjson needs input documents");
        }
        return ndjson(&args, &source, &expr, &bindings, &options, format);
    }

    let input = if args.no_input {
        None
    } else {
        Some(match &args.input {
            Some(path) => (
                fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
                path.display().to_string(),
            ),
            None => {
                let mut text = String::new();
                std::io::stdin()
                    .read_to_string(&mut text)
                    .context("reading stdin")?;
                (text, "stdin".to_owned())
            }
        })
    };
    let invalid = |e: serde_json::Error, name: &str| -> Result<ExitCode> {
        // Still exit code 2, but the envelope reports it too (ADR 0008).
        if args.json {
            output::line(&output::error_envelope(&invalid_input(&e), &source, None))?;
        }
        Err(anyhow::Error::new(e).context(format!("parsing {name}")))
    };

    let auditing = args.explain || args.audit.is_some();
    let result = if auditing || args.trace_log.is_some() {
        // The audit and the trace log record input values: parse the whole document.
        let doc = match &input {
            Some((text, name)) => match serde_json::from_str(text) {
                Ok(doc) => Some(doc),
                Err(e) => return invalid(e, name),
            },
            None => None,
        };
        match traced(&args, &expr, doc.as_ref(), &bindings, &options, auditing)? {
            Traced::Printed(code) => return Ok(code),
            Traced::Result(r) => r,
        }
    } else {
        match &input {
            Some((text, name)) => match Document::parse_cut(text, &input_read(&expr)) {
                Ok(doc) => expr.evaluate_document(&doc, &bindings, &options),
                Err(e) => return invalid(e, name),
            },
            None => expr.evaluate_with(None, &bindings, &options),
        }
    };
    match result {
        Ok(value) => {
            if args.filter {
                if truthy(value.as_ref())
                    && let Some((text, name)) = &input
                {
                    // The document was parsed without the fields the expression does not read.
                    let doc: Value = match serde_json::from_str(text) {
                        Ok(doc) => doc,
                        Err(e) => return invalid(e, name),
                    };
                    output::line(&if args.json {
                        output::ok_envelope(Some(&doc), None)
                    } else {
                        format.render(&doc)
                    })?;
                }
            } else if args.json {
                output::line(&output::ok_envelope(value.as_ref(), None))?;
            } else if let Some(value) = value {
                output::line(&format.render(&value))?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(err) => {
            if args.json {
                output::line(&output::error_envelope(&err, &source, None))?;
            } else {
                eprintln!("{err}");
            }
            Ok(ExitCode::from(1))
        }
    }
}

/// Evaluates the expression for each line of the input.
fn ndjson(
    args: &EvalArgs,
    source: &str,
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
    let invalid = |out: &mut BufWriter<_>, e: serde_json::Error, number| -> Result<()> {
        if args.json {
            let err = invalid_input(&e);
            writeln!(
                out,
                "{}",
                output::error_envelope(&err, source, Some(number))
            )?;
        } else {
            eprintln!("line {number}: invalid JSON: {e}");
        }
        Ok(())
    };
    let read = input_read(expr);
    for (index, line) in reader.lines().enumerate() {
        let number = index + 1;
        let line = line.context("reading input")?;
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let doc = match Document::parse_cut(text, &read) {
            Ok(doc) => doc,
            Err(e) => {
                failures += 1;
                invalid(&mut out, e, number)?;
                continue;
            }
        };
        match expr.evaluate_document(&doc, bindings, options) {
            Ok(value) if args.filter => {
                if truthy(value.as_ref()) {
                    // The line was parsed without the fields the expression does not read.
                    let doc: Value = match serde_json::from_str(text) {
                        Ok(doc) => doc,
                        Err(e) => {
                            failures += 1;
                            invalid(&mut out, e, number)?;
                            continue;
                        }
                    };
                    if args.json {
                        writeln!(out, "{}", output::ok_envelope(Some(&doc), Some(number)))?;
                    } else {
                        writeln!(out, "{}", format.render(&doc))?;
                    }
                }
            }
            Ok(value) if args.json => {
                writeln!(out, "{}", output::ok_envelope(value.as_ref(), Some(number)))?;
            }
            Ok(Some(value)) => writeln!(out, "{}", format.render(&value))?,
            Ok(None) => {}
            Err(err) => {
                failures += 1;
                if args.json {
                    writeln!(
                        out,
                        "{}",
                        output::error_envelope(&err, source, Some(number))
                    )?;
                } else {
                    eprintln!("line {number}: {err}");
                }
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

/// Fails instead of waiting when the input would be read from a terminal (ADR 0008).
fn guard_terminal(reads_stdin: bool, stdin_is_terminal: bool) -> Result<()> {
    if reads_stdin && stdin_is_terminal {
        bail!("standard input is a terminal: pass the input with -i FILE, or use --no-input");
    }
    Ok(())
}

/// What the expression reads of its input (ADR 0016): the input is parsed without the other
/// fields. An expression that does not read its input needs none of its fields.
fn input_read(expr: &Expression) -> reads::Read {
    expr.reads()
        .input()
        .cloned()
        .unwrap_or_else(|| reads::Read::Fields(Default::default()))
}

/// The error (`U2001`) of an input document that is not valid JSON.
fn invalid_input(err: &serde_json::Error) -> jlens_core::Error {
    jlens_core::Error::new("U2001").with("message", err.to_string())
}

/// Parses `--now`: milliseconds since the epoch, or an ISO 8601 timestamp.
pub fn parse_now(text: &str) -> std::result::Result<i64, String> {
    if let Ok(millis) = text.parse() {
        return Ok(millis);
    }
    let expected = || "expected milliseconds or an ISO 8601 timestamp".to_owned();
    let expr = Expression::compile("$toMillis($t)").expect("a valid expression");
    match expr.evaluate(None, &Bindings::new().var("t", text)) {
        Ok(Some(Value::Number(n))) => n.as_f64().map(|f| f as i64).ok_or_else(expected),
        _ => Err(expected()),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_waits_on_a_terminal() {
        assert!(guard_terminal(true, true).is_err());
        assert!(guard_terminal(true, false).is_ok());
        assert!(guard_terminal(false, true).is_ok());
    }

    #[test]
    fn now_takes_millis_or_a_timestamp() {
        assert_eq!(parse_now("1700000000000"), Ok(1_700_000_000_000));
        assert_eq!(parse_now("-1"), Ok(-1));
        assert_eq!(parse_now("1970-01-01T00:00:01Z"), Ok(1000));
        assert_eq!(parse_now("2026-01-01T00:00:00.000Z"), Ok(1_767_225_600_000));
        assert!(parse_now("yesterday").is_err());
        assert!(parse_now("").is_err());
    }
}
