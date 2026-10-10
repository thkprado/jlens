//! `jlens errors`: the error catalog.

use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Args;
use jlens_core::error::{CatalogEntry, Category, catalog};
use serde::Serialize;

use crate::{output, print_json};

#[derive(Args)]
pub struct ErrorsArgs {
    /// Describe one code, such as `S0202` (case does not matter).
    code: Option<String>,
    /// Print as JSON.
    #[arg(long)]
    json: bool,
}

/// One catalog entry as `--json` prints it.
#[derive(Serialize)]
struct Entry {
    code: &'static str,
    category: &'static str,
    template: &'static str,
}

impl From<&CatalogEntry> for Entry {
    fn from(e: &CatalogEntry) -> Self {
        Entry {
            code: e.code,
            category: category_name(e.category()),
            template: e.template,
        }
    }
}

fn category_name(c: Category) -> &'static str {
    match c {
        Category::Static => "static",
        Category::Type => "type",
        Category::Dynamic => "dynamic",
        Category::Guardrail => "guardrail",
        Category::Warning => "warning",
    }
}

fn category_title(c: Category) -> &'static str {
    match c {
        Category::Static => "Static errors (S)",
        Category::Type => "Type errors (T)",
        Category::Dynamic => "Dynamic errors (D)",
        Category::Guardrail => "Guardrail errors (U)",
        Category::Warning => "Warnings (W)",
    }
}

pub fn run(args: ErrorsArgs) -> Result<ExitCode> {
    let catalog = catalog();
    if let Some(code) = &args.code {
        let code = code.to_ascii_uppercase();
        let Some(e) = catalog.iter().find(|e| e.code == code) else {
            bail!("there is no error code {code}; run `jlens errors` for the list");
        };
        if args.json {
            print_json(&Entry::from(e))?;
        } else {
            output::line(e.code)?;
            output::line(&format!("  category: {}", category_title(e.category())))?;
            output::line(&format!("  message:  {}", e.template))?;
        }
        return Ok(ExitCode::SUCCESS);
    }
    if args.json {
        print_json(&catalog.iter().map(Entry::from).collect::<Vec<_>>())?;
        return Ok(ExitCode::SUCCESS);
    }
    let mut categories: Vec<Category> = Vec::new();
    for e in catalog {
        if !categories.contains(&e.category()) {
            categories.push(e.category());
        }
    }
    for (i, category) in categories.into_iter().enumerate() {
        if i > 0 {
            output::line("")?;
        }
        output::line(&format!("{}:", category_title(category)))?;
        for e in catalog.iter().filter(|e| e.category() == category) {
            output::line(&format!("  {}  {}", e.code, e.template))?;
        }
    }
    Ok(ExitCode::SUCCESS)
}
