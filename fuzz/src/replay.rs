//! Runs saved inputs through the body of a fuzz target, without libFuzzer, on any platform:
//! `cargo xtask fuzz-replay <target> <file or directory>...`.
//!
//! Each input runs on the target's 1 MB thread. A panic is reported and the next input runs; a
//! stack overflow aborts the program, so the path of each input is printed before it runs.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use jlens_fuzz::targets::TARGETS;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let names = TARGETS.iter().map(|(name, _)| *name).collect::<Vec<_>>();
    let Some((target, paths)) = args.split_first().filter(|(_, paths)| !paths.is_empty()) else {
        eprintln!(
            "usage: replay <target> <file or directory>...\ntargets: {}",
            names.join(", ")
        );
        return ExitCode::FAILURE;
    };
    let Some(&(_, run)) = TARGETS.iter().find(|(name, _)| name == target) else {
        eprintln!("unknown target {target}; targets: {}", names.join(", "));
        return ExitCode::FAILURE;
    };
    let mut files = Vec::new();
    for path in paths {
        if let Err(e) = collect(Path::new(path), &mut files) {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    }
    let mut failed = Vec::new();
    for file in &files {
        let data = match std::fs::read(file) {
            Ok(data) => data,
            Err(e) => {
                eprintln!("{}: {e}", file.display());
                return ExitCode::FAILURE;
            }
        };
        eprintln!("{}", file.display());
        if std::panic::catch_unwind(|| run(&data)).is_err() {
            failed.push(file);
        }
    }
    eprintln!("{target}: {} inputs, {} failed", files.len(), failed.len());
    for file in &failed {
        eprintln!("failed: {}", file.display());
    }
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Adds `path`, or the files of the directory `path` in order, to `files`.
fn collect(path: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !path.is_dir() {
        files.push(path.to_path_buf());
        return Ok(());
    }
    let mut entries = std::fs::read_dir(path)?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort();
    files.extend(entries.into_iter().filter(|p| p.is_file()));
    Ok(())
}
