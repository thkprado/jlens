//! `jlens functions`: the built-in function catalog.

use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Args;
use jlens_core::{Category, FunctionInfo, function_catalog};

use crate::{output, print_json};

#[derive(Args)]
pub struct FunctionsArgs {
    /// Describe one function (with or without the leading `$`).
    name: Option<String>,
    /// Print as JSON.
    #[arg(long)]
    json: bool,
}

fn category_title(c: Category) -> &'static str {
    match c {
        Category::String => "String",
        Category::Numeric => "Numeric",
        Category::Aggregation => "Aggregation",
        Category::Boolean => "Boolean",
        Category::Array => "Array",
        Category::Object => "Object",
        Category::HigherOrder => "Higher-order",
        Category::DateTime => "Date/time",
        Category::Encoding => "Encoding",
        Category::Misc => "Other",
    }
}

fn call(f: &FunctionInfo) -> String {
    let params: Vec<String> = f.params.iter().map(|p| format!("${p}")).collect();
    format!("${}({})", f.name, params.join(", "))
}

pub fn run(args: FunctionsArgs) -> Result<ExitCode> {
    let catalog = function_catalog();
    if let Some(name) = &args.name {
        let name = name.trim_start_matches('$');
        let Some(f) = catalog.iter().find(|f| f.name == name) else {
            bail!("there is no built-in function ${name}; run `jlens functions` for the list");
        };
        if args.json {
            print_json(f)?;
        } else {
            output::line(&call(f))?;
            output::line(&format!("  signature: {}", f.signature))?;
            output::line(&format!("  category:  {}", category_title(f.category)))?;
            output::line(&format!("  {}", f.summary))?;
        }
        return Ok(ExitCode::SUCCESS);
    }
    if args.json {
        print_json(&catalog)?;
        return Ok(ExitCode::SUCCESS);
    }
    let mut categories: Vec<Category> = Vec::new();
    for f in &catalog {
        if !categories.contains(&f.category) {
            categories.push(f.category);
        }
    }
    let width = catalog
        .iter()
        .map(|f| call(f).chars().count())
        .max()
        .unwrap_or(0);
    for (i, category) in categories.into_iter().enumerate() {
        if i > 0 {
            output::line("")?;
        }
        output::line(&format!("{}:", category_title(category)))?;
        for f in catalog.iter().filter(|f| f.category == category) {
            output::line(&format!("  {:width$}  {}", call(f), f.summary))?;
        }
    }
    Ok(ExitCode::SUCCESS)
}
