//! The `jlens` command-line interface.
//!
//! Exit codes: `0` success, `1` the expression has errors (syntax or evaluation), `2` usage or
//! I/O problems (unreadable files, invalid JSON input).

mod errors;
mod eval;
mod functions;
mod output;
mod report;
mod schema;
mod test;

use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use jlens_core::{Diagnostic, Severity};
use jlens_lsp::config;
use jlens_schema::LanguageService;
use serde_json::Value;

// Evaluation allocates for most of its work. The default allocators of musl (the Linux release
// binaries) and Windows make it up to 11 and 2 times slower; see the performance chapter.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

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
    /// Run files of test cases: inputs, bindings and the expected result or error code.
    Test(test::TestArgs),
    /// List the built-in functions, or describe one.
    Functions(functions::FunctionsArgs),
    /// List the error and warning codes, or describe one.
    Errors(errors::ErrorsArgs),
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
    /// The JSONata expression (omit when using --file).
    #[arg(required_unless_present = "files")]
    expr: Option<String>,
    /// Read expressions from files: several, or glob patterns such as `rules/*.jsonata`.
    #[arg(short, long = "file", value_name = "FILE", num_args = 1.., conflicts_with = "expr")]
    files: Vec<String>,
    /// Print the diagnostics of all expressions as one compact JSON array.
    #[arg(long)]
    json: bool,
    /// Fail with exit code 1 on warnings too.
    #[arg(long)]
    deny_warnings: bool,
    /// Also report fields that this schema (JSON Schema, e.g. from `jlens schema infer`) lacks.
    /// It replaces the schema that the configuration gives a file.
    #[arg(long, value_name = "FILE")]
    schema: Option<PathBuf>,
    /// The configuration that gives files a schema, variables and host functions, as for the
    /// language server [default: the `jlens.json` of the current directory, if there is one].
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Give the expression of the argument the configuration of this file, as if it were its
    /// contents. The file need not exist.
    #[arg(long, value_name = "FILE", conflicts_with = "files")]
    path: Option<PathBuf>,
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
        Command::Test(args) => test::run(args),
        Command::Functions(args) => functions::run(args),
        Command::Errors(args) => errors::run(args),
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
    // Read every expression first, so that an unreadable file prints nothing on stdout.
    let sources = match &args.expr {
        Some(expr) => vec![(expr.clone(), None)],
        None => schema::expand(&args.files)?
            .into_iter()
            .map(|file| {
                let source = fs::read_to_string(&file)
                    .with_context(|| format!("reading {}", file.display()))?;
                Ok((source, Some(file)))
            })
            .collect::<Result<Vec<_>>>()?,
    };
    let schema = args.schema.as_ref().map(schema::load).transpose()?;
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let entries = match &args.config {
        Some(path) => {
            let root = absolute(&cwd, path);
            let root = root.parent().unwrap_or(&cwd);
            config::parse(&read_json(path)?, root)
                .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?
        }
        None => config::load(&cwd).map_err(anyhow::Error::msg)?,
    };
    // The entry of each file (none for an inline expression), and one service per entry.
    let keys: Vec<Option<usize>> = sources
        .iter()
        .map(|(_, file)| {
            let path = absolute(&cwd, file.as_deref().or(args.path.as_deref())?);
            config::find(&entries, &path).map(|(i, _)| i)
        })
        .collect();
    let mut services: HashMap<Option<usize>, LanguageService> = HashMap::new();
    for &key in &keys {
        if services.contains_key(&key) {
            continue;
        }
        let (service, problems) = match (key, &schema) {
            (Some(i), Some(schema)) => entries[i]
                .settings
                .declare(LanguageService::new(Some(schema.clone()))),
            (Some(i), None) => entries[i].service(),
            (None, schema) => (LanguageService::new(schema.clone()), Vec::new()),
        };
        if !problems.is_empty() {
            let name = args
                .config
                .as_deref()
                .unwrap_or(Path::new(config::FILE_NAME));
            anyhow::bail!("{}: {}", name.display(), problems.join("; "));
        }
        services.insert(key, service);
    }
    let (mut errors, mut warnings) = (0, 0);
    let mut json = Vec::new();
    for ((source, file), key) in sources.iter().zip(&keys) {
        let file = file.as_ref().map(|f| f.display().to_string());
        let diagnostics = services[key].diagnostics(source);
        let file_errors = diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        let file_warnings = diagnostics.len() - file_errors;
        errors += file_errors;
        warnings += file_warnings;
        let name = file.as_deref().unwrap_or("<expr>");
        if args.json {
            for diagnostic in &diagnostics {
                json.push(diagnostic_json(source, file.as_deref(), diagnostic)?);
            }
            continue;
        }
        for diagnostic in &diagnostics {
            eprint!("{}", report::render(source, name, diagnostic));
        }
        if diagnostics.is_empty() {
            eprintln!("{name}: ok");
        } else {
            eprintln!("{name}: {file_errors} error(s), {file_warnings} warning(s)");
        }
    }
    if args.json {
        output::line(&serde_json::to_string(&json)?)?;
    } else if sources.len() > 1 {
        eprintln!(
            "{} file(s): {errors} error(s), {warnings} warning(s)",
            sources.len()
        );
    }
    Ok(if errors > 0 || (args.deny_warnings && warnings > 0) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// `path` made absolute against `cwd`, without `.` and `..` components, so that the paths of
/// files and of the configuration compare.
fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    let mut absolute = PathBuf::new();
    for component in cwd.join(path).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                absolute.pop();
            }
            component => absolute.push(component),
        }
    }
    absolute
}

/// A diagnostic as `check --json` prints it: `file` (absent for an inline expression), `line`
/// and `column` (1-based, the column in characters), then the diagnostic's own fields.
fn diagnostic_json(source: &str, file: Option<&str>, diagnostic: &Diagnostic) -> Result<Value> {
    let mut object = serde_json::Map::new();
    if let Some(file) = file {
        object.insert("file".into(), file.into());
    }
    let (line, column) = report::location(source, diagnostic.span.start);
    object.insert("line".into(), line.into());
    object.insert("column".into(), column.into());
    if let Value::Object(fields) = serde_json::to_value(diagnostic)? {
        object.extend(fields);
    }
    Ok(Value::Object(object))
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
