//! `cargo xtask mdbook-dedent`: an mdBook preprocessor, declared in `docs/book.toml`, that
//! removes the indentation every line of a code block shares.
//!
//! `{{#include file:anchor}}` keeps the lines of the anchored part as they are in the file
//! (mdBook issues 1430 and 1601), so a snippet taken from inside a function shows its
//! indentation: four spaces in the Rust guide, eight in the Java one. Running after `links`, the
//! preprocessor sees the included text and shifts each fenced block left, relative to its fence.

use std::io::{self, Read, Write};

use anyhow::{Context, Result, bail};
use serde_json::Value;

/// Runs as mdBook calls a preprocessor: `supports <renderer>` (every renderer is), or the book
/// as JSON on standard input, `[context, book]`, written back with its chapters dedented.
pub fn run(args: &[String]) -> Result<()> {
    if args.first().map(String::as_str) == Some("supports") {
        return Ok(());
    }
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .context("reading the book from mdBook")?;
    let Value::Array(mut parts) = serde_json::from_str(&input).context("parsing the book")? else {
        bail!("mdBook did not send `[context, book]`");
    };
    if parts.len() != 2 {
        bail!("mdBook did not send `[context, book]`");
    }
    let mut book = parts.pop().unwrap();
    chapters(&mut book);
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, &book)?;
    out.flush()?;
    Ok(())
}

/// Dedents the content of every chapter, at any depth: a chapter is an object with a `content`
/// string and its `sub_items`.
fn chapters(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.contains_key("sub_items")
                && let Some(Value::String(content)) = map.get_mut("content")
            {
                *content = dedent(content);
            }
            map.values_mut().for_each(chapters);
        }
        Value::Array(items) => items.iter_mut().for_each(chapters),
        _ => {}
    }
}

/// The Markdown with each fenced code block shifted left by the spaces that all its non-empty
/// lines have past the fence's own indentation. A block whose lines indent with tabs is left
/// as it is.
pub fn dedent(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut block: Option<(usize, char, usize, Vec<&str>)> = None;
    for line in markdown.split_inclusive('\n') {
        let indent = line.len() - line.trim_start_matches(' ').len();
        let rest = &line[indent..];
        let fence = rest.chars().next().filter(|c| *c == '`' || *c == '~');
        let run = fence.map_or(0, |c| rest.len() - rest.trim_start_matches(c).len());
        match &mut block {
            None => {
                out.push_str(line);
                if let Some(c) = fence
                    && run >= 3
                {
                    block = Some((indent, c, run, Vec::new()));
                }
            }
            Some((fence_indent, c, length, lines)) => {
                let closes = fence == Some(*c)
                    && run >= *length
                    && rest.trim_start_matches(*c).trim().is_empty();
                if closes {
                    shift(&mut out, *fence_indent, lines);
                    out.push_str(line);
                    block = None;
                } else {
                    lines.push(line);
                }
            }
        }
    }
    // A block left open runs to the end of the chapter, as CommonMark reads it.
    if let Some((fence_indent, _, _, lines)) = block {
        shift(&mut out, fence_indent, &lines);
    }
    out
}

/// Writes the lines of one block, without the spaces they all share past `fence_indent`.
fn shift(out: &mut String, fence_indent: usize, lines: &[&str]) {
    let blank = |line: &str| line.trim().is_empty();
    let common = lines
        .iter()
        .filter(|line| !blank(line))
        .map(|line| {
            let spaces = line.len() - line.trim_start_matches(' ').len();
            if line[spaces..].starts_with('\t') {
                0
            } else {
                spaces.saturating_sub(fence_indent)
            }
        })
        .min()
        .unwrap_or(0);
    for line in lines {
        if common == 0 || blank(line) {
            out.push_str(line);
        } else {
            out.push_str(&line[..fence_indent]);
            out.push_str(&line[fence_indent + common..]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::dedent;

    #[test]
    fn shifts_a_block_to_its_fence() {
        let md = "text\n\n```java\n        a();\n\n            b();\n```\n    indented text\n";
        assert_eq!(
            dedent(md),
            "text\n\n```java\na();\n\n    b();\n```\n    indented text\n"
        );
    }

    #[test]
    fn keeps_the_indentation_of_a_fence_in_a_list() {
        let md = "- item\n\n  ```sh\n      run\n  ```\n";
        assert_eq!(dedent(md), "- item\n\n  ```sh\n  run\n  ```\n");
    }

    #[test]
    fn leaves_blocks_without_shared_indentation_and_tabs_alone() {
        for md in [
            "```\na\n  b\n```\n",
            "```\n\ta\n\tb\n```\n",
            "  no fence\n",
            "``\n  x\n``\n",
        ] {
            assert_eq!(dedent(md), md);
        }
    }

    #[test]
    fn a_shorter_or_other_fence_does_not_close_a_block() {
        let md = "````\n  ```\n  ~~~\n  x\n````\n";
        assert_eq!(dedent(md), "````\n```\n~~~\nx\n````\n");
    }
}
